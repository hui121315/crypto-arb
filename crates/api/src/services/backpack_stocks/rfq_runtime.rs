use super::*;
use exchange::ws::{WsConfig, WsEvent, WsManager};
use std::{collections::BTreeMap, sync::Weak, time::Instant};
use tokio::{sync::broadcast, task::JoinSet};

struct Runner(JoinHandle<()>);
impl Drop for Runner {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn run(service: Weak<BackpackStocks>, hub: realtime::WsHub, url: String) {
    let manager = Arc::new(WsManager::new_with_event_capacity(
        WsConfig {
            url,
            exchange: "backpack:stock-rfq".into(),
            ..Default::default()
        },
        128,
    ));
    manager.suspend().await;
    let mut events = manager.subscribe();
    let runner = manager.clone();
    let _runner = Runner(tokio::spawn(async move {
        let _ = runner.run().await;
    }));
    let mut keys: Option<credentials::Credentials> = None;
    let mut key_check = Instant::now();
    let mut retry_at = Instant::now();
    let mut rechecks = JoinSet::new();
    let mut order_rechecks = JoinSet::new();
    let mut checked = BTreeMap::<String, Instant>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=tick.tick()=>{
                let Some(s)=service.upgrade() else{break};
                let pending:Vec<_>=s.rfq_records().into_iter().filter(|r|r.needs_follow_up()).collect();
                let orders:Vec<_>=s.plan_store.records().into_iter().filter(|p|p.cex_order.as_ref().is_some_and(StockCexOrder::needs_follow_up)).collect();
                let conversions:Vec<_>=s.exchange_conversion_store.rows().into_iter().filter(|p|p.order.as_ref().is_some_and(StockCexOrder::needs_follow_up)).collect();
                checked.retain(|id,_|pending.iter().any(|r|&r.request.request_id==id));
                let tracking=s.account_tracking_until_ms.load(Ordering::SeqCst)>common::time::now_ms();
                let order_tracking=s.order_tracking_until_ms.load(Ordering::SeqCst)>common::time::now_ms();
                if pending.is_empty() && orders.is_empty() && conversions.is_empty() && !tracking && !order_tracking {
                    let active=manager.is_active();
                    let connected=s.rfq_subscription.send_replace(None).is_some();
                    let had_problem=s.rfq_problem.write().take().is_some();
                    if active {manager.suspend().await;s.invalidate_account();}
                    if active || connected || had_problem {s.publish_rfq(&hub);}
                    keys=None;continue;
                }
                if Instant::now()<retry_at {continue;}
                if keys.is_none() || Instant::now()>=key_check {
                    key_check=Instant::now()+Duration::from_secs(30);
                    match (s.credential_loader)() {
                        Ok(next)=>{
                            if keys.as_ref().is_some_and(|old|old.fingerprint()!=next.fingerprint()) {
                                manager.suspend().await;s.rfq_store.disconnect();s.rfq_subscription.send_replace(None);s.invalidate_account();
                                rechecks.abort_all();checked.clear();
                                order_rechecks.abort_all();
                            }
                            keys=Some(next);
                        },
                        Err(problem)=>{
                            manager.suspend().await;s.rfq_store.disconnect();s.rfq_subscription.send_replace(None);s.invalidate_account();
                            *s.rfq_problem.write()=Some(problem);s.publish_rfq(&hub);keys=None;
                            retry_at=Instant::now()+Duration::from_secs(30);continue;
                        }
                    }
                }
                let fingerprint=keys.as_ref().expect("loaded credentials").fingerprint();
                if !tracking && !order_tracking && !pending.iter().any(|r|r.account_fingerprint==fingerprint) && !orders.iter().any(|p|p.terms.account_fingerprint==fingerprint) && !conversions.iter().any(|p|p.terms.account_fingerprint==fingerprint) {
                    manager.suspend().await;s.rfq_store.disconnect();s.rfq_subscription.send_replace(None);s.invalidate_account();
                    *s.rfq_problem.write()=Some("未结股票订单/RFQ 与当前 API 账户不同；请恢复原凭证后核对".into());
                    s.publish_rfq(&hub);retry_at=Instant::now()+Duration::from_secs(30);continue;
                }
                manager.activate();
                if tracking && s.rfq_subscription.borrow().as_deref()==Some(&fingerprint) && s.account_subscription.borrow().as_deref()!=Some(&fingerprint) {
                    let frame=keys.as_ref().expect("loaded credentials").subscribe_stream("account.balanceUpdate",common::time::now_ms());
                    let sent=match frame {Ok(frame)=>manager.send_text(frame).await.is_ok(),Err(_)=>false};
                    if sent {s.account_subscription.send_replace(Some(fingerprint.clone()));}
                }
                if (order_tracking || !orders.is_empty() || !conversions.is_empty()) && s.rfq_subscription.borrow().as_deref()==Some(&fingerprint) && s.order_subscription.borrow().as_deref()!=Some(&fingerprint) {
                    let frame=keys.as_ref().expect("loaded credentials").subscribe_stream("account.orderUpdate",common::time::now_ms());
                    let sent=match frame {Ok(frame)=>manager.send_text(frame).await.is_ok(),Err(_)=>false};
                    if sent {s.order_subscription.send_replace(Some(fingerprint.clone()));}
                }
                if order_rechecks.is_empty() && s.plan_store.problem().is_none() {
                    let now=common::time::now_ms();
                    if let Some(plan)=orders.iter().find(|p|p.terms.account_fingerprint==fingerprint && p.cex_order.as_ref().is_some_and(|o|o.recheck.next_at_ms.is_none_or(|t|t<=now))) {
                        let id=plan.plan_id.clone();let weak=service.clone();let hub=hub.clone();
                        order_rechecks.spawn(async move {
                            let Some(s)=weak.upgrade() else{return};
                            let Ok(_guard)=s.order_lock.try_lock() else{return};
                            if let Ok(keys)=(s.credential_loader)() {
                                let result=tokio::time::timeout(Duration::from_secs(25),s.reconcile_stock_order(&id,&keys,true)).await
                                    .map_err(|_|"股票订单核对超时，保留原订单与占用".to_owned()).and_then(|r|r);
                                if let Err(problem)=result {let _=s.stock_order_problem(&id,problem);}
                                s.publish_rfq(&hub);
                            }
                        });
                    }
                }
                // REST reconciliation is bounded and runs off the WS reader. It never resubmits.
                if order_rechecks.is_empty() && s.exchange_conversion_store.problem().is_none() {
                    let now=common::time::now_ms();
                    if let Some(p)=conversions.iter().find(|p|p.terms.account_fingerprint==fingerprint &&p.order.as_ref().is_some_and(|o|o.recheck.next_at_ms.is_none_or(|t|t<=now))) {
                        let id=p.plan_id.clone();let weak=service.clone();let hub=hub.clone();
                        order_rechecks.spawn(async move {
                            let Some(s)=weak.upgrade() else{return};let Ok(_guard)=s.order_lock.try_lock() else{return};
                            let result=tokio::time::timeout(Duration::from_secs(25),s.reconcile_conversion(&id,true)).await
                                .map_err(|_|"原账户兑换核对超时，保留占用".to_owned()).and_then(|r|r);
                            if let Err(e)=result {let _=s.conversion_problem(&id,e);}s.publish_rfq(&hub);
                        });
                    }
                }
                if rechecks.is_empty() {
                    let now=common::time::now_ms();
                    let next=pending.iter().filter(|r|r.account_fingerprint==fingerprint
                        && (r.needs_recheck || r.settlement_pending() || r.phase==StockRfqPhase::AcceptedBinding || r.expiry_time_ms.is_some_and(|t|t<now))
                        && if r.settlement_pending() || r.acceptance.is_some() {
                            r.settlement.next_at_ms.is_none_or(|t| t<=now)
                        } else {
                            checked.get(&r.request.request_id).is_none_or(|t|t.elapsed()>=Duration::from_secs(30))
                        }).min_by_key(|r|checked.get(&r.request.request_id).copied());
                    if let Some(record)=next {
                        let id=record.request.request_id.clone();checked.insert(id.clone(),Instant::now());
                        let weak=service.clone();let hub=hub.clone();
                        rechecks.spawn(async move {
                            let Some(s)=weak.upgrade() else{return};
                            let Ok(_guard)=s.rfq_lock.try_lock() else{return};
                            if let Ok(keys)=(s.credential_loader)() {
                                if let Err(problem)=s.reconcile_rfq_with_mode(&id,&keys,true).await {let _=s.rfq_problem_record(&id,problem);}
                                s.publish_rfq(&hub);
                            }
                        });
                    }
                }
            },
            _=rechecks.join_next(),if !rechecks.is_empty()=>{},
            _=order_rechecks.join_next(),if !order_rechecks.is_empty()=>{},
            event=events.recv()=>{
                let Some(s)=service.upgrade() else{break};
                match event {
                    Ok(WsEvent::Connected)=>{
                        s.rfq_store.disconnect();s.invalidate_account();checked.clear();
                        let subscribe=keys.as_ref().ok_or_else(||"Backpack 凭证未载入".to_owned()).and_then(|k|k.subscribe(common::time::now_ms()));
                        let sent=match subscribe {Ok(frame)=>manager.send_text(frame).await.is_ok(),Err(_)=>false};
                        s.rfq_subscription.send_replace(if sent {keys.as_ref().map(|k|k.fingerprint())}else{None});
                        *s.rfq_problem.write()=Some(if sent {"私有 RFQ 订阅已发送，等待官方账户事件"}else{"私有 RFQ 订阅失败"}.into());
                        if !sent {manager.suspend().await;retry_at=Instant::now()+Duration::from_secs(30);}
                    },
                    Ok(WsEvent::Disconnected(_))|Ok(WsEvent::CircuitOpened)|Err(broadcast::error::RecvError::Lagged(_))=>{
                        manager.suspend().await;
                        s.rfq_subscription.send_replace(None);s.rfq_store.disconnect();s.invalidate_account();checked.clear();
                        *s.rfq_problem.write()=Some("私有 RFQ 连接中断，旧报价已失效；重连后核对原请求".into());
                    },
                    Ok(WsEvent::Text(text)) if manager.is_active()=>{
                        let fingerprint=keys.as_ref().map(|k|k.fingerprint()).unwrap_or_default();
                        match account::apply_frame(&s,&text,&fingerprint,common::time::now_ms()) {
                            Ok(true)=>{s.publish_rfq(&hub);continue;},
                            Err(_)=>{s.invalidate_account();},
                            Ok(false)=>{},
                        }
                        match orders::apply_frame(&s,&text,&fingerprint,common::time::now_ms()) {
                            Ok(true)=>{s.publish_rfq(&hub);continue;},
                            Ok(false)=>{},
                            Err(problem)=>{*s.rfq_problem.write()=Some(problem);s.publish_rfq(&hub);continue;},
                        }
                        match s.apply_conversion_frame(&text,&fingerprint,common::time::now_ms()) {
                            Ok(true)=>{s.publish_rfq(&hub);continue;},Ok(false)=>{},
                            Err(problem)=>{*s.rfq_problem.write()=Some(problem);s.publish_rfq(&hub);continue;},
                        }
                        match apply_frame(&s,&text,&fingerprint,common::time::now_ms()) {
                            Ok(true)=>{*s.rfq_problem.write()=None;},
                            Ok(false)=>continue,
                            Err(problem)=>{
                                s.rfq_store.disconnect();s.rfq_subscription.send_replace(None);s.invalidate_account();
                                *s.rfq_problem.write()=Some(problem);manager.suspend().await;
                                retry_at=Instant::now()+Duration::from_secs(30);
                            }
                        }
                    },
                    Err(broadcast::error::RecvError::Closed)=>break,
                    _=>continue,
                }
                s.publish_rfq(&hub);
            }
        }
    }
}

pub(super) fn apply_frame(
    s: &BackpackStocks,
    text: &str,
    fingerprint: &str,
    now: i64,
) -> Result<bool, String> {
    let Some((remote, frame)) = rfq_protocol::frame_id(text)? else {
        return Ok(false);
    };
    let client = frame["C"]
        .as_str()
        .and_then(|s| s.parse::<u32>().ok())
        .or_else(|| frame["C"].as_u64().and_then(|v| u32::try_from(v).ok()));
    let owned = s.rfq_records().into_iter().find(|r| {
        r.account_fingerprint == fingerprint
            && (r.rfq_id.as_deref() == Some(&remote)
                || (r.rfq_id.is_none() && Some(r.client_id) == client))
    });
    let Some(record) = owned else {
        return Ok(false);
    };
    let durable = frame["e"] != "rfqCandidate" || record.rfq_id.is_none();
    Ok(
        s.record_rfq_receipt(&record.request.request_id, durable, |row| {
            let changed = rfq_protocol::apply_event(row, &frame, now)?;
            if changed {
                row.rfq_id = Some(remote);
            }
            Ok(changed)
        })?
        .is_some(),
    )
}
