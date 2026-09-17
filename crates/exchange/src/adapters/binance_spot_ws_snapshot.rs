//! Binance Spot WebSocket API cold-start baseline.
//!
//! The push `@ticker` stream can remain silent for an inactive symbol until the
//! next trade. One native all-symbol `ticker.24hr` request supplies a bounded
//! 30-second baseline. Exact watched markets never wait on this request; their
//! push subscriptions remain authoritative on the projection hot path.
//!
//! Official docs:
//! <https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/ws-api/market>

use super::binance_config::BinanceConfig;
use crate::error::{ExchangeError, ExchangeResult};
use crate::ws::trade_session::{WsSessionSpec, WsTradeSession};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::SpotTick;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::OnceLock;
use tracing::warn;

const EXCHANGE: &str = "binance";
const WS_API_URL: &str = "wss://ws-api.binance.com:443/ws-api/v3";
const METHOD_TICKER_24H: &str = "ticker.24hr";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const MAX_BOOTSTRAP_SYMBOLS: usize = 100;

#[derive(Clone)]
struct CachedRow {
    tick: SpotTick,
    cached_at_ms: i64,
}

pub(crate) async fn supplement_spot_ticks(
    config: &BinanceConfig,
    symbols: &[String],
    rows: Vec<SpotTick>,
) -> ExchangeResult<Vec<SpotTick>> {
    if config.testnet || config.base_url_override.is_some() {
        return Ok(rows);
    }
    let requested = normalized_symbols(symbols);
    if requested.is_empty() {
        return Ok(rows);
    }

    let mut by_symbol = rows_by_native_symbol(rows);
    let now = now_ms();
    let cache = cache();
    cache.retain(|_, row| is_fresh(row.cached_at_ms, now));
    hydrate_from_cache(&requested, cache, &mut by_symbol);

    if !needs_full_market_baseline(requested.len()) {
        return Ok(take_requested_rows(&requested, &mut by_symbol));
    }
    supplement_full_market(config, &requested, cache, by_symbol, now).await
}

async fn supplement_full_market(
    config: &BinanceConfig,
    requested: &[String],
    cache: &DashMap<String, CachedRow>,
    mut by_symbol: HashMap<String, SpotTick>,
    now_ms: i64,
) -> ExchangeResult<Vec<SpotTick>> {
    if claim_full_snapshot_refresh(now_ms) {
        match fetch_all(config.timeout_secs).await {
            Ok(fetched) => cache_rows(cache, &mut by_symbol, fetched),
            Err(error) if by_symbol.is_empty() => return Err(error),
            Err(error) => warn!(
                error = %error,
                "binance full-market spot ws baseline failed; keeping push rows"
            ),
        }
    }
    hydrate_from_cache(requested, cache, &mut by_symbol);
    Ok(take_requested_rows(requested, &mut by_symbol))
}

fn cache_rows(
    cache: &DashMap<String, CachedRow>,
    by_symbol: &mut HashMap<String, SpotTick>,
    fetched: Vec<(String, SpotTick)>,
) {
    let cached_at_ms = now_ms();
    for (symbol, tick) in fetched {
        cache.insert(
            symbol.clone(),
            CachedRow {
                tick: tick.clone(),
                cached_at_ms,
            },
        );
        by_symbol.insert(symbol, tick);
    }
}

fn take_requested_rows(
    requested: &[String],
    by_symbol: &mut HashMap<String, SpotTick>,
) -> Vec<SpotTick> {
    requested
        .iter()
        .filter_map(|symbol| by_symbol.remove(symbol))
        .collect()
}

fn normalized_symbols(symbols: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    symbols
        .iter()
        .filter_map(|symbol| crate::spot::compact_pair_symbol(symbol))
        .map(|symbol| symbol.to_ascii_uppercase())
        .filter(|symbol| seen.insert(symbol.clone()))
        .collect()
}

fn needs_full_market_baseline(requested_count: usize) -> bool {
    requested_count > MAX_BOOTSTRAP_SYMBOLS
}

fn rows_by_native_symbol(rows: Vec<SpotTick>) -> HashMap<String, SpotTick> {
    rows.into_iter()
        .filter_map(|row| {
            crate::spot::compact_pair_symbol(&row.symbol)
                .map(|symbol| (symbol.to_ascii_uppercase(), row))
        })
        .collect()
}

fn hydrate_from_cache(
    requested: &[String],
    cache: &DashMap<String, CachedRow>,
    rows: &mut HashMap<String, SpotTick>,
) {
    for symbol in requested {
        if rows.contains_key(symbol) {
            continue;
        }
        if let Some(cached) = cache.get(symbol) {
            rows.insert(symbol.clone(), cached.tick.clone());
        }
    }
}

async fn fetch_all(timeout_secs: u64) -> ExchangeResult<Vec<(String, SpotTick)>> {
    let request_id = next_request_id();
    let expected_id = request_id.clone();
    let text = session(timeout_secs)
        .send(
            all_market_request_payload(&request_id),
            Box::new(move |text| response_matches_id(text, &expected_id)),
        )
        .await?;
    parse_response(&text)
}

fn session(timeout_secs: u64) -> WsTradeSession {
    static SESSION: OnceLock<WsTradeSession> = OnceLock::new();
    SESSION
        .get_or_init(|| WsTradeSession::spawn(WsSessionSpec::new(WS_API_URL, timeout_secs.max(1))))
        .clone()
}

fn cache() -> &'static DashMap<String, CachedRow> {
    static CACHE: OnceLock<DashMap<String, CachedRow>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

fn next_request_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("crossline-spot-{}-{sequence}", now_ms())
}

fn claim_full_snapshot_refresh(now_ms: i64) -> bool {
    static LAST_ATTEMPT_MS: AtomicI64 = AtomicI64::new(0);
    let last = LAST_ATTEMPT_MS.load(Ordering::Acquire);
    if now_ms.saturating_sub(last) < CACHE_MAX_AGE_MS {
        return false;
    }
    LAST_ATTEMPT_MS
        .compare_exchange(last, now_ms, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn all_market_request_payload(request_id: &str) -> String {
    json!({
        "id": request_id,
        "method": METHOD_TICKER_24H,
        "params": {
            "type": "FULL",
            "symbolStatus": "TRADING"
        }
    })
    .to_string()
}

fn response_matches_id(text: &str, expected_id: &str) -> ExchangeResult<bool> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("binance spot ws response id: {error}; body={text}"))
    })?;
    Ok(value.get("id").and_then(Value::as_str) == Some(expected_id))
}

fn parse_response(text: &str) -> ExchangeResult<Vec<(String, SpotTick)>> {
    let response: TickerResponse = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!(
            "binance spot ws ticker response: {error}; body={text}"
        ))
    })?;
    if !(200..300).contains(&response.status) {
        let error = response.error.unwrap_or_default();
        return Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: error.code.to_string(),
            message: error.msg,
        });
    }
    Ok(response
        .result
        .into_iter()
        .filter_map(|row| {
            let symbol = row.symbol.to_ascii_uppercase();
            parse_row(&row).map(|tick| (symbol, tick))
        })
        .collect())
}

fn parse_row(row: &TickerRow) -> Option<SpotTick> {
    let (base, quote) = crate::spot::suffix_pair(&row.symbol)?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: EXCHANGE,
        base: &base,
        quote: &quote,
        bid: &row.bid_price,
        ask: &row.ask_price,
        last: &row.last_price,
        bid_size: Some(&row.bid_qty),
        ask_size: Some(&row.ask_qty),
        volume_24h: &row.quote_volume,
        exchange_ts_ms: (row.close_time > 0).then_some(row.close_time),
    })
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

#[derive(Debug, Deserialize)]
struct TickerResponse {
    status: u16,
    #[serde(default)]
    result: Vec<TickerRow>,
    #[serde(default)]
    error: Option<ErrorBody>,
}

#[derive(Debug, Default, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TickerRow {
    symbol: String,
    last_price: String,
    quote_volume: String,
    bid_price: String,
    bid_qty: String,
    ask_price: String,
    ask_qty: String,
    close_time: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use serde_json::Value;

    #[test]
    fn all_market_request_omits_symbol_filters() {
        let payload = all_market_request_payload("req-all");
        let value: Value = serde_json::from_str(&payload).expect("request json");
        assert_eq!(value["method"], "ticker.24hr");
        assert_eq!(value["params"]["type"], "FULL");
        assert_eq!(value["params"]["symbolStatus"], "TRADING");
        assert!(value["params"].get("symbol").is_none());
        assert!(value["params"].get("symbols").is_none());
    }

    #[test]
    fn exact_watchlists_never_enter_the_blocking_baseline_path() {
        assert!(!needs_full_market_baseline(1));
        assert!(!needs_full_market_baseline(MAX_BOOTSTRAP_SYMBOLS));
        assert!(needs_full_market_baseline(MAX_BOOTSTRAP_SYMBOLS + 1));
    }

    #[test]
    fn response_builds_complete_official_spot_row() {
        let text = r#"{
          "id":"req-1","status":200,
          "result":[{
            "symbol":"COTIUSDT","lastPrice":"0.0512","quoteVolume":"125000.5",
            "bidPrice":"0.0511","bidQty":"200","askPrice":"0.0513","askQty":"180",
            "closeTime":1720000000123
          }]
        }"#;
        let rows = parse_response(text).expect("response parses");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "COTIUSDT");
        assert_eq!(rows[0].1.symbol, "COTI/USDT");
        assert_eq!(rows[0].1.bid, Decimal::new(511, 4));
        assert_eq!(rows[0].1.ask_size, Some(Decimal::new(180, 0)));
        assert_eq!(rows[0].1.exchange_ts_ms, Some(1_720_000_000_123));
    }

    #[test]
    fn response_surfaces_official_api_error() {
        let error = parse_response(
            r#"{"id":"req-1","status":400,"error":{"code":-1121,"msg":"Invalid symbol."}}"#,
        )
        .expect_err("api error");
        assert!(error.to_string().contains("-1121"));
    }
}
