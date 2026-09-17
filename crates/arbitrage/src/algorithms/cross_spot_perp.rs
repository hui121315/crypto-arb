//! Cross-venue forward cash-and-carry: buy Spot on one venue and short Perp on another.

use crate::algorithms::{
    fee_evidence, funding_timeline, market,
    market_index::{IndexedSpot, MarketScanIndex},
    quote_conversion::{self, QuoteConversion},
};
use crate::models::{RawOpportunity, RawOpportunityExtra};
use shared_types::{
    ArbitrageType, FeeProduct, FundingRateData, SpotLegMode, SpotTick, StrategyKind, TickerInfo,
};

const MAX_PAIRS_PER_SYMBOL: usize = 16;

#[derive(Debug, Clone, Copy)]
pub struct CrossSpotPerpConfig {
    pub min_basis_bps: f64,
    pub min_volume_24h: f64,
    pub min_net_yield: f64,
    pub default_slippage: f64,
}

#[derive(Debug)]
struct ForwardQuote<'a> {
    basis: f64,
    spot_ask: f64,
    perp_bid: f64,
    open_conversion: QuoteConversion<'a>,
    close_conversion: QuoteConversion<'a>,
    quote_conversions: Vec<shared_types::OpportunityQuoteConversion>,
}

#[derive(Debug, Clone, Copy)]
struct PerpContext<'a, 'i> {
    perp: &'a TickerInfo,
    funding: &'a FundingRateData,
    venue_key: &'i str,
    quote: &'i str,
}

#[derive(Debug)]
struct ForwardCandidate<'a> {
    spot: &'a SpotTick,
    perp: &'a TickerInfo,
    funding: &'a FundingRateData,
    quote: ForwardQuote<'a>,
    volume: f64,
    native_one_cycle_edge: f64,
    verified_net_bps: Option<f64>,
}

impl Default for CrossSpotPerpConfig {
    fn default() -> Self {
        Self {
            min_basis_bps: 4.0,
            min_volume_24h: 100_000.0,
            min_net_yield: 0.0,
            default_slippage: 0.0,
        }
    }
}

pub fn scan(
    spot_ticks: &[SpotTick],
    perp_tickers: &[TickerInfo],
    funding_rates: &[FundingRateData],
    config: CrossSpotPerpConfig,
) -> Vec<RawOpportunity> {
    let index = MarketScanIndex::from_slices(spot_ticks, perp_tickers, funding_rates);
    scan_indexed(&index, config)
}

pub(crate) fn scan_indexed(
    index: &MarketScanIndex<'_>,
    config: CrossSpotPerpConfig,
) -> Vec<RawOpportunity> {
    let mut out = Vec::new();

    for (symbol, perps) in index.perp_groups() {
        let spots = index.spots(symbol);
        if spots.is_empty() {
            continue;
        }
        let mut candidates = Vec::new();
        for perp in perps {
            let Some(funding) = index.funding_by_key(&perp.venue_key, symbol, &perp.quote) else {
                continue;
            };
            append_symbol_candidates(
                &mut candidates,
                spots,
                PerpContext {
                    perp: perp.row,
                    funding,
                    venue_key: &perp.venue_key,
                    quote: &perp.quote,
                },
                index,
                config,
            );
        }
        retain_verified_frontier(&mut candidates);
        out.extend(
            candidates
                .into_iter()
                .map(|candidate| candidate.into_raw(symbol)),
        );
    }

    out.sort_by(|left, right| right.single_yield.total_cmp(&left.single_yield));
    out
}

fn retain_verified_frontier(candidates: &mut Vec<ForwardCandidate<'_>>) {
    candidates.sort_by(|left, right| {
        right
            .verified_net_bps
            .is_some()
            .cmp(&left.verified_net_bps.is_some())
            .then_with(|| {
                right
                    .verified_net_bps
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&left.verified_net_bps.unwrap_or(f64::NEG_INFINITY))
            })
            .then_with(|| {
                right
                    .native_one_cycle_edge
                    .total_cmp(&left.native_one_cycle_edge)
            })
            .then_with(|| left.spot.venue.cmp(&right.spot.venue))
            .then_with(|| left.perp.exchange.cmp(&right.perp.exchange))
    });
    candidates.truncate(MAX_PAIRS_PER_SYMBOL);
}

fn verified_net_bps(
    spot_venue: &str,
    perp_venue: &str,
    gross_yield: f64,
    config: CrossSpotPerpConfig,
) -> Option<f64> {
    let spot_fee = fee_evidence::standard_taker_fee_bps(spot_venue, FeeProduct::Spot)?;
    let perp_fee = fee_evidence::standard_taker_fee_bps(perp_venue, FeeProduct::Perp)?;
    let slippage_bps = config.default_slippage.max(0.0) * 10_000.0;
    let round_trip_cost_bps = (spot_fee + perp_fee) * 2.0 + slippage_bps * 4.0;
    Some(gross_yield * 10_000.0 - round_trip_cost_bps)
}

fn append_symbol_candidates<'a>(
    out: &mut Vec<ForwardCandidate<'a>>,
    spots: &[IndexedSpot<'a>],
    context: PerpContext<'a, '_>,
    index: &MarketScanIndex<'a>,
    config: CrossSpotPerpConfig,
) {
    for spot in spots {
        if spot.venue_key == context.venue_key {
            continue;
        }
        if let Some(candidate) = prepare_candidate(spot, context, index, config) {
            out.push(candidate);
        }
    }
}

fn prepare_candidate<'a>(
    spot: &IndexedSpot<'a>,
    context: PerpContext<'a, '_>,
    index: &MarketScanIndex<'a>,
    config: CrossSpotPerpConfig,
) -> Option<ForwardCandidate<'a>> {
    let open_conversion = index.conversion_by_key(context.venue_key, context.quote, &spot.quote)?;
    let close_conversion =
        index.conversion_by_key(context.venue_key, &spot.quote, context.quote)?;
    if !market::price_timestamps_are_synchronized([
        Some(spot.row.best_timestamp_ms()),
        Some(context.perp.timestamp),
        open_conversion.market.map(SpotTick::best_timestamp_ms),
        close_conversion.market.map(SpotTick::best_timestamp_ms),
    ]) {
        return None;
    }
    if !funding_evidence_is_ready(
        spot.row,
        context.perp,
        context.funding,
        open_conversion.market,
    ) {
        return None;
    }
    let spot_ask = spot.ask?;
    let perp_bid = market::ticker_bid(context.perp)?;
    let normalized_perp_bid = perp_bid * open_conversion.rate;
    prepare_forward_candidate(
        spot.row,
        spot.volume_24h,
        context.perp,
        context.funding,
        config,
        ForwardQuote {
            basis: (normalized_perp_bid - spot_ask) / spot_ask,
            spot_ask,
            perp_bid,
            open_conversion,
            close_conversion,
            quote_conversions: quote_conversion::round_trip_markets(
                (open_conversion, context.quote, &spot.quote),
                (close_conversion, &spot.quote, context.quote),
            ),
        },
    )
}

fn prepare_forward_candidate<'a>(
    spot: &'a SpotTick,
    spot_volume_24h: f64,
    perp: &'a TickerInfo,
    funding: &'a FundingRateData,
    config: CrossSpotPerpConfig,
    quote: ForwardQuote<'a>,
) -> Option<ForwardCandidate<'a>> {
    let basis_bps = quote.basis * 10_000.0;
    let volume = spot_volume_24h
        .min(perp.volume_24h)
        .min(funding.volume_24h)
        .min(
            quote
                .open_conversion
                .market
                .map_or(f64::MAX, market::spot_volume_usd),
        )
        .min(
            quote
                .close_conversion
                .market
                .map_or(f64::MAX, market::spot_volume_usd),
        );
    if basis_bps < config.min_basis_bps || volume < config.min_volume_24h {
        return None;
    }

    let native_one_cycle_edge = quote.basis + market::native_funding_rate(funding);
    if native_one_cycle_edge * 10_000.0 < config.min_basis_bps {
        return None;
    }

    let verified_net_bps =
        verified_net_bps(&spot.venue, &perp.exchange, native_one_cycle_edge, config);
    if verified_net_bps.is_some_and(|net_bps| net_bps <= config.min_net_yield.max(0.0) * 10_000.0) {
        return None;
    }

    Some(ForwardCandidate {
        spot,
        perp,
        funding,
        quote,
        volume,
        native_one_cycle_edge,
        verified_net_bps,
    })
}

impl ForwardCandidate<'_> {
    fn into_raw(self, symbol: &str) -> RawOpportunity {
        let basis_bps = self.quote.basis * 10_000.0;

        RawOpportunity {
            symbol: symbol.to_owned(),
            arb_type: ArbitrageType::CrossSpotFutures,
            long_exchange: self.spot.venue.clone(),
            short_exchange: self.perp.exchange.clone(),
            long_rate: market::zero_rate(
                symbol,
                &self.spot.venue,
                self.volume,
                self.spot.best_timestamp_ms(),
            ),
            short_rate: self.funding.clone(),
            // Compatibility slot: this is the native next-event projection, not an 8h normalization.
            spread_8h: self.native_one_cycle_edge,
            single_yield: self.native_one_cycle_edge,
            extra: RawOpportunityExtra {
                strategy_kind: Some(StrategyKind::CrossSpotPerp),
                type_label: Some("跨所期现".into()),
                description: Some(format!(
                    "{} 买 {} 现货，{} 做空永续；开仓基差 {:.4}%，下一原生 Funding {:.4}%{}，基差待退出时实现。",
                    self.spot.venue,
                    self.spot.symbol,
                    self.perp.exchange,
                    basis_bps / 100.0,
                    market::native_funding_rate(self.funding) * 100.0,
                    self.quote.open_conversion.description()
                )),
                long_action: Some(format!("{} 买入现货", self.spot.venue)),
                short_action: Some(format!("{} 做空永续", self.perp.exchange)),
                spot_leg_mode: Some(SpotLegMode::BuySpot),
                long_price: Some(self.quote.spot_ask),
                short_price: Some(self.quote.perp_bid),
                long_market_symbol: Some(self.spot.symbol.clone()),
                short_market_symbol: Some(self.perp.symbol.clone()),
                price_deviation: Some(self.quote.basis),
                basis_spread: Some(self.quote.basis),
                basis_bps: Some(basis_bps),
                annualized_funding_bps: Some(market::annualize_native_funding_bps(self.funding)),
                long_depth_symbol: Some(self.spot.symbol.clone()),
                short_depth_symbol: Some(self.perp.symbol.clone()),
                quote_conversions: self.quote.quote_conversions,
                execution_blockers: if self.quote.open_conversion.is_cross_quote()
                    || self.quote.close_conversion.is_cross_quote()
                {
                    vec![quote_conversion::TICKET_BINDING_BLOCKER.to_owned()]
                } else {
                    Vec::new()
                },
                ..Default::default()
            },
        }
    }
}

fn funding_evidence_is_ready(
    spot: &SpotTick,
    perp: &TickerInfo,
    funding: &FundingRateData,
    quote_market: Option<&SpotTick>,
) -> bool {
    let observed_at_ms = spot
        .best_timestamp_ms()
        .max(perp.timestamp)
        .max(funding.timestamp)
        .max(quote_market.map_or(0, SpotTick::best_timestamp_ms));
    funding.rate.is_finite()
        && funding.funding_interval > 0
        && funding.timestamp > 0
        && funding_timeline::native_next_settlement_is_plausible(
            funding.next_funding_time,
            funding.funding_interval,
            observed_at_ms,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    const NEXT_FUNDING_MS: i64 = 28_800_001;

    #[test]
    fn forward_basis_uses_buy_cost_and_native_funding_once() {
        let rows = scan(
            &[spot("okx", "BTC/USDT", dec!(99), dec!(100))],
            &[perp("binance", "BTCUSDT", 101.0, 101.2)],
            &[funding(
                "binance",
                "BTCUSDT",
                0.000_1,
                0.9,
                8,
                NEXT_FUNDING_MS,
            )],
            CrossSpotPerpConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert!((rows[0].single_yield - 0.010_1).abs() < 1e-12);
        assert_eq!(rows[0].spread_8h, rows[0].single_yield);
        assert_eq!(rows[0].extra.spot_leg_mode, Some(SpotLegMode::BuySpot));
        assert_eq!(
            rows[0].extra.long_market_symbol.as_deref(),
            Some("BTC/USDT")
        );
        assert_eq!(
            rows[0].extra.short_market_symbol.as_deref(),
            Some("BTCUSDT")
        );
    }

    #[test]
    fn one_hour_contract_does_not_read_the_normalized_rate() {
        let rows = scan(
            &[spot("okx", "BTC/USDT", dec!(99), dec!(100))],
            &[perp("binance", "BTCUSDT", 101.0, 101.2)],
            &[funding(
                "binance", "BTCUSDT", 0.000_01, 0.000_08, 1, 3_600_001,
            )],
            CrossSpotPerpConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert!((rows[0].single_yield - 0.010_01).abs() < 1e-12);
        assert!(rows[0]
            .extra
            .annualized_funding_bps
            .is_some_and(|value| (value - 876.0).abs() < 1e-9));
    }

    #[test]
    fn skewed_cross_venue_quotes_do_not_form_a_basis_candidate() {
        let mut perp = perp("binance", "BTCUSDT", 101.0, 101.2);
        perp.timestamp = i64::try_from(market::MAX_PRICE_TIMESTAMP_SKEW_MS).unwrap_or(i64::MAX) + 2;

        let rows = scan(
            &[spot("okx", "BTC/USDT", dec!(99), dec!(100))],
            &[perp],
            &[funding(
                "binance",
                "BTCUSDT",
                0.000_1,
                0.000_1,
                8,
                NEXT_FUNDING_MS,
            )],
            CrossSpotPerpConfig::default(),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn verified_frontier_bounds_dense_cross_venue_candidates() {
        let venues = ["binance", "bitget", "bybit", "gate", "kucoin", "okx"];
        let spots = venues
            .iter()
            .map(|venue| spot(venue, "BTC/USDT", dec!(99), dec!(100)))
            .collect::<Vec<_>>();
        let perps = venues
            .iter()
            .enumerate()
            .map(|(index, venue)| {
                let bid = 101.0 + index as f64 * 0.1;
                perp(venue, "BTCUSDT", bid, bid + 0.1)
            })
            .collect::<Vec<_>>();
        let funding_rates = venues
            .iter()
            .map(|venue| funding(venue, "BTCUSDT", 0.000_1, 0.000_1, 8, NEXT_FUNDING_MS))
            .collect::<Vec<_>>();

        let rows = scan(
            &spots,
            &perps,
            &funding_rates,
            CrossSpotPerpConfig::default(),
        );

        assert_eq!(rows.len(), MAX_PAIRS_PER_SYMBOL);
        assert!(rows.iter().all(|row| {
            verified_net_bps(
                &row.long_exchange,
                &row.short_exchange,
                row.single_yield,
                CrossSpotPerpConfig::default(),
            )
            .is_some_and(|net_bps| net_bps > 0.0)
        }));
    }

    #[test]
    fn exact_perp_quote_selects_the_matching_funding_contract() {
        let rows = scan(
            &[spot("okx", "MU/USDC", dec!(99), dec!(100))],
            &[perp("binance", "MU-USDC", 101.0, 101.2)],
            &[
                funding("binance", "MUUSDT", -0.5, -0.5, 8, NEXT_FUNDING_MS),
                funding("binance", "MU-USDC", 0.000_2, 0.8, 8, NEXT_FUNDING_MS),
            ],
            CrossSpotPerpConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].short_rate.symbol, "MU-USDC");
        assert!((rows[0].single_yield - 0.010_2).abs() < 1e-12);
    }

    #[test]
    fn cross_quote_requires_an_executable_conversion_and_stays_observation_only() {
        let spot_leg = spot("okx", "MU/USDT", dec!(99), dec!(100));
        let perp_leg = perp("binance", "MU-USDC", 101.0, 101.2);
        let funding_leg = funding("binance", "MU-USDC", 0.000_1, 0.000_1, 8, NEXT_FUNDING_MS);

        assert!(scan(
            std::slice::from_ref(&spot_leg),
            std::slice::from_ref(&perp_leg),
            std::slice::from_ref(&funding_leg),
            CrossSpotPerpConfig::default(),
        )
        .is_empty());

        let fx = spot("binance", "USDC/USDT", dec!(0.999), dec!(1.001));
        let rows = scan(
            &[spot_leg, fx],
            &[perp_leg],
            &[funding_leg],
            CrossSpotPerpConfig::default(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].extra.execution_blockers,
            [quote_conversion::TICKET_BINDING_BLOCKER]
        );
        assert_eq!(rows[0].extra.quote_conversions.len(), 2);
        assert!(rows[0]
            .extra
            .quote_conversions
            .iter()
            .all(|conversion| conversion.symbol == "USDC/USDT"));
    }

    #[test]
    fn reverse_basis_and_missing_funding_schedule_are_not_emitted() {
        let reverse = scan(
            &[spot("okx", "BTC/USDT", dec!(101), dec!(101.2))],
            &[perp("binance", "BTCUSDT", 98.8, 99.0)],
            &[funding(
                "binance",
                "BTCUSDT",
                -0.000_1,
                -0.000_1,
                8,
                NEXT_FUNDING_MS,
            )],
            CrossSpotPerpConfig::default(),
        );
        let missing_schedule = scan(
            &[spot("okx", "BTC/USDT", dec!(99), dec!(100))],
            &[perp("binance", "BTCUSDT", 101.0, 101.2)],
            &[funding("binance", "BTCUSDT", 0.000_1, 1.0, 8, 0)],
            CrossSpotPerpConfig::default(),
        );

        assert!(reverse.is_empty());
        assert!(missing_schedule.is_empty());
    }

    fn spot(
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
            volume_24h: dec!(2000000),
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        }
    }

    fn perp(exchange: &str, symbol: &str, bid: f64, ask: f64) -> TickerInfo {
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

    fn funding(
        exchange: &str,
        symbol: &str,
        rate: f64,
        rate_8h: f64,
        interval: u32,
        next_funding_time: i64,
    ) -> FundingRateData {
        FundingRateData {
            symbol: symbol.into(),
            exchange: exchange.into(),
            rate,
            rate_8h,
            predicted_rate: None,
            next_funding_time,
            funding_interval: interval,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }
}
