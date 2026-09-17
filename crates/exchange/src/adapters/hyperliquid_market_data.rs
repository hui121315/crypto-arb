//! Hyperliquid public market response parsing.

use common::time::now_ms;
use serde::Deserialize;
use shared_types::{FundingRateData, MarkIndexInfo, SpotTick, TickerInfo};
use std::collections::HashMap;

const NAME: &str = "hyperliquid";
const FUNDING_INTERVAL_HOURS: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct UniverseEntry {
    pub(super) name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct UniverseWrapper {
    pub(super) universe: Vec<UniverseEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SpotMetaWrapper {
    #[serde(default = "Vec::new")]
    pub(super) tokens: Vec<SpotToken>,
    #[serde(default = "Vec::new")]
    pub(super) universe: Vec<SpotUniverseEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SpotToken {
    pub(super) name: String,
    pub(super) index: u32,
    #[serde(default, rename = "szDecimals")]
    pub(super) sz_decimals: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SpotUniverseEntry {
    pub(super) name: String,
    #[serde(default = "Vec::new")]
    pub(super) tokens: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AssetCtx {
    #[serde(default)]
    pub(super) coin: Option<String>,
    #[serde(default)]
    pub(super) funding: String,
    #[serde(default, rename = "markPx")]
    pub(super) mark_px: String,
    #[serde(default, rename = "oraclePx")]
    pub(super) oracle_px: String,
    #[serde(default, rename = "openInterest")]
    pub(super) open_interest: String,
    #[serde(default, rename = "midPx")]
    pub(super) mid_px: Option<String>,
    #[serde(default, rename = "dayNtlVlm")]
    pub(super) day_ntl_vlm: String,
    #[serde(default, rename = "impactPxs")]
    pub(super) impact_pxs: Option<[String; 2]>,
}

pub(super) type PredictedFundingsResponse =
    Vec<(String, Vec<(String, Option<PredictedFundingInfo>)>)>;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PredictedFundingInfo {
    #[serde(default, rename = "fundingRate")]
    funding_rate: String,
    #[serde(default, rename = "nextFundingTime")]
    next_funding_time: i64,
}

pub(super) struct PredictedFundingEntry {
    rate: f64,
    next_funding_ms: i64,
}

pub(super) fn build_predicted_map(
    data: &PredictedFundingsResponse,
) -> HashMap<String, PredictedFundingEntry> {
    const HL_VENUE: &str = "HlPerp";
    let mut map = HashMap::with_capacity(data.len());
    for (coin, venues) in data {
        for (venue, info) in venues {
            if venue == HL_VENUE {
                let Some(info) = info else {
                    break;
                };
                if let Ok(rate) = info.funding_rate.parse::<f64>() {
                    if rate.is_finite() {
                        map.insert(
                            coin.clone(),
                            PredictedFundingEntry {
                                rate,
                                next_funding_ms: info.next_funding_time,
                            },
                        );
                    }
                }
                break;
            }
        }
    }
    map
}

#[derive(Debug, Deserialize)]
pub(super) struct L2Book {
    pub(super) levels: [Vec<L2Level>; 2],
    pub(super) time: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct L2Level {
    px: String,
    sz: String,
}

pub(super) fn parse_levels(levels: &[L2Level]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|level| {
            let price = level.px.parse().ok()?;
            let size = level.sz.parse().ok()?;
            Some([price, size])
        })
        .collect()
}

pub(super) fn hyperliquid_symbol_matches(raw: &str, target: &str) -> bool {
    clean_hyperliquid_symbol(raw).eq_ignore_ascii_case(target)
}

pub(super) fn ticker_rows(
    venue: &str,
    meta: &UniverseWrapper,
    ctxs: &[AssetCtx],
) -> Vec<TickerInfo> {
    let mut rows = Vec::with_capacity(meta.universe.len());
    for (index, entry) in meta.universe.iter().enumerate() {
        let Some(ctx) = ctxs.get(index) else {
            continue;
        };
        if let Some(row) =
            parse_ticker_for_venue(venue, &clean_hyperliquid_symbol(&entry.name), ctx)
        {
            rows.push(row);
        }
    }
    rows
}

pub(super) fn funding_rows(
    venue: &str,
    meta: &UniverseWrapper,
    ctxs: &[AssetCtx],
    predicted_map: &HashMap<String, PredictedFundingEntry>,
) -> Vec<FundingRateData> {
    let mut rows = Vec::with_capacity(meta.universe.len());
    for (index, entry) in meta.universe.iter().enumerate() {
        let Some(ctx) = ctxs.get(index) else {
            continue;
        };
        let symbol = clean_hyperliquid_symbol(&entry.name);
        let (predicted_rate, next_funding_time) = predicted_map
            .get(symbol.as_str())
            .map(|predicted| (Some(predicted.rate), predicted.next_funding_ms))
            .unwrap_or((None, 0));
        rows.push(parse_funding_for_venue(
            venue,
            &symbol,
            ctx,
            predicted_rate,
            next_funding_time,
        ));
    }
    rows
}

/// Parse a Hyperliquid asset context into a ticker, failing closed on price.
///
/// `last` is the mark price and must be a positive finite value or the tick is
/// dropped (returns `None`) instead of fabricating a 0.0 quote the opportunity
/// scanner / risk would treat as real. `bid`/`ask` prefer the positive
/// `impactPxs` BBO and otherwise fall back to the positive mid (else the mark).
/// `volume_24h` stays best-effort (non-execution).
pub(super) fn parse_ticker_for_venue(
    venue: &str,
    symbol: &str,
    ctx: &AssetCtx,
) -> Option<TickerInfo> {
    let last = parse_positive(&ctx.mark_px)?;
    let mid = ctx
        .mid_px
        .as_deref()
        .and_then(parse_positive)
        .unwrap_or(last);
    let (bid, ask) = ctx
        .impact_pxs
        .as_ref()
        .and_then(|prices| Some((parse_positive(&prices[0])?, parse_positive(&prices[1])?)))
        .unwrap_or((mid, mid));
    Some(TickerInfo {
        symbol: symbol.to_owned(),
        exchange: venue.into(),
        bid,
        ask,
        last,
        volume_24h: parse_f64(&ctx.day_ntl_vlm),
        timestamp: now_ms(),
    })
}

pub(super) fn parse_mark_index_for_venue(
    venue: &str,
    symbol: &str,
    ctx: &AssetCtx,
) -> Option<MarkIndexInfo> {
    Some(MarkIndexInfo {
        symbol: symbol.to_owned(),
        exchange: venue.to_owned(),
        mark_price: parse_positive(&ctx.mark_px)?,
        index_price: parse_optional_f64(&ctx.oracle_px),
        open_interest: parse_optional_f64(&ctx.open_interest),
        open_interest_value: None,
        timestamp: now_ms(),
    })
}

fn parse_funding_for_venue(
    venue: &str,
    symbol: &str,
    ctx: &AssetCtx,
    predicted_rate: Option<f64>,
    next_funding_time: i64,
) -> FundingRateData {
    let rate = parse_f64(&ctx.funding);
    FundingRateData {
        symbol: symbol.to_owned(),
        exchange: venue.to_owned(),
        rate,
        rate_8h: rate * 8.0,
        predicted_rate,
        next_funding_time,
        funding_interval: FUNDING_INTERVAL_HOURS,
        volume_24h: parse_f64(&ctx.day_ntl_vlm),
        timestamp: now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

pub(super) fn clean_hyperliquid_symbol(symbol: &str) -> String {
    symbol
        .split_once(':')
        .map_or(symbol, |(_, base)| base)
        .to_owned()
}

pub(super) fn parse_spot_tick(base: &str, quote: &str, ctx: &AssetCtx) -> Option<SpotTick> {
    let mid = ctx.mid_px.as_deref().unwrap_or(&ctx.mark_px);
    let bid = ctx
        .impact_pxs
        .as_ref()
        .map(|prices| prices[0].as_str())
        .unwrap_or(mid);
    let ask = ctx
        .impact_pxs
        .as_ref()
        .map(|prices| prices[1].as_str())
        .unwrap_or(mid);
    let last = if ctx.mark_px.is_empty() {
        mid
    } else {
        &ctx.mark_px
    };
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: NAME,
        base,
        quote,
        bid,
        ask,
        last,
        bid_size: None,
        ask_size: None,
        volume_24h: &ctx.day_ntl_vlm,
        exchange_ts_ms: None,
    })
}

pub(super) fn spot_token_names(tokens: &[SpotToken]) -> HashMap<u32, String> {
    tokens
        .iter()
        .map(|token| (token.index, token.name.to_ascii_uppercase()))
        .collect()
}

pub(super) fn spot_pair(
    entry: &SpotUniverseEntry,
    token_names: &HashMap<u32, String>,
) -> Option<(String, String)> {
    crate::spot::delimited_pair(&entry.name, '/').or_else(|| {
        let base = token_names.get(entry.tokens.first()?)?.clone();
        let quote = token_names.get(entry.tokens.get(1)?)?.clone();
        Some((base, quote))
    })
}

pub(super) fn spot_contexts_by_coin(contexts: &[AssetCtx]) -> Option<HashMap<&str, &AssetCtx>> {
    let rows = contexts
        .iter()
        .filter_map(|context| {
            context
                .coin
                .as_deref()
                .map(str::trim)
                .filter(|coin| !coin.is_empty())
                .map(|coin| (coin, context))
        })
        .collect::<HashMap<_, _>>();
    (!rows.is_empty()).then_some(rows)
}

pub(super) fn spot_context_for_entry<'a>(
    entry: &SpotUniverseEntry,
    index: usize,
    contexts: &'a [AssetCtx],
    contexts_by_coin: Option<&HashMap<&str, &'a AssetCtx>>,
) -> Option<&'a AssetCtx> {
    match contexts_by_coin {
        Some(rows) => rows.get(entry.name.trim()).copied(),
        None => contexts.get(index),
    }
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

#[cfg(test)]
#[path = "hyperliquid_market_data_tests.rs"]
mod tests;
