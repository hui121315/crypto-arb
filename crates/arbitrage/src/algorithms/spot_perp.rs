//! 同所现货-永续基差扫描。

use crate::algorithms::{
    funding_timeline, market,
    market_index::{IndexedSpot, MarketScanIndex},
    quote_conversion::{self, QuoteConversion},
};
use crate::models::{RawOpportunity, RawOpportunityExtra};
use shared_types::{
    ArbitrageType, FundingRateData, SpotLegMode, SpotTick, StrategyKind, TickerInfo,
};

#[derive(Debug, Clone, Copy)]
pub struct SpotPerpConfig {
    pub min_basis_bps: f64,
    pub min_volume_24h: f64,
}

#[derive(Debug, Clone, Copy)]
struct ForwardQuote<'a> {
    basis: f64,
    spot_ask: f64,
    perp_bid: f64,
    spot_quote: &'a str,
    perp_quote: &'a str,
    open_conversion: QuoteConversion<'a>,
    close_conversion: QuoteConversion<'a>,
}

#[derive(Debug, Clone, Copy)]
struct PerpContext<'a> {
    quote: &'a str,
    ticker: &'a TickerInfo,
    funding: &'a FundingRateData,
}

impl Default for SpotPerpConfig {
    fn default() -> Self {
        Self {
            min_basis_bps: 2.0,
            min_volume_24h: 100_000.0,
        }
    }
}

pub fn scan(
    spot_ticks: &[SpotTick],
    perp_tickers: &[TickerInfo],
    funding_rates: &[FundingRateData],
    config: SpotPerpConfig,
) -> Vec<RawOpportunity> {
    let index = MarketScanIndex::from_slices(spot_ticks, perp_tickers, funding_rates);
    scan_indexed(&index, config)
}

pub(crate) fn scan_indexed(
    index: &MarketScanIndex<'_>,
    config: SpotPerpConfig,
) -> Vec<RawOpportunity> {
    let mut out = Vec::new();

    for (symbol, spots) in index.spot_groups() {
        for spot in spots {
            let Some(perps) = index.perps_by_key(&spot.venue_key, symbol) else {
                continue;
            };
            for (perp_quote, perp) in perps {
                let Some(funding) = index.funding_by_key(&spot.venue_key, symbol, perp_quote)
                else {
                    continue;
                };
                if let Some(raw) = build_opportunity(
                    spot,
                    symbol,
                    index,
                    PerpContext {
                        quote: perp_quote,
                        ticker: perp,
                        funding,
                    },
                    config,
                ) {
                    out.push(raw);
                }
            }
        }
    }

    sort_by_edge(&mut out);
    out
}

fn build_opportunity(
    spot: &IndexedSpot<'_>,
    symbol: &str,
    index: &MarketScanIndex<'_>,
    context: PerpContext<'_>,
    config: SpotPerpConfig,
) -> Option<RawOpportunity> {
    let open_conversion = index.conversion_by_key(&spot.venue_key, context.quote, &spot.quote)?;
    let close_conversion = index.conversion_by_key(&spot.venue_key, &spot.quote, context.quote)?;
    if !market::price_timestamps_are_synchronized([
        Some(spot.row.best_timestamp_ms()),
        Some(context.ticker.timestamp),
        open_conversion.market.map(SpotTick::best_timestamp_ms),
        close_conversion.market.map(SpotTick::best_timestamp_ms),
    ]) {
        return None;
    }
    if !funding_evidence_is_ready(
        spot.row,
        context.ticker,
        context.funding,
        open_conversion.market,
    ) {
        return None;
    }
    let spot_ask = spot.ask?;
    let perp_bid = market::ticker_bid(context.ticker)?;
    let normalized_perp_bid = perp_bid * open_conversion.rate;
    forward_opportunity(
        spot.row,
        symbol,
        context.ticker,
        context.funding,
        config,
        ForwardQuote {
            basis: (normalized_perp_bid - spot_ask) / spot_ask,
            spot_ask,
            perp_bid,
            spot_quote: &spot.quote,
            perp_quote: context.quote,
            open_conversion,
            close_conversion,
        },
    )
}

fn forward_opportunity(
    spot: &SpotTick,
    symbol: &str,
    perp: &TickerInfo,
    funding: &FundingRateData,
    config: SpotPerpConfig,
    quote: ForwardQuote<'_>,
) -> Option<RawOpportunity> {
    let basis_bps = quote.basis * 10_000.0;
    let volume = market::spot_volume_usd(spot)
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
    Some(RawOpportunity {
        symbol: symbol.to_owned(),
        arb_type: ArbitrageType::SpotFutures,
        long_exchange: spot.venue.clone(),
        short_exchange: perp.exchange.clone(),
        long_rate: market::zero_rate(symbol, &spot.venue, volume, spot.best_timestamp_ms()),
        short_rate: funding.clone(),
        // Legacy wire slot: carry the native next-event projection, never an 8h normalization.
        spread_8h: native_one_cycle_edge,
        single_yield: native_one_cycle_edge,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::SpotPerp),
            type_label: Some("现货-永续".into()),
            description: Some(format!(
                "{} {} 买入现货并做空同所永续；开仓基差 {:.4}%，下一原生 Funding {:.4}%{}，基差待退出时实现。",
                spot.venue,
                spot.symbol,
                basis_bps / 100.0,
                market::native_funding_rate(funding) * 100.0,
                quote.open_conversion.description()
            )),
            long_price: Some(quote.spot_ask),
            short_price: Some(quote.perp_bid),
            long_market_symbol: Some(spot.symbol.clone()),
            short_market_symbol: Some(perp.symbol.clone()),
            price_deviation: Some(quote.basis),
            basis_spread: Some(quote.basis),
            basis_bps: Some(basis_bps),
            annualized_funding_bps: Some(market::annualize_native_funding_bps(funding)),
            long_action: Some(format!("{} 买入现货", spot.venue)),
            short_action: Some(format!("{} 做空永续", perp.exchange)),
            spot_leg_mode: Some(SpotLegMode::BuySpot),
            long_depth_symbol: Some(spot.symbol.clone()),
            short_depth_symbol: Some(perp.symbol.clone()),
            quote_conversions: quote_conversion::round_trip_markets(
                (quote.open_conversion, quote.perp_quote, quote.spot_quote),
                (quote.close_conversion, quote.spot_quote, quote.perp_quote),
            ),
            execution_blockers: if quote.open_conversion.is_cross_quote()
                || quote.close_conversion.is_cross_quote()
            {
                vec![quote_conversion::TICKET_BINDING_BLOCKER.to_owned()]
            } else {
                Vec::new()
            },
            ..Default::default()
        },
    })
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

fn sort_by_edge(rows: &mut [RawOpportunity]) {
    rows.sort_by(|a, b| {
        b.single_yield
            .partial_cmp(&a.single_yield)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    const NEXT_FUNDING_MS: i64 = 28_800_001;

    fn spot() -> SpotTick {
        SpotTick {
            venue: "binance".into(),
            symbol: "BTC/USDT".into(),
            bid: dec!(99),
            ask: dec!(100),
            last: dec!(100),
            bid_size: Some(dec!(1)),
            ask_size: Some(dec!(1)),
            volume_24h: dec!(1000000),
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        }
    }

    #[test]
    fn finds_same_venue_spot_perp_basis() {
        let perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 101.0,
            ask: 101.2,
            last: 101.1,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.0001,
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let rows = scan(&[spot()], &[perp], &[funding], SpotPerpConfig::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].extra.strategy_kind, Some(StrategyKind::SpotPerp));
        assert_eq!(rows[0].extra.spot_leg_mode, Some(SpotLegMode::BuySpot));
        assert!(rows[0].single_yield > 0.01);
    }

    #[test]
    fn matches_same_venue_with_normalized_key() {
        let mut spot = spot();
        spot.venue = " Binance ".into();
        spot.symbol = "btc/usdt".into();
        let perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 101.0,
            ask: 101.2,
            last: 101.1,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "btc".into(),
            exchange: "BINANCE".into(),
            rate: 0.0001,
            rate_8h: 0.0001,
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let rows = scan(&[spot], &[perp], &[funding], SpotPerpConfig::default());

        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn skewed_spot_and_perp_quotes_do_not_form_a_basis_candidate() {
        let spot = spot();
        let mut perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 101.0,
            ask: 101.2,
            last: 101.1,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        perp.timestamp = i64::try_from(market::MAX_PRICE_TIMESTAMP_SKEW_MS).unwrap_or(i64::MAX) + 2;
        let funding = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.0001,
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };

        assert!(scan(&[spot], &[perp], &[funding], SpotPerpConfig::default()).is_empty());
    }

    #[test]
    fn cross_quote_requires_an_executable_fx_market() {
        let mut spot = spot();
        spot.symbol = "mu/usdt".into();
        let perp = TickerInfo {
            symbol: "MU-USDC".into(),
            exchange: "binance".into(),
            bid: 101.0,
            ask: 101.2,
            last: 101.1,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "MU-USDC".into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.0001,
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let rows = scan(&[spot], &[perp], &[funding], SpotPerpConfig::default());

        assert!(rows.is_empty());
    }

    #[test]
    fn supports_same_and_cross_stablecoin_quotes() {
        let mut spot_usdt = spot();
        spot_usdt.symbol = "MU/USDT".into();
        let mut spot_usdc = spot();
        spot_usdc.symbol = "MU/USDC".into();
        spot_usdc.ask = dec!(100);
        spot_usdc.bid = dec!(99);
        spot_usdc.last = dec!(100);
        let fx = SpotTick {
            venue: "binance".into(),
            symbol: "USDC/USDT".into(),
            bid: dec!(0.999),
            ask: dec!(1.001),
            last: dec!(1),
            bid_size: Some(dec!(1000000)),
            ask_size: Some(dec!(1000000)),
            volume_24h: dec!(10000000),
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        };
        let perp_usdt = TickerInfo {
            symbol: "MUUSDTM".into(),
            exchange: "binance".into(),
            bid: 101.0,
            ask: 101.2,
            last: 101.1,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        let perp_usdc = TickerInfo {
            symbol: "MU-USDC".into(),
            exchange: "binance".into(),
            bid: 101.0,
            ask: 101.2,
            last: 101.1,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        let funding = |symbol: &str| FundingRateData {
            symbol: symbol.into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.8,
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };

        let rows = scan(
            &[spot_usdt, spot_usdc, fx],
            &[perp_usdt, perp_usdc],
            &[funding("MUUSDTM"), funding("MU-USDC")],
            SpotPerpConfig::default(),
        );

        assert_eq!(rows.len(), 4);
        assert!(rows.iter().any(|row| {
            row.extra.long_market_symbol.as_deref() == Some("MU/USDT")
                && row.extra.short_market_symbol.as_deref() == Some("MUUSDTM")
        }));
        assert!(rows.iter().any(|row| {
            row.extra.long_market_symbol.as_deref() == Some("MU/USDC")
                && row.extra.short_market_symbol.as_deref() == Some("MU-USDC")
        }));
        assert_eq!(
            rows.iter()
                .filter(|row| {
                    row.extra.execution_blockers == [quote_conversion::TICKET_BINDING_BLOCKER]
                })
                .count(),
            2
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.extra.quote_conversions.len() == 2)
                .count(),
            2
        );
    }

    /// 回归：`funding=0` 但 basis 大时，`annualized_funding_bps` 必须为 0；
    /// 否则前端会把基差按 funding 周期错误地年化（NFLX 在 Bitget/Gate
    /// funding=0 但 basis 0.46% 显示年化 ~500% 的 bug）。
    #[test]
    fn zero_funding_with_large_basis_does_not_annualize_basis() {
        // 模仿 NFLX 同所现货-永续：基差 ~0.46%，资金费 0%
        let perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 100.46,
            ask: 100.50,
            last: 100.48,
            volume_24h: 5_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: 0.0,
            rate_8h: 0.0, // 与用户报告一致：0 资金费
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 5_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let rows = scan(&[spot()], &[perp], &[funding], SpotPerpConfig::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].extra.strategy_kind, Some(StrategyKind::SpotPerp));
        let annualized = rows[0]
            .extra
            .annualized_funding_bps
            .expect("annualized_funding_bps must be set");
        // funding=0 → 年化 funding 必须为 0，不应被基差污染。
        assert!(
            annualized.abs() < 1e-6,
            "expected 0 annualized funding bps when funding rate=0, got {annualized}"
        );
        // basis 仍然记录在 basis_bps 字段（应该是 ~46 bps）
        let basis = rows[0].extra.basis_bps.unwrap_or(0.0);
        assert!(basis > 40.0 && basis < 50.0, "basis_bps = {basis}");
    }

    #[test]
    fn forward_negative_funding_reduces_edge() {
        let perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 100.05,
            ask: 100.10,
            last: 100.08,
            volume_24h: 5_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: -0.001,
            rate_8h: -0.001,
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 5_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };

        let rows = scan(&[spot()], &[perp], &[funding], SpotPerpConfig::default());

        assert!(rows.is_empty());
    }

    /// 回归：现货-永续只使用交易所原生 rate/interval，不能读取 `rate_8h`。
    #[test]
    fn one_hour_funding_does_not_overannualize() {
        let perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 100.10,
            ask: 100.20,
            last: 100.15,
            volume_24h: 5_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: 0.000_01,    // 1h 单期 0.001%
            rate_8h: 0.000_08, // adapter 标准化：0.000_01 * 8
            predicted_rate: None,
            next_funding_time: 3_600_001,
            funding_interval: 1, // 模拟 1h funding 合约
            volume_24h: 5_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let rows = scan(&[spot()], &[perp], &[funding], SpotPerpConfig::default());
        assert_eq!(rows.len(), 1);
        let annualized = rows[0]
            .extra
            .annualized_funding_bps
            .expect("annualized_funding_bps must be set");
        // 0.000_01 * 24 * 365 * 10_000 = 876
        assert!(
            (annualized - 876.0).abs() < 1e-3,
            "expected 876 bps for native 1h funding, got {annualized}"
        );
        let basis = (100.10 - 100.0) / 100.0;
        assert!((rows[0].spread_8h - (basis + 0.000_01)).abs() < 1e-12);
        assert!((rows[0].single_yield - (basis + 0.000_01)).abs() < 1e-12);
    }

    #[test]
    fn reverse_basis_is_not_emitted() {
        let perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 97.0,
            ask: 98.0,
            last: 97.5,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: -0.0001,
            rate_8h: -0.0001,
            predicted_rate: None,
            next_funding_time: NEXT_FUNDING_MS,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let rows = scan(&[spot()], &[perp], &[funding], SpotPerpConfig::default());
        assert!(rows.is_empty());
    }

    #[test]
    fn missing_native_funding_schedule_is_not_emitted() {
        let perp = TickerInfo {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            bid: 101.0,
            ask: 101.2,
            last: 101.1,
            volume_24h: 2_000_000.0,
            timestamp: 1,
        };
        let funding = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.5,
            predicted_rate: None,
            next_funding_time: 0,
            funding_interval: 8,
            volume_24h: 2_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let mut outside_native_window = funding.clone();
        outside_native_window.next_funding_time = 9 * 3_600_000;

        assert!(scan(
            &[spot()],
            std::slice::from_ref(&perp),
            &[funding],
            SpotPerpConfig::default()
        )
        .is_empty());
        assert!(scan(
            &[spot()],
            &[perp],
            &[outside_native_window],
            SpotPerpConfig::default()
        )
        .is_empty());
    }
}
