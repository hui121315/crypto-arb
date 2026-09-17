use common::time::now_ms;
use serde::Deserialize;
use serde_json::json;
use shared_types::OrderBookInfo;

const EXCHANGE: &str = "binance";

pub(super) fn subscription_payload(method: &str, symbols: &[String], request_id: u64) -> String {
    let params = symbols
        .iter()
        .map(|symbol| format!("{symbol}@depth20@100ms"))
        .collect::<Vec<_>>();
    json!({
        "method": method,
        "params": params,
        "id": request_id,
    })
    .to_string()
}

pub(super) fn parse_snapshot(text: &str) -> Option<(String, OrderBookInfo)> {
    let envelope: CombinedEnvelope = serde_json::from_str(text).ok()?;
    let symbol = envelope.stream.split('@').next()?.to_owned();
    let bids = parse_levels(&envelope.data.bids);
    let asks = parse_levels(&envelope.data.asks);
    if bids.is_empty() || asks.is_empty() {
        return None;
    }
    Some((
        symbol.clone(),
        OrderBookInfo {
            symbol: crate::spot::native_pair_symbol(&symbol, '/')?,
            exchange: EXCHANGE.into(),
            bids,
            asks,
            timestamp: now_ms(),
        },
    ))
}

fn parse_levels(levels: &[Vec<String>]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|level| Some([level.first()?.parse().ok()?, level.get(1)?.parse().ok()?]))
        .collect()
}

#[derive(Debug, Deserialize)]
struct CombinedEnvelope {
    stream: String,
    data: DepthPayload,
}

#[derive(Debug, Deserialize)]
struct DepthPayload {
    #[serde(default)]
    bids: Vec<Vec<String>>,
    #[serde(default)]
    asks: Vec<Vec<String>>,
}
