use super::*;
use exchange::ws::{WsConfig, WsEvent, WsManager};
use std::{collections::BTreeSet, sync::Weak};
use tokio::sync::broadcast;

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
            exchange: "backpack:stocks".into(),
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
    let mut active_streams = BTreeSet::new();
    let mut published = None;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let Some(s) = service.upgrade() else { break };
                if hub.subscriber_count(realtime::channels::STOCKS)>0
                    || (s.background_monitoring() && s.snapshot.read().monitor.alerts.include_peer) {
                    s.poll_peer_ws().await;
                    s.refresh_peer(common::time::now_ms());
                }
                let mut desired = s.snapshot.read().security.as_ref().map(protocol::streams).unwrap_or_default();
                if hub.subscriber_count(realtime::channels::STOCKS)>0 {
                    desired.extend(s.batch_streams());
                } else {
                    s.batch_connection(false);
                }
                if !desired.is_empty() { desired.insert("bookTicker.USDT_USDC".into()); }
                if (hub.subscriber_count(realtime::channels::STOCKS) == 0 && !s.background_monitoring()) || desired.is_empty() {
                    manager.suspend().await;
                    active_streams.clear();
                    s.snapshot.write().connected = false;
                    s.batch_connection(false);
                } else {
                    manager.activate();
                    if manager.is_connected().await && desired != active_streams {
                        let unsubscribe_ok = active_streams.is_empty() || manager.send_text(
                            serde_json::json!({"method":"UNSUBSCRIBE","params":active_streams}).to_string()
                        ).await.is_ok();
                        if unsubscribe_ok && manager.send_text(serde_json::json!({"method":"SUBSCRIBE","params":desired}).to_string()).await.is_ok() {
                            active_streams = desired;
                            s.snapshot.write().connected = true;
                            s.batch_connection(true);
                        } else {
                            manager.suspend().await;
                            active_streams.clear();
                            let mut snapshot = s.snapshot.write();
                            snapshot.connected = false;
                            snapshot.problem = Some("Backpack 订阅切换失败，旧连接已关闭，等待重连".into());
                            drop(snapshot);
                            s.batch_connection(false);
                        }
                    }
                }
            }
            event = events.recv() => {
                let Some(s) = service.upgrade() else { break };
                match event {
                    Ok(WsEvent::Connected) => {
                        active_streams.clear();
                        s.batch_connection(false);
                        let mut snapshot = s.snapshot.write();
                        snapshot.connected = true;
                        snapshot.books.clear();
                        snapshot.conversion_book = None;
                        snapshot.conversion_book_problem = None;
                        snapshot.reference = None;
                        snapshot.reference_problem = None;
                        snapshot.problem = None;
                    }
                    Ok(WsEvent::Disconnected(_)) | Ok(WsEvent::CircuitOpened) => {
                        active_streams.clear();
                        s.batch_connection(false);
                        let mut snapshot = s.snapshot.write();
                        snapshot.connected = false;
                        snapshot.problem = Some("Backpack 行情连接中断，旧报价仅供观察".into());
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // A lost frame makes the local sequence uncertain; replace the session.
                        manager.suspend().await;
                        active_streams.clear();
                        s.batch_connection(false);
                        let mut snapshot = s.snapshot.write();
                        snapshot.connected = false;
                        snapshot.books.clear();
                        snapshot.conversion_book = None;
                        snapshot.reference = None;
                        snapshot.problem = Some("股票行情接收积压，正在重新订阅".into());
                    }
                    Ok(WsEvent::Text(text)) => {
                        if !manager.is_active() { continue; }
                        s.batch_frame(&text, common::time::now_ms());
                        let mut snapshot = s.snapshot.write();
                        // Each feed clears only its own error; a reference tick cannot repair a bad book.
                        let _ = protocol::apply(&mut snapshot, &text, common::time::now_ms());
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
            }
        }
        let Some(s) = service.upgrade() else { break };
        let mut snapshot = s.snapshot.write();
        if published.as_ref() != Some(&*snapshot) {
            // HTTP selection/quote replies and WS status changes need one monotonic ordering.
            // Native price timestamps remain unchanged for freshness checks.
            snapshot.observed_at_ms =
                common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
            let current = snapshot.clone();
            drop(snapshot);
            s.publish(&hub);
            published = Some(current);
        }
    }
}
