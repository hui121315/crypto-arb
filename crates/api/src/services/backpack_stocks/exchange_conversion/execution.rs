use super::*;

impl BackpackStocks {
    pub(crate) async fn submit_exchange_conversion(
        self: &Arc<Self>,
        r: StockStablecoinSubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.submit_conversion_with(r, hub, move || {
            submission::check_live(&trading.risk_config())
        })
        .await
    }
    pub(super) async fn submit_conversion_with<F>(
        self: &Arc<Self>,
        r: StockStablecoinSubmitRequest,
        hub: realtime::WsHub,
        live: F,
    ) -> Result<StockMarketSnapshot, String>
    where
        F: Fn() -> Result<(), String> + Send + Sync + 'static,
    {
        if !r.confirm_live {
            return Err("请确认本次 Backpack 账户兑换".into());
        }
        let p = self.exchange_conversion_store.get(&r.plan_id)?;
        if p.order.is_some() {
            return Ok(self.snapshot());
        }
        if p.revision != r.revision || !p.can_submit(common::time::now_ms()) {
            return Err("账户兑换计划已变化或过期，没有提交".into());
        }
        live()?;
        let guard = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "已有股票资金请求正在提交，请等待原结果")?;
        self.order_tracking_until_ms.store(
            common::time::now_ms().saturating_add(30_000),
            Ordering::SeqCst,
        );
        self.ensure_rfq_started(hub.clone());
        let service = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        // One bounded owner survives a disconnected HTTP client. Durable intent precedes POST.
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(25),
                service.send_conversion(&r, &hub, &live),
            )
            .await
            .map_err(|_| "兑换提交等待超时，请查询原订单；不会重发或释放占用".to_owned())
            .and_then(|r| r);
            service.publish_rfq(&hub);
            drop(guard);
            let _ = tx.send(result.map(|_| service.snapshot()));
        });
        rx.await
            .map_err(|_| "兑换提交任务中断，请查询原订单".to_owned())?
    }
    async fn send_conversion(
        &self,
        r: &StockStablecoinSubmitRequest,
        hub: &realtime::WsHub,
        live: &(impl Fn() -> Result<(), String> + Sync),
    ) -> Result<(), String> {
        let _guard = self
            .order_lock
            .try_lock()
            .map_err(|_| "原账户订单正在处理")?;
        let plan = self.exchange_conversion_store.get(&r.plan_id)?;
        if plan.order.is_some() {
            return Ok(());
        }
        let keys = (self.credential_loader)()?;
        let fp = keys.fingerprint();
        if fp != plan.terms.account_fingerprint {
            return Err("当前凭证不是兑换计划原账户".into());
        }
        let mut subscription = self.order_subscription.subscribe();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if subscription.borrow_and_update().as_deref() == Some(&fp) {
                    return Ok::<_, String>(());
                }
                subscription
                    .changed()
                    .await
                    .map_err(|_| "账户私有订单连接已停止")?;
            }
        })
        .await
        .map_err(|_| "账户私有订单 WS 尚未就绪，没有提交")??;
        self.invalidate_funding_inventory(&fp);
        let account = self.read_account(&keys).await?;
        let now = common::time::now_ms();
        let book = self.conversion_book(now)?;
        let fresh = compile(
            &plan.request,
            plan.terms.market.clone(),
            book,
            &account,
            now,
        )?;
        let old_price =
            order_protocol::decimal(plan.terms.book.bid.as_deref().ok_or("原兑换买价缺失")?)?;
        let new_price =
            order_protocol::decimal(fresh.book.bid.as_deref().ok_or("当前兑换买价缺失")?)?;
        if new_price < old_price
            || order_protocol::decimal(&fresh.taker_fee_bps)?
                > order_protocol::decimal(&plan.terms.taker_fee_bps)?
        {
            return Err("兑换买价变差或手续费上升，未改变原限价，未提交".into());
        }
        live()?;
        if (self.credential_loader)()?.fingerprint() != fp {
            return Err("提交前账户凭证变化，未兑换".into());
        }
        let mut send = false;
        let p = self
            .exchange_conversion_store
            .change(&r.plan_id, common::time::now_ms(), |p| {
                if p.order.is_some() {
                    return Ok(false);
                }
                if p.revision != r.revision || !p.can_submit(common::time::now_ms()) {
                    return Err("兑换原版本过期或变化，没有提交".into());
                }
                p.order = Some(StockCexOrder::intent(common::time::now_ms()));
                send = true;
                Ok(true)
            })?;
        if !send {
            return Ok(());
        }
        self.publish_rfq(hub);
        let i = &p.terms.instruction;
        match self
            .signed_rfq_request(
                &keys,
                reqwest::Method::POST,
                i.path(),
                i.signing_instruction(),
                &i.request_body(),
            )
            .await
        {
            Ok(bytes) => {
                let applied = serde_json::from_slice::<Value>(&bytes)
                    .map_err(|_| "账户兑换下单回复不完整，只核对原订单".to_owned())
                    .and_then(|v| {
                        self.conversion_receipt(&p.plan_id, |o, i| {
                            order_protocol::apply_order(o, i, &v, false, common::time::now_ms())
                        })
                    });
                if let Err(problem) = applied {
                    self.conversion_problem(&p.plan_id, problem)?;
                }
            }
            Err(error) => {
                self.exchange_conversion_store
                    .change(&p.plan_id, common::time::now_ms(), |p| {
                        let o = p.order.as_mut().ok_or("原兑换意图缺失")?;
                        if error.rejected
                            && o.order_id.is_none()
                            && o.fills.is_empty()
                            && o.phase == StockCexOrderPhase::SubmissionUnknown
                        {
                            o.phase = StockCexOrderPhase::Rejected;
                            o.executed_quantity = Some("0".into());
                            o.executed_quote_quantity = Some("0".into());
                            o.recheck.next_at_ms = None;
                        }
                        o.problem = Some(error.problem);
                        o.updated_at_ms = common::time::now_ms().max(o.updated_at_ms);
                        Ok(true)
                    })?;
            }
        }
        self.invalidate_funding_inventory(&fp);
        Ok(())
    }
    pub(crate) async fn recheck_exchange_conversion(
        self: &Arc<Self>,
        id: &str,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self.order_lock.try_lock().map_err(|_| "账户订单正在核对")?;
        let p = self.exchange_conversion_store.get(id)?;
        if p.order
            .as_ref()
            .is_some_and(StockCexOrder::receipt_complete)
        {
            return Ok(self.snapshot());
        }
        let result = self.reconcile_conversion(id, false).await;
        if let Err(problem) = &result {
            self.conversion_problem(id, problem.clone())?;
        }
        self.publish_rfq(hub);
        result?;
        Ok(self.snapshot())
    }
    pub(in crate::services::backpack_stocks) async fn reconcile_conversion(
        &self,
        id: &str,
        automatic: bool,
    ) -> Result<(), String> {
        let keys = (self.credential_loader)()?;
        let p = self.exchange_conversion_store.get(id)?;
        if p.terms.account_fingerprint != keys.fingerprint() {
            return Err("请恢复兑换计划原账户后再核对".into());
        }
        let o = p.order.as_ref().ok_or("兑换尚未提交，没有远端订单")?;
        if o.receipt_complete() {
            return Ok(());
        }
        let now = common::time::now_ms();
        if o.recheck.next_at_ms.is_some_and(|t| t > now) || automatic && o.recheck.paused {
            return Ok(());
        }
        self.exchange_conversion_store.change(id, now, |p| {
            let o = p.order.as_mut().ok_or("原兑换订单缺失")?;
            if automatic {
                o.recheck.attempts = o.recheck.attempts.saturating_add(1).min(6);
                o.recheck.paused = o.recheck.attempts >= 6;
            }
            o.recheck.next_at_ms = Some(now.saturating_add(if automatic { 30_000 } else { 5_000 }));
            o.updated_at_ms = now.max(o.updated_at_ms);
            Ok(true)
        })?;
        let client = store::client_id(&p)?;
        let symbol = STOCK_CONVERSION_SYMBOL;
        if !o.phase.terminal() {
            let params = if let Some(id) = &o.order_id {
                json!({"symbol":symbol,"orderId":id})
            } else {
                json!({"symbol":symbol,"clientId":client})
            };
            match self
                .signed_rfq_request(
                    &keys,
                    reqwest::Method::GET,
                    "/api/v1/order",
                    "orderQuery",
                    &params,
                )
                .await
            {
                Ok(bytes) => {
                    let v: Value =
                        serde_json::from_slice(&bytes).map_err(|_| "兑换订单查询回复不完整")?;
                    self.conversion_receipt(id, |o, i| {
                        order_protocol::apply_order(o, i, &v, false, common::time::now_ms())
                    })?;
                }
                Err(e) if e.status == Some(404) => {}
                Err(e) => return Err(e.problem),
            }
        }
        let p = self.exchange_conversion_store.get(id)?;
        let o = p.order.as_ref().ok_or("原兑换订单缺失")?;
        if !o.phase.terminal() || o.order_id.is_none() {
            let mut found = None;
            for page in 0..3 {
                let mut query =
                    json!({"symbol":symbol,"limit":100,"offset":page*100,"sortDirection":"Desc"});
                if let Some(remote) = &o.order_id {
                    query["orderId"] = json!(remote);
                }
                let bytes = self
                    .signed_rfq_request(
                        &keys,
                        reqwest::Method::GET,
                        "/wapi/v1/history/orders",
                        "orderHistoryQueryAll",
                        &query,
                    )
                    .await
                    .map_err(String::from)?;
                let rows: Vec<Value> =
                    serde_json::from_slice(&bytes).map_err(|_| "兑换订单历史回复不完整")?;
                for row in &rows {
                    if order_protocol::client(&row["clientId"]) == Some(client) {
                        if found.is_some() {
                            return Err("兑换 clientId 对应多条订单，保留占用并核查".into());
                        }
                        found = Some(row.clone());
                    }
                }
                if rows.len() < 100 || o.order_id.is_some() {
                    break;
                }
            }
            if let Some(v) = found {
                self.conversion_receipt(id, |o, i| {
                    order_protocol::apply_order(o, i, &v, false, common::time::now_ms())
                })?;
            }
        }
        let p = self.exchange_conversion_store.get(id)?;
        let remote = p
            .order
            .as_ref()
            .and_then(|o| o.order_id.as_ref())
            .ok_or("暂未找到原兑换订单，保留占用且不重发")?;
        let mut fills = vec![];
        for page in 0..6 {
            let bytes=self.signed_rfq_request(&keys,reqwest::Method::GET,"/wapi/v1/history/fills","fillHistoryQueryAll",&json!({"symbol":symbol,"orderId":remote,"limit":100,"offset":page*100,"sortDirection":"Asc"})).await.map_err(String::from)?;
            let rows: Vec<Value> =
                serde_json::from_slice(&bytes).map_err(|_| "兑换成交历史不完整")?;
            let full = rows.len() >= 100;
            fills.extend(rows);
            if fills.len() > 512 || full && page == 5 {
                return Err("兑换成交明细超过核对预算，保留占用".into());
            }
            if !full {
                break;
            }
        }
        self.conversion_receipt(id, |o, i| {
            order_protocol::apply_fills(o, i, &fills, common::time::now_ms())
        })?;
        self.invalidate_funding_inventory(&keys.fingerprint());
        Ok(())
    }
    pub(in crate::services::backpack_stocks) fn conversion_problem(
        &self,
        id: &str,
        problem: String,
    ) -> Result<(), String> {
        self.exchange_conversion_store
            .change(id, common::time::now_ms(), |p| {
                let Some(o) = p.order.as_mut() else {
                    return Ok(false);
                };
                if o.problem.as_ref() == Some(&problem) {
                    return Ok(false);
                }
                o.problem = Some(problem);
                o.updated_at_ms = common::time::now_ms().max(o.updated_at_ms);
                Ok(true)
            })?;
        Ok(())
    }
    fn conversion_receipt(
        &self,
        id: &str,
        apply: impl FnOnce(&mut StockCexOrder, &StockCexInstruction) -> Result<bool, String>,
    ) -> Result<(), String> {
        let now = common::time::now_ms();
        let result = self.exchange_conversion_store.change(id, now, |p| {
            apply(
                p.order.as_mut().ok_or("兑换提交意图缺失")?,
                &p.terms.instruction,
            )
        });
        if let Err(problem) = &result {
            self.exchange_conversion_store.change(id, now, |p| {
                let o = p.order.as_mut().ok_or("兑换提交意图缺失")?;
                if o.order_id.is_some() || !o.fills.is_empty() {
                    o.evidence_conflict = true;
                    o.recheck.paused = true;
                }
                if o.problem.as_ref() == Some(problem) {
                    return Ok(false);
                }
                o.problem = Some(problem.clone());
                o.updated_at_ms = now.max(o.updated_at_ms);
                Ok(true)
            })?;
        }
        result.map(|_| ())
    }
    pub(in crate::services::backpack_stocks) fn apply_conversion_frame(
        &self,
        text: &str,
        fp: &str,
        now: i64,
    ) -> Result<bool, String> {
        let v: Value = serde_json::from_str(text).map_err(|_| "兑换 WS 回复无效")?;
        let v = v.get("data").unwrap_or(&v);
        if !matches!(
            v["e"].as_str(),
            Some(
                "orderAccepted" | "orderFill" | "orderCancelled" | "orderExpired" | "orderModified"
            )
        ) || v["s"] != STOCK_CONVERSION_SYMBOL
        {
            return Ok(false);
        }
        let Some(p) = self.exchange_conversion_store.rows().into_iter().find(|p| {
            p.terms.account_fingerprint == fp
                && p.order.is_some()
                && store::client_id(p).ok() == order_protocol::client(&v["c"])
        }) else {
            return Ok(false);
        };
        self.conversion_receipt(&p.plan_id, |o, i| {
            order_protocol::apply_order(o, i, v, true, now)
        })?;
        let changed = self.exchange_conversion_store.get(&p.plan_id)?.revision != p.revision;
        if changed {
            self.invalidate_funding_inventory(fp);
        }
        Ok(changed)
    }
}
