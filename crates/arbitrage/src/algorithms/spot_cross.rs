//! Prefunded cross-venue spot spread scanner.

use crate::algorithms::{
    market,
    market_index::{IndexedSpot, MarketScanIndex},
    quote_conversion,
};
use crate::models::{RawOpportunity, RawOpportunityExtra};
use shared_types::{ArbitrageType, SpotLegMode, SpotTick, StrategyKind};

const EDGE_VENUES_PER_SIDE: usize = 8;
const MAX_PAIRS_PER_SYMBOL: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct SpotCrossConfig {
    pub min_spread_bps: f64,
    pub min_volume_24h: f64,
}

impl Default for SpotCrossConfig {
    fn default() -> Self {
        Self {
            min_spread_bps: 5.0,
            min_volume_24h: 100_000.0,
        }
    }
}

pub fn scan(ticks: &[SpotTick], config: SpotCrossConfig) -> Vec<RawOpportunity> {
    let index = MarketScanIndex::from_slices(ticks, &[], &[]);
    scan_indexed(&index, config)
}

pub(crate) fn scan_indexed(
    index: &MarketScanIndex<'_>,
    config: SpotCrossConfig,
) -> Vec<RawOpportunity> {
    let mut out = Vec::new();
    for (symbol, ticks) in index.spot_groups() {
        let rows = ticks
            .iter()
            .filter(|tick| tick.volume_24h >= config.min_volume_24h)
            .collect();
        append_symbol_pairs(symbol, rows, index, config, &mut out);
    }
    out.sort_by(compare_edge);
    out
}

fn append_symbol_pairs(
    symbol: &str,
    rows: Vec<&IndexedSpot<'_>>,
    index: &MarketScanIndex<'_>,
    config: SpotCrossConfig,
    out: &mut Vec<RawOpportunity>,
) {
    if rows.len() < 2 {
        return;
    }
    let mut buys = rows.clone();
    buys.sort_by(|left, right| {
        left.ask
            .unwrap_or(f64::INFINITY)
            .total_cmp(&right.ask.unwrap_or(f64::INFINITY))
    });
    let mut sells = rows;
    sells.sort_by(|left, right| {
        right
            .bid
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&left.bid.unwrap_or(f64::NEG_INFINITY))
    });
    let mut pairs = Vec::new();

    for buy in buys.iter().take(EDGE_VENUES_PER_SIDE) {
        for sell in sells.iter().take(EDGE_VENUES_PER_SIDE) {
            if buy.venue_key == sell.venue_key {
                continue;
            }
            let (Some(buy_ask), Some(sell_bid)) = (buy.ask, sell.bid) else {
                continue;
            };
            let Some(open_conversion) =
                index.conversion_by_key(&sell.venue_key, &sell.quote, &buy.quote)
            else {
                continue;
            };
            let Some(close_conversion) =
                index.conversion_by_key(&sell.venue_key, &buy.quote, &sell.quote)
            else {
                continue;
            };
            if !market::price_timestamps_are_synchronized([
                Some(buy.row.best_timestamp_ms()),
                Some(sell.row.best_timestamp_ms()),
                open_conversion.market.map(SpotTick::best_timestamp_ms),
                close_conversion.market.map(SpotTick::best_timestamp_ms),
            ]) {
                continue;
            }
            let normalized_sell_bid = sell_bid * open_conversion.rate;
            let edge = (normalized_sell_bid - buy_ask) / buy_ask;
            let volume = buy.volume_24h.min(sell.volume_24h);
            if edge * 10_000.0 < config.min_spread_bps || volume < config.min_volume_24h {
                continue;
            }
            pairs.push(raw_pair(
                symbol,
                ExecutableSpotPair {
                    buy: buy.row,
                    sell: sell.row,
                    buy_ask,
                    sell_bid: normalized_sell_bid,
                    edge,
                    volume,
                    conversion_description: open_conversion.description(),
                    quote_conversions: quote_conversion::round_trip_markets(
                        (open_conversion, &sell.quote, &buy.quote),
                        (close_conversion, &buy.quote, &sell.quote),
                    ),
                },
            ));
        }
    }
    pairs.sort_by(compare_edge);
    pairs.truncate(MAX_PAIRS_PER_SYMBOL);
    out.extend(pairs);
}

struct ExecutableSpotPair<'a> {
    buy: &'a SpotTick,
    sell: &'a SpotTick,
    buy_ask: f64,
    sell_bid: f64,
    edge: f64,
    volume: f64,
    conversion_description: String,
    quote_conversions: Vec<shared_types::OpportunityQuoteConversion>,
}

fn raw_pair(symbol: &str, pair: ExecutableSpotPair<'_>) -> RawOpportunity {
    let ExecutableSpotPair {
        buy,
        sell,
        buy_ask,
        sell_bid,
        edge,
        volume,
        conversion_description,
        quote_conversions,
    } = pair;
    let edge_bps = edge * 10_000.0;
    RawOpportunity {
        symbol: symbol.to_owned(),
        arb_type: ArbitrageType::SpotCross,
        long_exchange: buy.venue.clone(),
        short_exchange: sell.venue.clone(),
        long_rate: market::zero_rate(symbol, &buy.venue, volume, buy.best_timestamp_ms()),
        short_rate: market::zero_rate(symbol, &sell.venue, volume, sell.best_timestamp_ms()),
        spread_8h: edge,
        single_yield: edge,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::SpotCross),
            type_label: Some("现货跨所".into()),
            description: Some(format!(
                "{} 以卖一价买入、{} 以买一价卖出预置库存；换算后毛价差 {:.4}%{}，按完整库存循环成本验收。",
                buy.venue,
                sell.venue,
                edge_bps / 100.0,
                conversion_description,
            )),
            long_action: Some(format!("{} 买入现货", buy.venue)),
            short_action: Some(format!("{} 卖出现货库存", sell.venue)),
            spot_leg_mode: Some(SpotLegMode::SellInventory),
            long_price: Some(buy_ask),
            short_price: Some(sell_bid),
            long_market_symbol: Some(buy.symbol.clone()),
            short_market_symbol: Some(sell.symbol.clone()),
            price_deviation: Some(edge),
            basis_spread: Some(edge),
            basis_bps: Some(edge_bps),
            annualized_funding_bps: None,
            long_depth_symbol: Some(buy.symbol.clone()),
            short_depth_symbol: Some(sell.symbol.clone()),
            execution_blockers: if quote_conversions.is_empty() {
                Vec::new()
            } else {
                vec![quote_conversion::TICKET_BINDING_BLOCKER.to_owned()]
            },
            quote_conversions,
            ..Default::default()
        },
    }
}

fn compare_edge(left: &RawOpportunity, right: &RawOpportunity) -> std::cmp::Ordering {
    right.single_yield.total_cmp(&left.single_yield)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn emits_prefunded_same_quote_spot_spread() {
        let rows = scan(
            &[
                tick("okx", "BTC/USDT", dec!(99), dec!(100)),
                tick("binance", "BTCUSDT", dec!(102), dec!(103)),
            ],
            SpotCrossConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].extra.strategy_kind, Some(StrategyKind::SpotCross));
        assert_eq!(
            rows[0].extra.spot_leg_mode,
            Some(SpotLegMode::SellInventory)
        );
        assert_eq!(rows[0].long_exchange, "okx");
        assert_eq!(rows[0].short_exchange, "binance");
        assert_eq!(
            rows[0].extra.long_market_symbol.as_deref(),
            Some("BTC/USDT")
        );
        assert_eq!(
            rows[0].extra.short_market_symbol.as_deref(),
            Some("BTCUSDT")
        );
        assert!(rows[0].extra.execution_blockers.is_empty());
    }

    #[test]
    fn rejects_cross_quote_without_fx_book_and_same_venue_aliases() {
        let rows = scan(
            &[
                tick("OKX-LIVE", "BTC/USDT", dec!(99), dec!(100)),
                tick("okx", "BTC/USDT", dec!(102), dec!(103)),
                tick("binance", "BTC/USDC", dec!(102), dec!(103)),
            ],
            SpotCrossConfig::default(),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn cross_quote_uses_executable_fx_and_stays_observation_only() {
        let rows = scan(
            &[
                tick("okx", "BTC/USDC", dec!(99), dec!(100)),
                tick("binance", "BTC/USDT", dec!(102), dec!(103)),
                tick("binance", "USDT/USDC", dec!(0.999), dec!(1.001)),
            ],
            SpotCrossConfig::default(),
        );

        let row = rows
            .iter()
            .find(|row| row.symbol == "BTC")
            .expect("cross-quote BTC row");
        assert!(row
            .extra
            .short_price
            .is_some_and(|price| (price - 102.0 * 0.999).abs() < 1e-9));
        assert!(row
            .extra
            .execution_blockers
            .iter()
            .any(|blocker| blocker == quote_conversion::TICKET_BINDING_BLOCKER));
        assert_eq!(row.extra.quote_conversions.len(), 2);
        assert!(row
            .extra
            .quote_conversions
            .iter()
            .all(|conversion| conversion.symbol == "USDT/USDC"));
    }

    #[test]
    fn skewed_spot_quotes_do_not_form_a_cross_venue_candidate() {
        let buy = tick("okx", "BTC/USDT", dec!(99), dec!(100));
        let mut sell = tick("binance", "BTCUSDT", dec!(102), dec!(103));
        let stale_boundary =
            i64::try_from(market::MAX_PRICE_TIMESTAMP_SKEW_MS).unwrap_or(i64::MAX) + 2;
        sell.exchange_ts_ms = Some(stale_boundary);
        sell.received_at_ms = stale_boundary;

        assert!(scan(&[buy, sell], SpotCrossConfig::default()).is_empty());
    }

    fn tick(
        venue: &str,
        symbol: &str,
        bid: rust_decimal::Decimal,
        ask: rust_decimal::Decimal,
    ) -> SpotTick {
        SpotTick {
            venue: venue.into(),
            symbol: symbol.into(),
            bid,
            ask,
            last: ask,
            bid_size: Some(dec!(1)),
            ask_size: Some(dec!(1)),
            volume_24h: dec!(1000000),
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        }
    }
}
