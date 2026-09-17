//! OKX public instrument-change WebSocket stream.
//!
//! The channel is incremental, so REST remains the initial snapshot and
//! reconnect recovery source. While the session is healthy, state, tick-size,
//! minimum-size, and listing updates override the REST snapshot immediately.
//!
//! Official docs:
//! - <https://www.okx.com/docs-v5/en/#public-data-websocket-instruments-channel>
//! - <https://www.okx.com/docs-v5/trick_en/#instrument-configuration>

use crate::adapters::okx_instruments::{OkxInstrumentRow, OkxInstrumentRule};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

const EXCHANGE: &str = "okx";
const WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const CHANNEL: &str = "instruments";
const PRODUCT: &str = "SWAP";

static PRODUCTION_STREAM: OnceLock<Arc<InstrumentStream>> = OnceLock::new();

pub(crate) fn production_stream() -> Arc<InstrumentStream> {
    Arc::clone(PRODUCTION_STREAM.get_or_init(InstrumentStream::new))
}

#[derive(Debug)]
pub(crate) struct InstrumentStream {
    manager: Arc<WsManager>,
    rules: Arc<DashMap<String, OkxInstrumentRule>>,
}

impl InstrumentStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: EXCHANGE.into(),
            heartbeat_interval: Duration::from_secs(20),
            heartbeat: WsHeartbeat::Text("ping".into()),
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        }));
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            rules: Arc::new(DashMap::new()),
        });

        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(async move {
            if let Err(error) = manager.run().await {
                warn!(error = %error, "okx instrument ws supervisor exited");
            }
        });
        stream
    }

    pub(crate) fn latest_rule(&self, inst_id: &str) -> Option<OkxInstrumentRule> {
        self.rules.get(inst_id).map(|entry| entry.value().clone())
    }

    pub(crate) fn rules(&self) -> Vec<OkxInstrumentRule> {
        self.rules
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    async fn run_dispatch(self: Arc<Self>) {
        let mut receiver = self.manager.subscribe();
        loop {
            match receiver.recv().await {
                Ok(event) => self.handle_event(event).await,
                Err(RecvError::Lagged(missed)) => {
                    self.rules.clear();
                    warn!(
                        missed,
                        "okx instrument ws receiver lagged; awaiting REST recovery"
                    );
                }
                Err(RecvError::Closed) => {
                    self.rules.clear();
                    return;
                }
            }
        }
    }

    async fn handle_event(&self, event: WsEvent) {
        if matches!(&event, WsEvent::Connected) {
            self.subscribe().await;
            return;
        }
        self.handle_non_connected_event(event);
    }

    fn handle_non_connected_event(&self, event: WsEvent) {
        match event {
            WsEvent::Text(text) => self.apply_text(&text),
            WsEvent::Disconnected(reason) => self.on_disconnected(&reason),
            WsEvent::CircuitOpened => self.on_circuit_opened(),
            WsEvent::Connected | WsEvent::Binary(_) => {}
        }
    }

    fn on_disconnected(&self, reason: &str) {
        self.rules.clear();
        debug!(%reason, "okx instrument ws disconnected; awaiting REST recovery");
    }

    fn on_circuit_opened(&self) {
        self.rules.clear();
        warn!("okx instrument ws circuit opened; awaiting REST recovery");
    }

    async fn subscribe(&self) {
        if let Err(error) = self
            .manager
            .send(Message::Text(subscription_payload()))
            .await
        {
            warn!(error = %error, "okx instrument ws subscribe failed");
        }
    }

    fn apply_text(&self, text: &str) {
        for rule in parse_update(text) {
            self.rules.insert(rule.inst_id.clone(), rule);
        }
    }
}

fn subscription_payload() -> String {
    json!({
        "id": "crossline-instruments-swap",
        "op": "subscribe",
        "args": [{"channel": CHANNEL, "instType": PRODUCT}],
    })
    .to_string()
}

fn parse_update(text: &str) -> Vec<OkxInstrumentRule> {
    let Ok(envelope) = serde_json::from_str::<InstrumentEnvelope>(text) else {
        return Vec::new();
    };
    let Some(arg) = envelope.arg else {
        return Vec::new();
    };
    if arg.channel != CHANNEL || arg.inst_type != PRODUCT {
        return Vec::new();
    }
    envelope
        .data
        .into_iter()
        .filter_map(|row| OkxInstrumentRule::from_row(row).ok())
        .collect()
}

#[derive(Debug, Deserialize)]
struct InstrumentEnvelope {
    arg: Option<InstrumentArg>,
    #[serde(default)]
    data: Vec<OkxInstrumentRow>,
}

#[derive(Debug, Deserialize)]
struct InstrumentArg {
    channel: String,
    #[serde(rename = "instType")]
    inst_type: String,
}

#[cfg(test)]
#[path = "okx_ws_instruments_tests.rs"]
mod tests;
