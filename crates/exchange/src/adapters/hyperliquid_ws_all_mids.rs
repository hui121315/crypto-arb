//! Hyperliquid full-market mids stream.
//!
//! Official `allMids` includes core spot mids when no builder-dex is selected,
//! so one connection replaces hundreds of per-spot `activeAssetCtx` watches.
//! Execution depth remains owned by the on-demand `l2Book` stream.

use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::broadcast::{error::RecvError, Receiver};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

const WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
const CHANNEL: &str = "allMids";
const CACHE_MAX_AGE_MS: i64 = 30_000;

#[derive(Debug)]
pub(super) struct AllMidsStream {
    manager: Arc<WsManager>,
    mids: Arc<DashMap<String, String>>,
    observed_at_ms: AtomicI64,
}

impl AllMidsStream {
    pub(super) fn new(venue: &'static str) -> Arc<Self> {
        let manager = Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: format!("{venue}-all-mids"),
            heartbeat_interval: Duration::from_secs(30),
            heartbeat: WsHeartbeat::Text(json!({"method": "ping"}).to_string()),
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        }));
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            mids: Arc::new(DashMap::new()),
            observed_at_ms: AtomicI64::new(0),
        });

        // Register before the supervisor can publish Connected; otherwise a
        // fast handshake can lose the only event that sends the subscription.
        let receiver = manager.subscribe();
        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "hyperliquid allMids ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch(receiver));
        stream
    }

    pub(super) fn snapshot(&self, coins: &[String]) -> Option<Vec<(String, String)>> {
        let observed_at_ms = self.observed_at_ms.load(Ordering::Relaxed);
        if observed_at_ms == 0 || now_ms().saturating_sub(observed_at_ms) > CACHE_MAX_AGE_MS {
            return None;
        }
        Some(
            coins
                .iter()
                .filter_map(|coin| {
                    self.mids
                        .get(coin)
                        .map(|mid| (coin.clone(), mid.value().clone()))
                })
                .collect(),
        )
    }

    async fn run_dispatch(self: Arc<Self>, mut receiver: Receiver<WsEvent>) {
        loop {
            match receiver.recv().await {
                Ok(event) => self.handle_event(event),
                Err(RecvError::Lagged(missed)) => {
                    self.clear();
                    warn!(missed, "hyperliquid allMids ws receiver lagged");
                }
                Err(RecvError::Closed) => {
                    self.clear();
                    return;
                }
            }
        }
    }

    fn handle_event(&self, event: WsEvent) {
        match event {
            WsEvent::Connected => self.subscribe(),
            WsEvent::Text(text) => self.on_text(&text),
            WsEvent::Disconnected(reason) => {
                self.clear();
                debug!(%reason, "hyperliquid allMids ws disconnected");
            }
            WsEvent::CircuitOpened => self.clear(),
            WsEvent::Binary(_) => {}
        }
    }

    fn subscribe(&self) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            let payload = json!({
                "method": "subscribe",
                "subscription": {"type": CHANNEL}
            });
            if let Err(error) = manager.send(Message::Text(payload.to_string())).await {
                warn!(error = %error, "hyperliquid allMids ws subscribe failed");
            }
        });
    }

    fn on_text(&self, text: &str) {
        let Some(mids) = parse_all_mids(text) else {
            return;
        };
        self.mids.clear();
        for (coin, mid) in mids {
            self.mids.insert(coin, mid);
        }
        self.observed_at_ms.store(now_ms(), Ordering::Relaxed);
    }

    fn clear(&self) {
        self.mids.clear();
        self.observed_at_ms.store(0, Ordering::Relaxed);
    }
}

fn parse_all_mids(text: &str) -> Option<Vec<(String, String)>> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("channel")?.as_str()? != CHANNEL {
        return None;
    }
    let mids = value.pointer("/data/mids")?.as_object()?;
    Some(
        mids.iter()
            .filter_map(|(coin, mid)| Some((coin.clone(), mid.as_str()?.to_owned())))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_all_mids_shape_with_spot_assets() {
        let rows = parse_all_mids(
            r#"{"channel":"allMids","data":{"mids":{"BTC":"63000.5","@1":"12.5"}}}"#,
        )
        .expect("allMids frame");

        assert!(rows.contains(&("BTC".into(), "63000.5".into())));
        assert!(rows.contains(&("@1".into(), "12.5".into())));
    }
}
