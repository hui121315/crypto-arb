//! 期权时间价值与永续资金费相对价值扫描。

use crate::algorithms::market;
use crate::models::{RawOpportunity, RawOpportunityExtra};
use shared_types::{
    ArbitrageType, FundingRateData, OptionMarketQuote, OptionType, StrategyKind, TickerInfo,
};
use std::collections::HashMap;

const MS_PER_DAY: f64 = 86_400_000.0;
const PERIODS_PER_YEAR_8H: f64 = 3.0 * 365.0;
const MIN_DELTA: f64 = 0.05;

#[derive(Debug, Clone, Copy)]
pub struct OptionsPerpBasisConfig {
    pub min_spread_bps: f64,
    pub min_volume_24h: f64,
    pub now_ms: i64,
}

impl Default for OptionsPerpBasisConfig {
    fn default() -> Self {
        Self {
            min_spread_bps: 25.0,
            min_volume_24h: 10_000.0,
            now_ms: 0,
        }
    }
}

pub fn scan(
    quotes: &[OptionMarketQuote],
    perp_tickers: &[TickerInfo],
    funding_rates: &[FundingRateData],
    config: OptionsPerpBasisConfig,
) -> Vec<RawOpportunity> {
    let tickers = ticker_map(perp_tickers);
    let funding = funding_map(funding_rates);
    let mut out = Vec::new();

    for quote in quotes {
        let key = market::venue_symbol_key(&quote.venue, &quote.underlying);
        let Some(perp) = tickers.get(&key) else {
            continue;
        };
        let Some(rate) = funding.get(&key) else {
            continue;
        };
        if let Some(row) = build_opportunity(quote, perp, rate, config) {
            out.push(row);
        }
    }

    out.sort_by(|a, b| {
        b.single_yield
            .partial_cmp(&a.single_yield)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn build_opportunity(
    quote: &OptionMarketQuote,
    perp: &TickerInfo,
    rate: &FundingRateData,
    config: OptionsPerpBasisConfig,
) -> Option<RawOpportunity> {
    let spot = market::positive(perp.last)?;
    let mark = option_mark(quote)?;
    let days = days_to_expiry(quote, config.now_ms)?;
    let time_value = (mark - intrinsic_value(quote, spot)).max(0.0);
    let delta = quote.delta.abs();
    if delta < MIN_DELTA {
        return None;
    }
    let option_carry_bps = time_value / (spot * delta) / days * 365.0 * 10_000.0;
    let funding_bps = market::annualize_rate_8h_bps(rate.rate_8h);
    let spread_bps = option_carry_bps - funding_bps;
    let volume = quote.volume_24h.min(perp.volume_24h).min(rate.volume_24h);

    if spread_bps.abs() < config.min_spread_bps || volume < config.min_volume_24h {
        return None;
    }

    let edge = annualized_bps_to_8h_period(spread_bps.abs());
    let (long_exchange, short_exchange, long_action, short_action) =
        leg_labels(quote, rate, spread_bps);

    Some(RawOpportunity {
        symbol: quote.underlying.clone(),
        arb_type: ArbitrageType::OptionsPerpBasis,
        long_exchange,
        short_exchange,
        long_rate: market::zero_rate(&quote.underlying, &quote.venue, volume, quote.timestamp_ms),
        short_rate: rate.clone(),
        spread_8h: edge,
        single_yield: edge,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::OptionsPerpBasis),
            type_label: Some("期权-永续基差".into()),
            description: Some(format!(
                "{} 时间价值年化 {:.3}%，永续资金费年化 {:.3}%。",
                quote.symbol,
                option_carry_bps / 100.0,
                funding_bps / 100.0
            )),
            long_action: Some(long_action),
            short_action: Some(short_action),
            long_price: Some(mark),
            short_price: Some(spot),
            price_deviation: Some(spread_bps / 10_000.0),
            basis_spread: Some(edge),
            basis_bps: Some(spread_bps.abs()),
            // `annualized_funding_bps` 字段统一表示永续 funding leg 的年化（bps），
            // 与 `spread_bps`（策略 edge = 期权 carry - funding）语义不同。
            // 使用 funding_bps 的有符号值，让前端能正确叠加 basis 而不会双重计算。
            annualized_funding_bps: Some(funding_bps),
            ..Default::default()
        },
    })
}

fn option_mark(quote: &OptionMarketQuote) -> Option<f64> {
    market::positive(quote.mark).or_else(|| market::positive((quote.bid + quote.ask) * 0.5))
}

fn intrinsic_value(quote: &OptionMarketQuote, spot: f64) -> f64 {
    match quote.option_type {
        OptionType::Call => (spot - quote.strike).max(0.0),
        OptionType::Put => (quote.strike - spot).max(0.0),
    }
}

fn days_to_expiry(quote: &OptionMarketQuote, now_ms: i64) -> Option<f64> {
    let now = if now_ms > 0 {
        now_ms
    } else {
        quote.timestamp_ms
    };
    let days = (quote.expiry_ms - now) as f64 / MS_PER_DAY;
    days.is_finite().then_some(days).filter(|v| *v > 0.0)
}

fn annualized_bps_to_8h_period(annualized_bps: f64) -> f64 {
    annualized_bps / 10_000.0 / PERIODS_PER_YEAR_8H
}

fn leg_labels(
    quote: &OptionMarketQuote,
    rate: &FundingRateData,
    spread_bps: f64,
) -> (String, String, String, String) {
    if spread_bps >= 0.0 {
        (
            rate.exchange.clone(),
            quote.venue.clone(),
            format!("{} 持有永续对冲腿", rate.exchange),
            format!("{} 卖出偏贵期权时间价值", quote.venue),
        )
    } else {
        (
            quote.venue.clone(),
            rate.exchange.clone(),
            format!("{} 买入偏便宜期权时间价值", quote.venue),
            format!("{} 做空永续资金费腿", rate.exchange),
        )
    }
}

fn ticker_map(tickers: &[TickerInfo]) -> HashMap<(String, String), &TickerInfo> {
    tickers
        .iter()
        .map(|item| (market::venue_symbol_key(&item.exchange, &item.symbol), item))
        .collect()
}

fn funding_map(rates: &[FundingRateData]) -> HashMap<(String, String), &FundingRateData> {
    rates
        .iter()
        .map(|item| (market::venue_symbol_key(&item.exchange, &item.symbol), item))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_option_time_value_vs_funding_spread() {
        let rows = scan(&[quote()], &[ticker()], &[funding()], Default::default());
        assert_eq!(rows.len(), 1);
        let kind = rows[0].extra.strategy_kind;
        assert_eq!(kind, Some(StrategyKind::OptionsPerpBasis));
    }

    /// 回归：`annualized_funding_bps` 字段必须装永续 funding leg 的真实年化（`funding_bps`），
    /// 而不是策略 edge `spread_bps = option_carry_bps - funding_bps`（修复前 bug：
    /// 把两者都装进同一字段，前端会双重计算 funding）。
    #[test]
    fn annualized_funding_bps_holds_funding_leg_not_strategy_edge() {
        let rows = scan(&[quote()], &[ticker()], &[funding()], Default::default());
        assert_eq!(rows.len(), 1);
        let extra = &rows[0].extra;
        let annualized_funding = extra
            .annualized_funding_bps
            .expect("annualized_funding_bps must be set");
        let basis = extra.basis_bps.expect("basis_bps must be set");

        // funding leg 年化 = rate_8h * 3 * 365 * 10_000 = 0.000_1 * 3 * 365 * 10_000 = 1_095 bps
        assert!(
            (annualized_funding - 1_095.0).abs() < 1e-3,
            "annualized_funding_bps should be funding leg annualized (1_095), got {annualized_funding}"
        );
        // basis_bps = |option_carry - funding| 是策略 edge，理应 ≠ funding leg 年化
        assert!(
            (basis - annualized_funding).abs() > 1.0,
            "basis_bps={basis} and annualized_funding_bps={annualized_funding} must NOT be the same value (they have different semantics)"
        );
    }

    #[test]
    fn skips_quotes_without_usable_delta() {
        let mut low_delta = quote();
        low_delta.delta = 0.01;

        let rows = scan(&[low_delta], &[ticker()], &[funding()], Default::default());

        assert!(rows.is_empty());
    }

    #[test]
    fn matches_perp_leg_with_normalized_venue_key() {
        let mut quote = quote();
        quote.venue = " Deribit ".into();
        quote.underlying = "btc".into();
        let rows = scan(&[quote], &[ticker()], &[funding()], Default::default());

        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn interval_does_not_change_edge_from_standardized_rate() {
        let rows_8h = scan(&[quote()], &[ticker()], &[funding()], Default::default());
        let mut one_hour = funding();
        one_hour.funding_interval = 1;
        let rows_1h = scan(&[quote()], &[ticker()], &[one_hour], Default::default());

        assert_eq!(rows_8h.len(), 1);
        assert_eq!(rows_1h.len(), 1);
        assert!((rows_8h[0].single_yield - rows_1h[0].single_yield).abs() < 1e-12);
    }

    fn quote() -> OptionMarketQuote {
        OptionMarketQuote {
            venue: "deribit".into(),
            symbol: "BTC-30MAY26-100000-C".into(),
            underlying: "BTC".into(),
            option_type: OptionType::Call,
            strike: 100_000.0,
            expiry_ms: MS_PER_DAY as i64 * 30,
            bid: 1_900.0,
            ask: 2_100.0,
            mark: 2_000.0,
            volume_24h: 100_000.0,
            timestamp_ms: 0,
            delta: 0.5,
        }
    }

    fn ticker() -> TickerInfo {
        TickerInfo {
            symbol: "BTC".into(),
            exchange: "deribit".into(),
            bid: 100_000.0,
            ask: 100_050.0,
            last: 100_000.0,
            volume_24h: 500_000.0,
            timestamp: 0,
        }
    }

    fn funding() -> FundingRateData {
        let mut rate = market::zero_rate("BTC", "deribit", 500_000.0, 0);
        rate.rate = 0.0001;
        rate.rate_8h = 0.0001;
        rate
    }
}
