//! 行情策略共享的轻量转换函数。

use rust_decimal::prelude::ToPrimitive;
use shared_types::{normalized_venue_name, FundingRateData, SpotTick, TickerInfo, VenueId};

const QUOTES: &[&str] = &[
    "USDTM", "USDCM", "USDT0", "USDM", "USDH", "USDT", "USDC", "USD", "BTC", "ETH", "SOL",
];

pub(crate) const MAX_PRICE_TIMESTAMP_SKEW_MS: u64 = 2_000;

pub(crate) fn price_timestamps_are_synchronized<const N: usize>(
    timestamps: [Option<i64>; N],
) -> bool {
    let mut earliest = i64::MAX;
    let mut latest = i64::MIN;
    let mut observed = 0usize;
    for timestamp in timestamps.into_iter().flatten() {
        if timestamp <= 0 {
            return false;
        }
        earliest = earliest.min(timestamp);
        latest = latest.max(timestamp);
        observed = observed.saturating_add(1);
    }
    observed >= 2 && earliest.abs_diff(latest) <= MAX_PRICE_TIMESTAMP_SKEW_MS
}

pub(crate) fn spot_price(tick: &SpotTick) -> Option<f64> {
    positive_decimal(tick.ask)
}

pub(crate) fn spot_bid(tick: &SpotTick) -> Option<f64> {
    positive_decimal(tick.bid)
}

pub(crate) fn spot_volume_usd(tick: &SpotTick) -> f64 {
    positive_decimal(tick.volume_24h).unwrap_or(0.0)
}

pub(crate) fn canonical_base_symbol(symbol: &str) -> String {
    let symbol = symbol.split_once(':').map_or(symbol, |(_, base)| base);
    let upper = symbol.to_ascii_uppercase();
    split_pair(&upper)
        .map(|(base, _)| base.to_owned())
        .unwrap_or(upper)
}

pub(crate) fn canonical_quote_symbol(symbol: &str) -> Option<String> {
    let symbol = symbol.split_once(':').map_or(symbol, |(_, pair)| pair);
    let upper = symbol.to_ascii_uppercase();
    let (_, quote) = split_pair(&upper)?;
    Some(
        match quote {
            "USDTM" => "USDT",
            "USDCM" => "USDC",
            "USDM" => "USD",
            value => value,
        }
        .to_owned(),
    )
}

pub(crate) fn canonical_perp_quote_symbol(venue: &str, symbol: &str) -> Option<String> {
    canonical_quote_symbol(symbol).or_else(|| {
        VenueId::from_exchange_name(venue).and_then(|venue| match venue {
            VenueId::Hyperliquid => Some("USDC".to_owned()),
            VenueId::Binance
            | VenueId::Okx
            | VenueId::Bybit
            | VenueId::Bitget
            | VenueId::Gate
            | VenueId::Kucoin => Some("USDT".to_owned()),
            VenueId::GateCrossEx | VenueId::Htx | VenueId::Kraken => None,
        })
    })
}

/// Perpetual ticker adapters normalize venue symbols to their base asset before
/// the strategy scan. Reject a pair only when both original symbols still carry
/// explicit, conflicting quote assets; the downstream instrument registry owns
/// the final executable contract-identity check.
#[cfg(test)]
pub(crate) fn quote_assets_do_not_conflict(left: &str, right: &str) -> bool {
    let left = canonical_quote_symbol(left);
    let right = canonical_quote_symbol(right);
    quote_symbols_do_not_conflict(left.as_deref(), right.as_deref())
}

#[cfg(test)]
pub(crate) fn quote_symbols_do_not_conflict(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

pub(crate) fn venue_symbol_key(venue: &str, symbol: &str) -> (String, String) {
    (normalized_venue_name(venue), symbol.to_ascii_uppercase())
}

pub(crate) fn venue_base_symbol_key(venue: &str, symbol: &str) -> (String, String) {
    (normalized_venue_name(venue), canonical_base_symbol(symbol))
}

pub(crate) fn same_venue(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    if is_hyperliquid_builder(left) || is_hyperliquid_builder(right) {
        return false;
    }
    venue_id_without_allocation(left)
        .zip(venue_id_without_allocation(right))
        .is_some_and(|(left, right)| left == right)
}

pub(crate) fn venue_identity_key(venue: &str) -> String {
    let normalized = normalized_venue_name(venue);
    if is_hyperliquid_builder(&normalized) {
        return normalized;
    }
    venue_id_without_allocation(&normalized)
        .map(|venue| venue.as_str().to_owned())
        .unwrap_or(normalized)
}

fn is_hyperliquid_builder(venue: &str) -> bool {
    venue.split_once(':').is_some_and(|(family, builder)| {
        venue_id_without_allocation(family) == Some(VenueId::Hyperliquid)
            && !builder.trim().is_empty()
    })
}

fn venue_id_without_allocation(venue: &str) -> Option<VenueId> {
    let family = venue
        .trim()
        .split_once(':')
        .map_or(venue.trim(), |(family, _)| family.trim());
    let family = strip_live_suffix(family);
    if family.eq_ignore_ascii_case("binance") {
        Some(VenueId::Binance)
    } else if family.eq_ignore_ascii_case("okx") {
        Some(VenueId::Okx)
    } else if family.eq_ignore_ascii_case("bybit") {
        Some(VenueId::Bybit)
    } else if family.eq_ignore_ascii_case("bitget") {
        Some(VenueId::Bitget)
    } else if family.eq_ignore_ascii_case("gate") {
        Some(VenueId::Gate)
    } else if family.eq_ignore_ascii_case("gate_crossex")
        || family.eq_ignore_ascii_case("gate-crossex")
        || family.eq_ignore_ascii_case("crossex")
    {
        Some(VenueId::GateCrossEx)
    } else if family.eq_ignore_ascii_case("kraken") {
        Some(VenueId::Kraken)
    } else if family.eq_ignore_ascii_case("kucoin") {
        Some(VenueId::Kucoin)
    } else if family.eq_ignore_ascii_case("hyperliquid") {
        Some(VenueId::Hyperliquid)
    } else {
        None
    }
}

fn strip_live_suffix(value: &str) -> &str {
    ["_live", "-live"]
        .into_iter()
        .find_map(|suffix| {
            let split = value.len().checked_sub(suffix.len())?;
            let prefix = value.get(..split)?;
            value
                .get(split..)?
                .eq_ignore_ascii_case(suffix)
                .then_some(prefix)
        })
        .unwrap_or(value)
}

pub(crate) fn ticker_bid(ticker: &TickerInfo) -> Option<f64> {
    positive(ticker.bid)
}

pub(crate) fn ticker_ask(ticker: &TickerInfo) -> Option<f64> {
    positive(ticker.ask)
}

pub(crate) fn positive_decimal(value: rust_decimal::Decimal) -> Option<f64> {
    value.to_f64().and_then(positive)
}

pub(crate) fn positive(value: f64) -> Option<f64> {
    value.is_finite().then_some(value).filter(|v| *v > 0.0)
}

pub(crate) fn zero_rate(
    symbol: &str,
    exchange: &str,
    volume_24h: f64,
    timestamp: i64,
) -> FundingRateData {
    FundingRateData {
        symbol: symbol.to_owned(),
        exchange: exchange.to_owned(),
        rate: 0.0,
        rate_8h: 0.0,
        predicted_rate: None,
        next_funding_time: 0,
        funding_interval: 8,
        volume_24h,
        timestamp,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

/// 把 8h 标准化的费率（`FundingRateData::rate_8h`）年化为 bps。
///
/// `rate_8h` 由 adapter 统一归一化为"8 小时单期费率"，无论交易所原生周期是
/// 1h / 4h / 8h，调用方都应直接传入 `rate.rate_8h`，不要再用真实
/// `funding_interval` 二次缩放（那会把 1h 合约多算 8 倍、4h 合约多算 2 倍）。
///
/// 公式：`rate_8h * (24 / 8) * 365 * 10_000 = rate_8h * 3 * 365 * 10_000`。
pub fn annualize_rate_8h_bps(rate_8h: f64) -> f64 {
    rate_8h * 3.0 * 365.0 * 10_000.0
}

pub(crate) fn native_funding_rate(rate: &FundingRateData) -> f64 {
    finite_rate(rate.rate)
}

pub(crate) fn annualize_native_funding_bps(rate: &FundingRateData) -> f64 {
    if !rate.rate.is_finite() || rate.funding_interval == 0 {
        return 0.0;
    }
    rate.rate * 24.0 / f64::from(rate.funding_interval) * 365.0 * 10_000.0
}

fn finite_rate(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn split_pair(symbol: &str) -> Option<(&str, &str)> {
    for delimiter in ['/', '-', '_'] {
        if let Some((base, quote)) = symbol.split_once(delimiter) {
            return non_empty_pair(base, quote);
        }
    }
    for quote in QUOTES {
        if let Some(base) = symbol.strip_suffix(quote) {
            if base != symbol {
                return non_empty_pair(base, quote);
            }
        }
    }
    None
}

fn non_empty_pair<'a>(base: &'a str, quote: &'a str) -> Option<(&'a str, &'a str)> {
    (!base.is_empty() && !quote.is_empty()).then_some((base, quote))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_base_symbol_uppercases_and_strips_contract_suffixes() {
        assert_eq!(canonical_base_symbol("MUUSDTM"), "MU");
        assert_eq!(canonical_base_symbol("MAG7-USDH"), "MAG7");
        assert_eq!(canonical_base_symbol("BTCUSDT"), "BTC");
        assert_eq!(canonical_base_symbol("hype:btcusdt"), "BTC");
        assert_eq!(canonical_base_symbol("mu-usdc"), "MU");
        assert_eq!(canonical_base_symbol("mag7-usdh"), "MAG7");
    }

    #[test]
    fn canonical_quote_symbol_normalizes_contract_suffixes() {
        assert_eq!(canonical_quote_symbol("BTC/USDT").as_deref(), Some("USDT"));
        assert_eq!(canonical_quote_symbol("BTCUSDTM").as_deref(), Some("USDT"));
        assert_eq!(
            canonical_quote_symbol("hype:MU-USDC").as_deref(),
            Some("USDC")
        );
        assert_eq!(canonical_quote_symbol("BTC"), None);
        assert_eq!(
            canonical_perp_quote_symbol("binance", "BTC").as_deref(),
            Some("USDT")
        );
        assert_eq!(
            canonical_perp_quote_symbol("hyperliquid", "BTC").as_deref(),
            Some("USDC")
        );
        assert_eq!(
            canonical_perp_quote_symbol("binance", "BTCUSDC").as_deref(),
            Some("USDC")
        );
        assert_eq!(canonical_perp_quote_symbol("kraken", "BTC"), None);
        assert_eq!(
            canonical_perp_quote_symbol("gate_crossex:gate", "BTC"),
            None
        );
        assert!(quote_assets_do_not_conflict("BTC", "BTC"));
        assert!(quote_assets_do_not_conflict("BTC", "BTC/USDT"));
        assert!(!quote_assets_do_not_conflict("BTC/USDT", "BTC/USDC"));
    }

    #[test]
    fn venue_equality_keeps_aliases_and_builder_dexes_distinct() {
        assert!(same_venue(" OKX-LIVE ", "okx"));
        assert!(same_venue("gate-crossex", "crossex"));
        assert!(same_venue(" Hyperliquid:XYZ ", "hyperliquid:xyz"));
        assert!(!same_venue("hyperliquid:xyz", "hyperliquid:km"));
        assert!(!same_venue("hyperliquid:xyz", "hyperliquid"));
        assert!(!same_venue("unknown-a", "unknown-b"));
    }

    #[test]
    fn venue_identity_key_collapses_aliases_but_not_builder_dexes() {
        assert_eq!(venue_identity_key(" OKX-LIVE "), "okx");
        assert_eq!(venue_identity_key("crossex"), "gate_crossex");
        assert_eq!(venue_identity_key("Hyperliquid:XYZ"), "hyperliquid:xyz");
        assert_ne!(
            venue_identity_key("hyperliquid:xyz"),
            venue_identity_key("hyperliquid:km")
        );
    }

    #[test]
    fn annualizes_the_native_funding_interval_without_rate_8h() {
        let mut rate = zero_rate("BTC", "binance", 1.0, 1);
        rate.rate = 0.000_01;
        rate.rate_8h = 0.123;
        rate.funding_interval = 1;

        assert!((annualize_native_funding_bps(&rate) - 876.0).abs() < 1e-9);
    }

    #[test]
    fn executable_prices_require_positive_synchronized_timestamps() {
        assert!(price_timestamps_are_synchronized([
            Some(10_000),
            Some(12_000),
            None,
        ]));
        assert!(!price_timestamps_are_synchronized([
            Some(10_000),
            Some(12_001),
            None,
        ]));
        assert!(!price_timestamps_are_synchronized(
            [Some(0), Some(1), None,]
        ));
    }
}
