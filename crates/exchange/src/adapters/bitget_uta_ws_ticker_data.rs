use super::bitget_uta_config::{BitgetUtaCategory, BitgetUtaWsArgs};
use crate::adapter::strip_common_suffixes;
use common::time::now_ms;
use serde::Deserialize;
use serde_json::json;
use shared_types::{FundingRateData, MarkIndexInfo, TickerInfo};

pub(super) const EXCHANGE: &str = "bitget";
pub(super) const TOPIC: &str = "ticker";

#[derive(Debug, Clone)]
pub(super) struct CachedTicker {
    /// 交易对（`BTCUSDT`）。v3 ticker 帧的 `data[]` 项不带 symbol 字段，
    /// 只能取自信封 `arg.symbol` —— 这是缓存键的唯一事实源。
    pub(super) symbol: String,
    pub(super) item: TickerItem,
    /// 信封级 `ts`（毫秒）。`data[]` 项同样不带时间戳。
    pub(super) ts_ms: i64,
    pub(super) cached_at_ms: i64,
}

pub(super) fn stream_symbol(symbol: &str) -> String {
    let upper = strip_common_suffixes(symbol);
    format!("{upper}USDT")
}

pub(super) fn channel_payload(op: &str, symbols: &[String]) -> String {
    let args: Vec<_> = symbols
        .iter()
        .map(|symbol| {
            BitgetUtaWsArgs::new(
                BitgetUtaCategory::UsdtFutures,
                TOPIC,
                &stream_symbol(symbol),
            )
        })
        .collect();
    json!({ "op": op, "args": args }).to_string()
}

pub(super) fn parse_ticker_update(text: &str) -> Vec<CachedTicker> {
    let Ok(envelope) = serde_json::from_str::<TickerEnvelope>(text) else {
        return Vec::new();
    };
    if envelope.arg.topic != TOPIC || envelope.arg.symbol.is_empty() {
        return Vec::new();
    }
    let now = now_ms();
    let ts_ms = envelope.ts.unwrap_or(now);
    envelope
        .data
        .into_iter()
        .map(|item| CachedTicker {
            symbol: envelope.arg.symbol.clone(),
            item,
            ts_ms,
            cached_at_ms: now,
        })
        .collect()
}

/// Parse a Bitget UTA WS ticker, failing closed when a required price is absent.
///
/// `bid1Price`/`ask1Price`/`lastPrice` drive execution and risk decisions, so a
/// missing/zero/unparseable value yields `None` (the tick is dropped) instead of
/// a fabricated `0.0` price. 24h volume stays best-effort.
pub(super) fn parse_ticker(row: &CachedTicker) -> Option<TickerInfo> {
    let item = &row.item;
    let bid = parse_positive(&item.bid1_price)?;
    let ask = parse_positive(&item.ask1_price)?;
    let last = parse_positive(&item.last_price)?;
    Some(TickerInfo {
        symbol: strip_common_suffixes(&row.symbol),
        exchange: EXCHANGE.into(),
        bid,
        ask,
        last,
        volume_24h: parse_quote_volume(item),
        timestamp: row.ts_ms,
    })
}

pub(super) fn parse_mark_index(row: &CachedTicker) -> Option<MarkIndexInfo> {
    let item = &row.item;
    Some(MarkIndexInfo {
        symbol: strip_common_suffixes(&row.symbol),
        exchange: EXCHANGE.into(),
        mark_price: parse_positive(&item.mark_price)?,
        index_price: parse_optional_f64(&item.index_price),
        open_interest: parse_optional_f64(&item.open_interest),
        open_interest_value: None,
        timestamp: row.ts_ms,
    })
}

/// 从 ticker 帧装配 funding：帧带 fundingRate/nextFundingTime，但不带
/// interval——由 REST instruments 回灌（`seed_funding_intervals`），缺席则跳过
/// 该符号（部分服务，不猜 interval 以免年化算错）。
pub(super) fn parse_funding(row: &CachedTicker, interval_hours: u32) -> Option<FundingRateData> {
    let rate = parse_optional_f64(&row.item.funding_rate)?;
    if interval_hours == 0 {
        return None;
    }
    let next_funding_time = row
        .item
        .next_funding_time
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)?;
    Some(FundingRateData {
        symbol: strip_common_suffixes(&row.symbol),
        exchange: EXCHANGE.into(),
        rate,
        rate_8h: rate * (8.0 / f64::from(interval_hours)),
        predicted_rate: None,
        next_funding_time,
        funding_interval: interval_hours,
        volume_24h: parse_quote_volume(&row.item),
        timestamp: row.ts_ms,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

pub(super) fn parse_quote_volume(item: &TickerItem) -> f64 {
    let quote = parse_f64(&item.turnover_24h);
    if quote > 0.0 {
        return quote;
    }
    parse_f64(&item.volume_24h)
}

fn parse_f64(value: &str) -> f64 {
    value.parse().unwrap_or(0.0)
}

fn parse_optional_f64(value: &str) -> Option<f64> {
    value.parse().ok().filter(|value: &f64| value.is_finite())
}

fn parse_positive(value: &str) -> Option<f64> {
    parse_optional_f64(value).filter(|value| *value > 0.0)
}

/// 2026-07-27 生产实抓帧结构：
/// `{"action":"snapshot","arg":{"instType":"usdt-futures","topic":"ticker",
///   "symbol":"BTCUSDT"},"data":[{...无 symbol/ts...}],"ts":1785088069374}`
#[derive(Debug, Deserialize)]
struct TickerEnvelope {
    arg: TickerArg,
    #[serde(default)]
    data: Vec<TickerItem>,
    #[serde(default)]
    ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TickerArg {
    topic: String,
    #[serde(default)]
    symbol: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct TickerItem {
    #[serde(default, rename = "lastPrice")]
    pub(super) last_price: String,
    #[serde(default, rename = "bid1Price")]
    pub(super) bid1_price: String,
    #[serde(default, rename = "ask1Price")]
    pub(super) ask1_price: String,
    #[serde(default, rename = "volume24h")]
    pub(super) volume_24h: String,
    #[serde(default, rename = "turnover24h")]
    pub(super) turnover_24h: String,
    #[serde(default, rename = "markPrice")]
    pub(super) mark_price: String,
    #[serde(default, rename = "indexPrice")]
    pub(super) index_price: String,
    #[serde(default, rename = "openInterest")]
    pub(super) open_interest: String,
    #[serde(default, rename = "fundingRate")]
    pub(super) funding_rate: String,
    #[serde(default, rename = "nextFundingTime")]
    pub(super) next_funding_time: String,
}
