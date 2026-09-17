//! Executable stable-quote conversion evidence shared by basis strategies.

use crate::algorithms::market;
use shared_types::{OpportunityQuoteConversion, SpotTick};
use std::collections::HashMap;

pub(crate) const TICKET_BINDING_BLOCKER: &str =
    "跨计价币套利需要票据绑定的换汇、结算币风险与退出成本下限，仅观察不执行";

#[derive(Debug, Clone, Copy)]
pub(crate) struct QuoteConversion<'a> {
    pub(crate) rate: f64,
    pub(crate) market: Option<&'a SpotTick>,
}

#[derive(Debug, Default)]
struct VenueConversions<'a> {
    usdt_to_usdc: Option<QuoteConversion<'a>>,
    usdc_to_usdt: Option<QuoteConversion<'a>>,
}

#[derive(Debug, Default)]
pub(crate) struct QuoteConversionIndex<'a> {
    by_venue: HashMap<String, VenueConversions<'a>>,
}

impl<'a> QuoteConversionIndex<'a> {
    #[cfg(test)]
    pub(crate) fn new(spot_ticks: &'a [SpotTick]) -> Self {
        let mut index = Self::default();
        for tick in spot_ticks {
            index.insert_tick(tick);
        }
        index
    }

    pub(crate) fn find_on_venue(
        &self,
        venue: &str,
        from_quote: &str,
        to_quote: &str,
    ) -> Option<QuoteConversion<'a>> {
        if from_quote == to_quote {
            return Some(QuoteConversion {
                rate: 1.0,
                market: None,
            });
        }
        let routes = self.by_venue.get(&market::venue_identity_key(venue))?;
        match (from_quote, to_quote) {
            ("USDT", "USDC") => routes.usdt_to_usdc,
            ("USDC", "USDT") => routes.usdc_to_usdt,
            _ => None,
        }
    }

    #[cfg(test)]
    fn insert_tick(&mut self, tick: &'a SpotTick) {
        let base = market::canonical_base_symbol(&tick.symbol);
        let Some(quote) = market::canonical_quote_symbol(&tick.symbol) else {
            return;
        };
        let venue_key = market::venue_identity_key(&tick.venue);
        self.insert_normalized_tick(&venue_key, &base, &quote, tick);
    }

    pub(crate) fn insert_normalized_tick(
        &mut self,
        venue_key: &str,
        base: &str,
        quote: &str,
        tick: &'a SpotTick,
    ) {
        if !is_supported_stable_pair(base, quote) {
            return;
        }
        let routes = self.by_venue.entry(venue_key.to_owned()).or_default();
        match (base, quote) {
            ("USDT", "USDC") => {
                insert_latest(&mut routes.usdt_to_usdc, market::spot_bid(tick), tick);
                insert_latest(
                    &mut routes.usdc_to_usdt,
                    market::spot_price(tick).map(|ask| 1.0 / ask),
                    tick,
                );
            }
            ("USDC", "USDT") => {
                insert_latest(&mut routes.usdc_to_usdt, market::spot_bid(tick), tick);
                insert_latest(
                    &mut routes.usdt_to_usdc,
                    market::spot_price(tick).map(|ask| 1.0 / ask),
                    tick,
                );
            }
            _ => {}
        }
    }
}

fn insert_latest<'a>(
    slot: &mut Option<QuoteConversion<'a>>,
    rate: Option<f64>,
    tick: &'a SpotTick,
) {
    let Some(rate) = rate.filter(|rate| rate.is_finite() && *rate > 0.0) else {
        return;
    };
    if slot
        .and_then(|current| current.market)
        .is_some_and(|current| current.best_timestamp_ms() > tick.best_timestamp_ms())
    {
        return;
    }
    *slot = Some(QuoteConversion {
        rate,
        market: Some(tick),
    });
}

impl QuoteConversion<'_> {
    pub(crate) fn is_cross_quote(self) -> bool {
        self.market.is_some()
    }

    pub(crate) fn description(self) -> String {
        self.market.map_or_else(String::new, |market| {
            format!(
                "；按 {} 可执行汇率 {:.6} 归一计价",
                market.symbol, self.rate
            )
        })
    }
}

pub(crate) fn required_market(
    conversion: QuoteConversion<'_>,
    from_quote: &str,
    to_quote: &str,
) -> Option<OpportunityQuoteConversion> {
    let market = conversion.market?;
    Some(OpportunityQuoteConversion {
        from_quote: from_quote.to_owned(),
        to_quote: to_quote.to_owned(),
        rate: conversion.rate,
        venue: market.venue.clone(),
        symbol: market.symbol.clone(),
        market_evidence: None,
    })
}

pub(crate) fn round_trip_markets(
    open: (QuoteConversion<'_>, &str, &str),
    close: (QuoteConversion<'_>, &str, &str),
) -> Vec<OpportunityQuoteConversion> {
    [
        required_market(open.0, open.1, open.2),
        required_market(close.0, close.1, close.2),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(crate) fn find_on_venue<'a>(
    venue: &str,
    from_quote: &str,
    to_quote: &str,
    spot_ticks: &'a [SpotTick],
) -> Option<QuoteConversion<'a>> {
    if from_quote == to_quote {
        return Some(QuoteConversion {
            rate: 1.0,
            market: None,
        });
    }
    if !is_supported_stable_pair(from_quote, to_quote) {
        return None;
    }
    spot_ticks
        .iter()
        .filter(|tick| market::same_venue(venue, &tick.venue))
        .filter_map(|tick| conversion_from_tick(tick, from_quote, to_quote))
        .max_by_key(|(_, tick)| tick.best_timestamp_ms())
        .map(|(rate, market)| QuoteConversion {
            rate,
            market: Some(market),
        })
}

fn conversion_from_tick<'a>(
    tick: &'a SpotTick,
    from_quote: &str,
    to_quote: &str,
) -> Option<(f64, &'a SpotTick)> {
    let base = market::canonical_base_symbol(&tick.symbol);
    let quote = market::canonical_quote_symbol(&tick.symbol)?;
    let rate = if base == from_quote && quote == to_quote {
        market::spot_bid(tick)?
    } else if base == to_quote && quote == from_quote {
        1.0 / market::spot_price(tick)?
    } else {
        return None;
    };
    rate.is_finite().then_some((rate, tick))
}

fn is_supported_stable_pair(left: &str, right: &str) -> bool {
    matches!((left, right), ("USDT", "USDC") | ("USDC", "USDT"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn uses_the_executable_side_of_the_conversion_book() {
        let direct = tick("USDC/USDT", dec!(0.999), dec!(1.001));
        let inverse = tick("USDT/USDC", dec!(0.998), dec!(1.002));

        let direct_rate =
            find_on_venue("binance", "USDC", "USDT", &[direct]).map(|value| value.rate);
        let inverse_rate =
            find_on_venue("binance", "USDC", "USDT", &[inverse]).map(|value| value.rate);

        assert_eq!(direct_rate, Some(0.999));
        assert!(inverse_rate.is_some_and(|value| (value - 1.0 / 1.002).abs() < 1e-12));
    }

    #[test]
    fn indexed_lookup_builds_both_executable_directions_once() {
        let ticks = [tick("USDT/USDC", dec!(0.998), dec!(1.002))];
        let index = QuoteConversionIndex::new(&ticks);

        let sell_usdt = index
            .find_on_venue("binance", "USDT", "USDC")
            .map(|value| value.rate);
        let buy_usdt = index
            .find_on_venue("binance", "USDC", "USDT")
            .map(|value| value.rate);

        assert_eq!(sell_usdt, Some(0.998));
        assert!(buy_usdt.is_some_and(|value| (value - 1.0 / 1.002).abs() < 1e-12));
    }

    fn tick(symbol: &str, bid: rust_decimal::Decimal, ask: rust_decimal::Decimal) -> SpotTick {
        SpotTick {
            venue: "binance".into(),
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
