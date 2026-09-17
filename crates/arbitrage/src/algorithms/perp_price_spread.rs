//! Cross-venue perpetual price convergence scanner.

use crate::algorithms::{
    funding_timeline,
    market_index::{IndexedPerp, MarketScanIndex},
    quote_conversion,
};
use crate::models::{RawOpportunity, RawOpportunityExtra};
use shared_types::{ArbitrageType, FundingRateData, SpotTick, StrategyKind, TickerInfo};

const EDGE_VENUES_PER_SIDE: usize = 8;
const MAX_PAIRS_PER_SYMBOL: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct PerpPriceSpreadConfig {
    pub min_spread_bps: f64,
    pub min_volume_24h: f64,
}

impl Default for PerpPriceSpreadConfig {
    fn default() -> Self {
        Self {
            min_spread_bps: 5.0,
            min_volume_24h: 100_000.0,
        }
    }
}

pub fn scan(
    tickers: &[TickerInfo],
    funding: &[FundingRateData],
    spot_ticks: &[SpotTick],
    config: PerpPriceSpreadConfig,
) -> Vec<RawOpportunity> {
    let index = MarketScanIndex::from_slices(spot_ticks, tickers, funding);
    scan_indexed(&index, config)
}

pub(crate) fn scan_indexed(
    index: &MarketScanIndex<'_>,
    config: PerpPriceSpreadConfig,
) -> Vec<RawOpportunity> {
    let mut out = Vec::new();
    for (symbol, tickers) in index.perp_groups() {
        let rows = tickers
            .iter()
            .filter(|ticker| {
                ticker.row.volume_24h.is_finite() && ticker.row.volume_24h >= config.min_volume_24h
            })
            .collect();
        append_symbol_pairs(symbol, rows, index, config, &mut out);
    }
    out.sort_by(compare_edge);
    out
}

fn append_symbol_pairs(
    symbol: &str,
    rows: Vec<&IndexedPerp<'_>>,
    index: &MarketScanIndex<'_>,
    config: PerpPriceSpreadConfig,
    out: &mut Vec<RawOpportunity>,
) {
    if rows.len() < 2 {
        return;
    }
    let mut buys = rows.clone();
    buys.sort_by(|left, right| left.row.ask.total_cmp(&right.row.ask));
    let mut sells = rows;
    sells.sort_by(|left, right| right.row.bid.total_cmp(&left.row.bid));
    let mut pairs = Vec::new();

    for buy in buys.iter().take(EDGE_VENUES_PER_SIDE) {
        for sell in sells.iter().take(EDGE_VENUES_PER_SIDE) {
            if buy.venue_key == sell.venue_key {
                continue;
            }
            if let Some(pair) = raw_pair(symbol, buy, sell, index, config) {
                pairs.push(pair);
            }
        }
    }
    pairs.sort_by(compare_edge);
    pairs.truncate(MAX_PAIRS_PER_SYMBOL);
    out.extend(pairs);
}

fn raw_pair(
    symbol: &str,
    buy: &IndexedPerp<'_>,
    sell: &IndexedPerp<'_>,
    index: &MarketScanIndex<'_>,
    config: PerpPriceSpreadConfig,
) -> Option<RawOpportunity> {
    let (Some(buy_ask), Some(sell_bid)) = (buy.ask, sell.bid) else {
        return None;
    };
    let open_conversion = index.conversion_by_key(&sell.venue_key, &sell.quote, &buy.quote)?;
    let close_conversion = index.conversion_by_key(&sell.venue_key, &buy.quote, &sell.quote)?;
    if !crate::algorithms::market::price_timestamps_are_synchronized([
        Some(buy.row.timestamp),
        Some(sell.row.timestamp),
        open_conversion.market.map(SpotTick::best_timestamp_ms),
        close_conversion.market.map(SpotTick::best_timestamp_ms),
    ]) {
        return None;
    }
    let normalized_sell_bid = sell_bid * open_conversion.rate;
    let edge = (normalized_sell_bid - buy_ask) / buy_ask;
    let volume = buy.row.volume_24h.min(sell.row.volume_24h);
    if edge * 10_000.0 < config.min_spread_bps || volume < config.min_volume_24h {
        return None;
    }
    let observed_at_ms = buy.row.timestamp.max(sell.row.timestamp);
    let long = index
        .funding_by_key(&buy.venue_key, symbol, &buy.quote)
        .filter(|row| funding_evidence_ready(row, observed_at_ms))?;
    let short = index
        .funding_by_key(&sell.venue_key, symbol, &sell.quote)
        .filter(|row| funding_evidence_ready(row, observed_at_ms))?;
    let mut blockers = Vec::new();
    if open_conversion.is_cross_quote() || close_conversion.is_cross_quote() {
        blockers.push(quote_conversion::TICKET_BINDING_BLOCKER.to_owned());
    }
    let long_rate = long.clone();
    let short_rate = short.clone();
    let edge_bps = edge * 10_000.0;
    let funding_description = native_funding_description(long, short);

    Some(RawOpportunity {
        symbol: symbol.to_owned(),
        arb_type: ArbitrageType::CrossExchange,
        long_exchange: buy.row.exchange.clone(),
        short_exchange: sell.row.exchange.clone(),
        long_rate,
        short_rate,
        spread_8h: edge,
        single_yield: edge,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::PerpPriceSpread),
            type_label: Some("永续价差".into()),
            description: Some(format!(
                "{} 以卖一价做多、{} 以买一价做空；换算后价差 {:.4}%{}{}。历史收敛仅供观察，平仓后才确认盈亏。",
                buy.row.exchange,
                sell.row.exchange,
                edge_bps / 100.0,
                open_conversion.description(),
                funding_description,
            )),
            long_action: Some(format!("{} 做多永续", buy.row.exchange)),
            short_action: Some(format!("{} 做空永续", sell.row.exchange)),
            long_price: Some(buy_ask),
            short_price: Some(normalized_sell_bid),
            long_market_symbol: Some(buy.row.symbol.clone()),
            short_market_symbol: Some(sell.row.symbol.clone()),
            long_depth_symbol: Some(buy.row.symbol.clone()),
            short_depth_symbol: Some(sell.row.symbol.clone()),
            price_deviation: Some(edge),
            basis_spread: Some(edge),
            basis_bps: Some(edge_bps),
            annualized_funding_bps: None,
            quote_conversions: quote_conversion::round_trip_markets(
                (open_conversion, &sell.quote, &buy.quote),
                (close_conversion, &buy.quote, &sell.quote),
            ),
            execution_blockers: blockers,
            ..Default::default()
        },
    })
}

fn funding_evidence_ready(row: &FundingRateData, observed_at_ms: i64) -> bool {
    row.timestamp > 0
        && row.funding_interval > 0
        && row.rate.is_finite()
        && funding_timeline::native_next_settlement_is_plausible(
            row.next_funding_time,
            row.funding_interval,
            observed_at_ms.max(row.timestamp),
        )
}

fn native_funding_description(long: &FundingRateData, short: &FundingRateData) -> String {
    format!(
        "；当前原生资金费多腿 {:+.4}%/{}h、空腿 {:+.4}%/{}h，不换算成 8h",
        long.rate * 100.0,
        long.funding_interval,
        short.rate * 100.0,
        short.funding_interval,
    )
}

fn compare_edge(left: &RawOpportunity, right: &RawOpportunity) -> std::cmp::Ordering {
    right
        .single_yield
        .total_cmp(&left.single_yield)
        .then_with(|| {
            right
                .long_rate
                .volume_24h
                .min(right.short_rate.volume_24h)
                .total_cmp(&left.long_rate.volume_24h.min(left.short_rate.volume_24h))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn emits_executable_bid_ask_spread_with_funding_evidence() {
        let tickers = [
            ticker("binance", "BTCUSDT", 99.0, 100.0),
            ticker("okx", "BTC-USDT", 101.0, 102.0),
        ];
        let funding = [rate("binance", "BTC"), rate("okx", "BTC")];

        let rows = scan(&tickers, &funding, &[], PerpPriceSpreadConfig::default());

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].extra.strategy_kind,
            Some(StrategyKind::PerpPriceSpread)
        );
        assert_eq!(rows[0].extra.long_price, Some(100.0));
        assert_eq!(rows[0].extra.short_price, Some(101.0));
        assert_eq!(rows[0].extra.long_depth_symbol.as_deref(), Some("BTCUSDT"));
        assert_eq!(
            rows[0].extra.short_depth_symbol.as_deref(),
            Some("BTC-USDT")
        );
        assert_eq!(rows[0].extra.annualized_funding_bps, None);
        assert!(rows[0].extra.execution_blockers.is_empty());
    }

    #[test]
    fn rejects_same_venue_and_cross_quote_without_conversion() {
        let rows = scan(
            &[
                ticker("okx", "BTC-USDT", 99.0, 100.0),
                ticker("OKX-LIVE", "BTC-USDT", 102.0, 103.0),
                ticker("binance", "BTCUSDC", 102.0, 103.0),
            ],
            &[],
            &[],
            PerpPriceSpreadConfig::default(),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn normalized_base_symbols_reach_instrument_registry_gating() {
        let rows = scan(
            &[
                ticker("binance", "BTC", 99.0, 100.0),
                ticker("okx", "BTC", 101.0, 102.0),
            ],
            &[rate("binance", "BTC"), rate("okx", "BTC")],
            &[],
            PerpPriceSpreadConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BTC");
        assert!(rows[0].extra.execution_blockers.is_empty());
    }

    #[test]
    fn missing_funding_does_not_enter_the_opportunity_list() {
        let rows = scan(
            &[
                ticker("binance", "BTCUSDT", 99.0, 100.0),
                ticker("okx", "BTC-USDT", 101.0, 102.0),
            ],
            &[],
            &[],
            PerpPriceSpreadConfig::default(),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn invalid_funding_schedule_does_not_enter_the_opportunity_list() {
        let mut invalid = rate("binance", "BTC");
        invalid.funding_interval = 0;
        let rows = scan(
            &[
                ticker("binance", "BTCUSDT", 99.0, 100.0),
                ticker("okx", "BTC-USDT", 101.0, 102.0),
            ],
            &[invalid, rate("okx", "BTC")],
            &[],
            PerpPriceSpreadConfig::default(),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn skewed_quotes_do_not_form_a_convergence_candidate() {
        let mut stale = ticker("binance", "BTCUSDT", 99.0, 100.0);
        stale.timestamp = 1;
        let mut fresh = ticker("okx", "BTC-USDT", 101.0, 102.0);
        fresh.timestamp = i64::try_from(crate::algorithms::market::MAX_PRICE_TIMESTAMP_SKEW_MS)
            .unwrap_or(i64::MAX)
            + 2;
        let rows = scan(
            &[stale, fresh],
            &[rate("binance", "BTC"), rate("okx", "BTC")],
            &[],
            PerpPriceSpreadConfig::default(),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn exact_quote_selects_the_matching_funding_contract() {
        let mut wrong_quote = rate("binance", "BTCUSDC");
        wrong_quote.rate = 0.50;
        wrong_quote.rate_8h = 0.50;
        let rows = scan(
            &[
                ticker("binance", "BTCUSDT", 99.0, 100.0),
                ticker("okx", "BTC-USDT", 101.0, 102.0),
            ],
            &[
                wrong_quote,
                rate("binance", "BTCUSDT"),
                rate("okx", "BTC-USDT"),
            ],
            &[],
            PerpPriceSpreadConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].long_rate.symbol, "BTCUSDT");
        assert_eq!(rows[0].short_rate.symbol, "BTC-USDT");
    }

    #[test]
    fn cross_quote_uses_both_executable_fx_sides_and_stays_blocked() {
        let rows = scan(
            &[
                ticker("hyperliquid", "BTC", 99.0, 100.0),
                ticker("binance", "BTCUSDT", 102.0, 103.0),
            ],
            &[rate("hyperliquid", "BTC"), rate("binance", "BTCUSDT")],
            &[spot("binance", "USDT/USDC", 0.999, 1.001)],
            PerpPriceSpreadConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert!(rows[0]
            .extra
            .short_price
            .is_some_and(|price| (price - 102.0 * 0.999).abs() < 1e-9));
        assert!(rows[0]
            .extra
            .execution_blockers
            .iter()
            .any(|blocker| blocker == quote_conversion::TICKET_BINDING_BLOCKER));
        assert_eq!(rows[0].extra.quote_conversions.len(), 2);
        assert!(rows[0]
            .extra
            .quote_conversions
            .iter()
            .all(|conversion| conversion.symbol == "USDT/USDC"));
    }

    #[test]
    fn native_funding_is_not_replaced_by_the_eight_hour_field() {
        let mut one_hour = rate("binance", "BTCUSDT");
        one_hour.rate = 0.000_01;
        one_hour.rate_8h = 0.50;
        one_hour.funding_interval = 1;
        one_hour.next_funding_time = 3_600_001;
        let rows = scan(
            &[
                ticker("binance", "BTCUSDT", 99.0, 100.0),
                ticker("okx", "BTC-USDT", 101.0, 102.0),
            ],
            &[one_hour, rate("okx", "BTC-USDT")],
            &[],
            PerpPriceSpreadConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].long_rate.rate, 0.000_01);
        assert_eq!(rows[0].extra.annualized_funding_bps, None);
        assert!(rows[0]
            .extra
            .description
            .as_deref()
            .is_some_and(|description| description.contains("不换算成 8h")));
    }

    fn ticker(exchange: &str, symbol: &str, bid: f64, ask: f64) -> TickerInfo {
        TickerInfo {
            symbol: symbol.into(),
            exchange: exchange.into(),
            bid,
            ask,
            last: (bid + ask) * 0.5,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        }
    }

    fn rate(exchange: &str, symbol: &str) -> FundingRateData {
        FundingRateData {
            symbol: symbol.into(),
            exchange: exchange.into(),
            rate: 0.0001,
            rate_8h: 0.0001,
            predicted_rate: None,
            next_funding_time: 8 * 3_600_000,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }

    fn spot(venue: &str, symbol: &str, bid: f64, ask: f64) -> SpotTick {
        SpotTick {
            venue: venue.into(),
            symbol: symbol.into(),
            bid: rust_decimal::Decimal::from_f64_retain(bid).unwrap_or(dec!(0)),
            ask: rust_decimal::Decimal::from_f64_retain(ask).unwrap_or(dec!(0)),
            last: rust_decimal::Decimal::from_f64_retain(ask).unwrap_or(dec!(0)),
            bid_size: Some(dec!(1000000)),
            ask_size: Some(dec!(1000000)),
            volume_24h: dec!(1000000),
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        }
    }
}
