//! 永续跨所候选枚举。
//!
//! 在同一个市场索引里绑定 Funding、精确永续合约 BBO 与报价币换算，再枚举
//! “做多低费率场所 + 做空高费率场所”的同窗原生结算机会。

use crate::algorithms::{
    funding_timeline, market,
    market_index::{IndexedFunding, MarketScanIndex},
    quote_conversion::{self, QuoteConversion},
};
use crate::models::{RawOpportunity, RawOpportunityExtra};
use shared_types::{ArbitrageType, StrategyKind};
#[cfg(test)]
use shared_types::{FundingRateData, TickerInfo};
#[cfg(test)]
use std::collections::HashMap;

const EDGE_VENUES_PER_SIDE: usize = 8;

#[cfg(test)]
pub(crate) fn scan(
    snapshot: &HashMap<String, HashMap<String, FundingRateData>>,
    min_spread: f64,
    min_volume_24h: f64,
) -> Vec<RawOpportunity> {
    let mut funding = snapshot
        .values()
        .flat_map(|by_venue| by_venue.values().cloned())
        .collect::<Vec<_>>();
    for row in &mut funding {
        if market::canonical_quote_symbol(&row.symbol).is_none() {
            row.symbol = format!("{}USDT", market::canonical_base_symbol(&row.symbol));
        }
    }
    let tickers = funding
        .iter()
        .map(|row| TickerInfo {
            symbol: row.symbol.clone(),
            exchange: row.exchange.clone(),
            bid: 100.0,
            ask: 100.0,
            last: 100.0,
            volume_24h: row.volume_24h,
            timestamp: row.timestamp,
        })
        .collect::<Vec<_>>();
    let index = MarketScanIndex::from_slices(&[], &tickers, &funding);
    scan_indexed(&index, min_spread, min_volume_24h)
}

pub(crate) fn scan_indexed(
    index: &MarketScanIndex<'_>,
    min_spread: f64,
    min_volume_24h: f64,
) -> Vec<RawOpportunity> {
    let mut out = Vec::new();
    for (symbol, rates) in index.funding_groups() {
        let entries: Vec<&IndexedFunding<'_>> = rates
            .iter()
            .filter(|rate| rate.row.volume_24h >= min_volume_24h)
            .collect();
        if entries.len() < 2 {
            continue;
        }

        append_symbol_pairs(symbol, entries, index, min_spread, &mut out);
    }
    // Rank by the native joint-event yield. The legacy `spread_8h` DTO slot must never
    // influence P0 strategy decisions.
    out.sort_by(compare_opportunity_quality);
    out
}

fn append_symbol_pairs(
    symbol: &str,
    mut entries: Vec<&IndexedFunding<'_>>,
    index: &MarketScanIndex<'_>,
    min_spread: f64,
    out: &mut Vec<RawOpportunity>,
) {
    entries.sort_by(|a, b| {
        market::native_funding_rate(a.row)
            .partial_cmp(&market::native_funding_rate(b.row))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut pairs = Vec::new();
    // Hot-path bound: only compare top/bottom EDGE_VENUES_PER_SIDE venues per symbol.
    // This produces at most 64 in-memory pairs. Keep the complete bounded frontier here;
    // the engine ranks it after attaching BBO, WS-health and fee evidence, then applies
    // the smaller product-facing per-symbol cap.
    for low in entries.iter().take(EDGE_VENUES_PER_SIDE) {
        for high in entries.iter().rev().take(EDGE_VENUES_PER_SIDE) {
            if low.venue_key == high.venue_key {
                continue;
            }
            if let Some(row) = prepare_pair(symbol, low, high, index, min_spread) {
                pairs.push(row);
            }
        }
    }
    pairs.sort_by(compare_opportunity_quality);
    out.extend(pairs);
}

fn prepare_pair(
    symbol: &str,
    low: &IndexedFunding<'_>,
    high: &IndexedFunding<'_>,
    index: &MarketScanIndex<'_>,
    min_spread: f64,
) -> Option<RawOpportunity> {
    let low_data = low.row;
    let high_data = high.row;
    let joint_event = funding_timeline::current_joint_event_input(
        funding_timeline::FundingTimelineInput {
            long_rate: market::native_funding_rate(low_data),
            long_next_ms: low_data.next_funding_time,
            long_interval_hours: low_data.funding_interval,
            short_rate: market::native_funding_rate(high_data),
            short_next_ms: high_data.next_funding_time,
            short_interval_hours: high_data.funding_interval,
        },
        low_data.timestamp.max(high_data.timestamp),
    )?;
    let native_edge = joint_event.funding_yield;
    if native_edge < min_spread {
        return None;
    }

    let low_quote = low.resolved_quote.as_deref()?;
    let high_quote = high.resolved_quote.as_deref()?;
    let low_ticker = index.perp_by_key(&low.venue_key, symbol, low_quote)?;
    let high_ticker = index.perp_by_key(&high.venue_key, symbol, high_quote)?;
    let low_ask = market::ticker_ask(low_ticker)?;
    let high_bid = market::ticker_bid(high_ticker)?;
    let (open_conversion, close_conversion) = quote_route(
        index,
        [&high.venue_key, &low.venue_key],
        high_quote,
        low_quote,
    )?;
    if !market::price_timestamps_are_synchronized([
        Some(low_ticker.timestamp),
        Some(high_ticker.timestamp),
        open_conversion
            .market
            .map(shared_types::SpotTick::best_timestamp_ms),
        close_conversion
            .market
            .map(shared_types::SpotTick::best_timestamp_ms),
    ]) {
        return None;
    }
    let normalized_high_bid = high_bid * open_conversion.rate;
    let price_reference = (low_ask + normalized_high_bid) * 0.5;
    let price_gap = (normalized_high_bid - low_ask) / price_reference;
    let quote_conversions = quote_conversion::round_trip_markets(
        (open_conversion, high_quote, low_quote),
        (close_conversion, low_quote, high_quote),
    );
    let execution_blockers = if quote_conversions.is_empty() {
        Vec::new()
    } else {
        vec![quote_conversion::TICKET_BINDING_BLOCKER.to_owned()]
    };

    // 在永续合约费率为正时，做空高费率所赚资金费；做多低费率所付低成本。
    Some(RawOpportunity {
        symbol: symbol.to_owned(),
        arb_type: ArbitrageType::CrossExchange,
        long_exchange: low_data.exchange.clone(),
        short_exchange: high_data.exchange.clone(),
        long_rate: low_data.clone(),
        short_rate: high_data.clone(),
        // Legacy DTO slot. PerpCross stores the native joint-event edge here and never an 8h
        // normalization; `single_yield` is the strategy authority.
        spread_8h: native_edge,
        single_yield: native_edge,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::PerpCross),
            type_label: Some(StrategyKind::PerpCross.label_zh().to_owned()),
            description: Some(format!(
                "{} 以卖一价做多永续，{} 以买一价做空永续；原生同窗 Funding 净差 {:.4}%，开仓价差 {:+.4}%{}。",
                low_data.exchange,
                high_data.exchange,
                native_edge * 100.0,
                price_gap * 100.0,
                open_conversion.description(),
            )),
            long_action: Some(format!("{} 做多永续", low_data.exchange)),
            short_action: Some(format!("{} 做空永续", high_data.exchange)),
            long_price: Some(low_ask),
            short_price: Some(normalized_high_bid),
            long_market_symbol: Some(low_ticker.symbol.clone()),
            short_market_symbol: Some(high_ticker.symbol.clone()),
            long_depth_symbol: Some(low_ticker.symbol.clone()),
            short_depth_symbol: Some(high_ticker.symbol.clone()),
            price_deviation: Some(price_gap),
            quote_conversions,
            execution_blockers,
            ..Default::default()
        },
    })
}

fn quote_route<'a>(
    index: &MarketScanIndex<'a>,
    venues: [&str; 2],
    from_quote: &str,
    to_quote: &str,
) -> Option<(QuoteConversion<'a>, QuoteConversion<'a>)> {
    venues.into_iter().find_map(|venue| {
        let open = index.conversion_by_key(venue, from_quote, to_quote)?;
        let close = index.conversion_by_key(venue, to_quote, from_quote)?;
        Some((open, close))
    })
}

fn compare_opportunity_quality(a: &RawOpportunity, b: &RawOpportunity) -> std::cmp::Ordering {
    b.single_yield
        .partial_cmp(&a.single_yield)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            min_volume(b)
                .partial_cmp(&min_volume(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn min_volume(row: &RawOpportunity) -> f64 {
    row.long_rate.volume_24h.min(row.short_rate.volume_24h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    const HOUR_MS: i64 = 3_600_000;
    const TEST_OBSERVED_AT_MS: i64 = 7 * HOUR_MS;
    const TEST_NEXT_FUNDING_MS: i64 = 8 * HOUR_MS;

    fn fr(symbol: &str, exchange: &str, rate_8h: f64, volume: f64) -> FundingRateData {
        fr_native(symbol, exchange, rate_8h, rate_8h, 8, volume)
    }

    fn fr_native(
        symbol: &str,
        exchange: &str,
        rate: f64,
        rate_8h: f64,
        interval: u32,
        volume: f64,
    ) -> FundingRateData {
        FundingRateData {
            symbol: symbol.into(),
            exchange: exchange.into(),
            rate,
            rate_8h,
            predicted_rate: None,
            next_funding_time: TEST_NEXT_FUNDING_MS,
            funding_interval: interval,
            volume_24h: volume,
            timestamp: TEST_OBSERVED_AT_MS,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }

    fn snapshot_with(
        entries: &[(&str, &str, f64, f64)],
    ) -> HashMap<String, HashMap<String, FundingRateData>> {
        let mut m: HashMap<String, HashMap<String, FundingRateData>> = HashMap::new();
        for (sym, ex, rate, vol) in entries {
            m.entry((*sym).to_owned())
                .or_default()
                .insert((*ex).to_owned(), fr(sym, ex, *rate, *vol));
        }
        m
    }

    #[test]
    fn finds_opportunity_above_threshold() {
        let snap = snapshot_with(&[
            ("BTC", "binance", 0.0001, 1_000_000_000.0),
            ("BTC", "okx", 0.0005, 1_000_000_000.0),
            ("BTC", "bybit", 0.0003, 1_000_000_000.0),
        ]);
        let opps = scan(&snap, 0.000_1, 100_000.0);
        assert_eq!(opps.len(), 3);
        let o = &opps[0];
        assert_eq!(o.symbol, "BTC");
        assert_eq!(o.long_exchange, "binance"); // 最低费率做多
        assert_eq!(o.short_exchange, "okx"); // 最高费率做空
        assert!((o.spread_8h - 0.000_4).abs() < 1e-9);
        assert_eq!(o.extra.type_label.as_deref(), Some("永续跨所"));
        assert_eq!(o.extra.long_price, Some(100.0));
        assert_eq!(o.extra.short_price, Some(100.0));
        assert_eq!(o.extra.long_market_symbol.as_deref(), Some("BTCUSDT"));
        assert_eq!(o.extra.short_market_symbol.as_deref(), Some("BTCUSDT"));
    }

    #[test]
    fn mixed_intervals_rank_only_by_native_joint_event_yield() {
        let mut snap: HashMap<String, HashMap<String, FundingRateData>> = HashMap::new();
        snap.entry("MU".to_owned()).or_default().insert(
            "hyperliquid:xyz".to_owned(),
            fr_native("MU", "hyperliquid:xyz", 0.000_01, 0.000_08, 1, 1_000_000.0),
        );
        snap.entry("MU".to_owned()).or_default().insert(
            "okx".to_owned(),
            fr_native("MU", "okx", 0.000_16, 0.000_16, 8, 1_000_000.0),
        );

        let rows = scan(&snap, 0.000_01, 100_000.0);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].long_exchange, "hyperliquid:xyz");
        assert_eq!(rows[0].short_exchange, "okx");
        assert!((rows[0].spread_8h - 0.000_15).abs() < 1e-12);
        assert!((rows[0].single_yield - 0.000_15).abs() < 1e-12);
        assert_eq!(rows[0].extra.annualized_funding_bps, None);
        assert_eq!(rows[0].long_rate.funding_interval, 1);
    }

    #[test]
    fn rejects_profitable_rates_when_the_next_native_events_are_staggered() {
        let mut long = fr_native("BTC", "binance", 0.000_01, 0.000_08, 1, 1_000_000.0);
        let mut short = fr_native("BTC", "okx", 0.000_8, 0.000_8, 8, 1_000_000.0);
        long.next_funding_time = TEST_NEXT_FUNDING_MS;
        short.next_funding_time = TEST_NEXT_FUNDING_MS + HOUR_MS;
        let snap = HashMap::from([(
            "BTC".to_owned(),
            HashMap::from([("binance".to_owned(), long), ("okx".to_owned(), short)]),
        )]);

        let rows = scan(&snap, 0.000_01, 100_000.0);

        assert!(rows.is_empty());
    }

    #[test]
    fn filters_below_min_spread() {
        let snap = snapshot_with(&[
            ("BTC", "binance", 0.0001, 1_000_000_000.0),
            ("BTC", "okx", 0.000_15, 1_000_000_000.0),
        ]);
        let opps = scan(&snap, 0.000_1, 100_000.0);
        assert!(opps.is_empty(), "spread 0.00005 < min 0.0001 应被过滤");
    }

    #[test]
    fn filters_below_min_volume() {
        let snap = snapshot_with(&[
            ("BTC", "binance", 0.0001, 100.0), // 成交量过小
            ("BTC", "okx", 0.001, 1_000_000_000.0),
        ]);
        let opps = scan(&snap, 0.000_1, 100_000.0);
        assert!(opps.is_empty(), "binance 成交量低于阈值应被过滤");
    }

    #[test]
    fn ignores_single_exchange_symbol() {
        let snap = snapshot_with(&[("BTC", "binance", 0.0001, 1_000_000_000.0)]);
        let opps = scan(&snap, 0.000_1, 100_000.0);
        assert!(opps.is_empty());
    }

    #[test]
    fn skips_same_venue_alias_pairs() {
        let snap = snapshot_with(&[
            ("BTC", "OKX-LIVE", 0.0001, 1_000_000_000.0),
            ("BTC", "okx", 0.0010, 1_000_000_000.0),
        ]);
        let opps = scan(&snap, 0.000_1, 100_000.0);

        assert!(opps.is_empty());
    }

    #[test]
    fn sorts_by_native_joint_event_yield_descending() {
        let snap = snapshot_with(&[
            ("BTC", "a", 0.0001, 1e9),
            ("BTC", "b", 0.0002, 1e9),
            ("ETH", "a", 0.0001, 1e9),
            ("ETH", "b", 0.001, 1e9),
        ]);
        let opps = scan(&snap, 0.000_1, 100_000.0);
        assert_eq!(opps.len(), 2);
        // ETH spread 0.0009 > BTC spread 0.0001
        assert_eq!(opps[0].symbol, "ETH");
        assert_eq!(opps[1].symbol, "BTC");
    }

    #[test]
    fn quality_order_ignores_misleading_legacy_spread() {
        let mut weaker = raw_for_quality("WEAK", 0.000_1);
        let mut stronger = raw_for_quality("STRONG", 0.000_9);
        weaker.spread_8h = 99.0;
        stronger.spread_8h = -99.0;

        let mut rows = [weaker, stronger];
        rows.sort_by(compare_opportunity_quality);

        assert_eq!(rows[0].symbol, "STRONG");
    }

    #[test]
    fn exact_funding_contract_requires_its_own_ws_bbo() {
        let funding = [
            fr_native("BTCUSDT", "binance", 0.000_1, 0.000_1, 8, 1e9),
            fr_native("BTCUSDT", "okx", 0.000_5, 0.000_5, 8, 1e9),
        ];
        let tickers = [ticker("binance", "BTCUSDT", 99.0, 100.0)];
        let index = MarketScanIndex::from_slices(&[], &tickers, &funding);

        assert!(scan_indexed(&index, 0.000_1, 100_000.0).is_empty());
    }

    #[test]
    fn cross_quote_pair_uses_one_indexed_round_trip_conversion() {
        let funding = [
            fr_native("MUUSDC", "binance", 0.000_1, 0.000_1, 8, 1e9),
            fr_native("MUUSDT", "okx", 0.000_5, 0.000_5, 8, 1e9),
        ];
        let tickers = [
            ticker("binance", "MUUSDC", 99.0, 100.0),
            ticker("okx", "MUUSDT", 101.0, 102.0),
        ];
        let conversions = [stable_tick("okx", "USDT/USDC", dec!(0.999), dec!(1.001))];
        let index = MarketScanIndex::from_slices(&conversions, &tickers, &funding);

        let rows = scan_indexed(&index, 0.000_1, 100_000.0);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].extra.quote_conversions.len(), 2);
        assert_eq!(rows[0].extra.short_market_symbol.as_deref(), Some("MUUSDT"));
        assert!(rows[0]
            .extra
            .execution_blockers
            .iter()
            .any(|blocker| blocker == quote_conversion::TICKET_BINDING_BLOCKER));
    }

    #[test]
    fn keeps_secondary_low_rate_pair_for_same_symbol() {
        let snap = snapshot_with(&[
            ("MU", "hyperliquid:km", 0.000_005_707_8, 153_276.0),
            ("MU", "hyperliquid:xyz", 0.000_027_958_7, 187_034_130.0),
            ("MU", "bybit", 0.000_658_82, 5_522_527.0),
            ("MU", "kucoin", 0.000_937, 831_918.0),
        ]);
        let opps = scan(&snap, 0.000_05, 100_000.0);

        assert!(opps.iter().any(|opp| {
            opp.long_exchange == "hyperliquid:km" && opp.short_exchange == "kucoin"
        }));
        assert!(opps.iter().any(|opp| {
            opp.long_exchange == "hyperliquid:xyz" && opp.short_exchange == "kucoin"
        }));
        assert!(opps.iter().all(|opp| opp.symbol == "MU"));
    }

    #[test]
    fn keeps_builder_dex_when_more_than_four_low_rate_venues_exist() {
        let snap = snapshot_with(&[
            ("MU", "hyperliquid:km", 0.000_045_662_4, 153_276.0),
            ("MU", "bybit", 0.000_080_0, 5_522_527.0),
            ("MU", "bitget", 0.000_080_0, 4_900_000.0),
            ("MU", "gate", 0.000_112_0, 2_100_000.0),
            ("MU", "hyperliquid:xyz", 0.000_158_099_2, 181_886_100.0),
            ("MU", "okx", 0.000_584_332_0, 1_200_000.0),
            ("MU", "kucoin", 0.000_856_999_9, 831_918.0),
        ]);
        let opps = scan(&snap, 0.000_05, 100_000.0);

        assert!(
            opps.iter().any(|opp| {
                opp.long_exchange == "hyperliquid:xyz" && opp.short_exchange == "kucoin"
            }),
            "xyz MU should survive frontier generation when it is economically ranked above weaker cross pairs"
        );
    }

    #[test]
    fn exposes_the_bounded_frontier_until_market_evidence_is_attached() {
        let snap = snapshot_with(&[
            ("MU", "binance", 0.000_0, 1_000_000.0),
            ("MU", "bitget", 0.000_1, 1_000_000.0),
            ("MU", "bybit", 0.000_2, 1_000_000.0),
            ("MU", "okx", 0.000_3, 1_000_000.0),
            ("MU", "kucoin", 0.000_4, 1_000_000.0),
        ]);

        let rows = scan(&snap, 0.000_05, 100_000.0);

        assert_eq!(rows.len(), 10);
    }

    fn raw_for_quality(symbol: &str, edge: f64) -> RawOpportunity {
        RawOpportunity {
            symbol: symbol.to_owned(),
            arb_type: ArbitrageType::CrossExchange,
            long_exchange: "a".into(),
            short_exchange: "b".into(),
            long_rate: fr_native("MUUSDT", "a", 0.0, 0.0, 8, 1e9),
            short_rate: fr_native("MUUSDT", "b", 0.0, 0.0, 8, 1e9),
            spread_8h: edge,
            single_yield: edge,
            extra: Default::default(),
        }
    }

    fn ticker(exchange: &str, symbol: &str, bid: f64, ask: f64) -> TickerInfo {
        TickerInfo {
            symbol: symbol.into(),
            exchange: exchange.into(),
            bid,
            ask,
            last: (bid + ask) * 0.5,
            volume_24h: 1e9,
            timestamp: TEST_OBSERVED_AT_MS,
        }
    }

    fn stable_tick(
        venue: &str,
        symbol: &str,
        bid: rust_decimal::Decimal,
        ask: rust_decimal::Decimal,
    ) -> shared_types::SpotTick {
        shared_types::SpotTick {
            venue: venue.into(),
            symbol: symbol.into(),
            bid,
            ask,
            last: ask,
            bid_size: Some(dec!(1_000_000)),
            ask_size: Some(dec!(1_000_000)),
            volume_24h: dec!(1_000_000),
            exchange_ts_ms: Some(TEST_OBSERVED_AT_MS),
            received_at_ms: TEST_OBSERVED_AT_MS,
        }
    }
}
