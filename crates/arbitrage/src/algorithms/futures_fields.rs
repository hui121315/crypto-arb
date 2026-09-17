//! 期货套利表格扩展字段的轻量派生逻辑。

use shared_types::{ArbitrageType, FundingPrediction, StrategyKind};

pub fn classify_strategy(arb_type: ArbitrageType) -> StrategyKind {
    match arb_type {
        ArbitrageType::CrossExchange => StrategyKind::PerpCross,
        ArbitrageType::SpotFutures => StrategyKind::SpotPerp,
        ArbitrageType::CrossSpotFutures => StrategyKind::CrossSpotPerp,
        ArbitrageType::SpotCross => StrategyKind::SpotCross,
        ArbitrageType::Triangular => StrategyKind::Triangular,
        ArbitrageType::FundingCarry => StrategyKind::FundingCarry,
        ArbitrageType::OptionsPerpBasis => StrategyKind::OptionsPerpBasis,
    }
}

pub fn predict_next_funding(
    long_native_rate: f64,
    short_native_rate: f64,
    confidence: f64,
) -> FundingPrediction {
    let long_bps = rate_to_bps(long_native_rate);
    let short_bps = rate_to_bps(short_native_rate);
    FundingPrediction {
        long_bps,
        short_bps,
        net_bps: short_bps - long_bps,
        confidence: confidence.clamp(0.0, 1.0),
    }
}

pub fn borrow_cost_bps_per_day(arb_type: ArbitrageType) -> Option<f64> {
    match arb_type {
        ArbitrageType::CrossExchange | ArbitrageType::FundingCarry => Some(0.0),
        ArbitrageType::SpotFutures | ArbitrageType::CrossSpotFutures => None,
        ArbitrageType::SpotCross | ArbitrageType::Triangular | ArbitrageType::OptionsPerpBasis => {
            None
        }
    }
}

pub fn funding_window_alignment_minutes(long_next_ms: i64, short_next_ms: i64) -> Option<i32> {
    if long_next_ms <= 0 || short_next_ms <= 0 {
        return None;
    }
    let minutes = (long_next_ms - short_next_ms) / 60_000;
    Some(minutes.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

pub fn min_hold_hours(
    min_holding_periods: u32,
    settlement_interval_hours: u32,
    round_trip_cost: f64,
    net_single_yield: f64,
) -> f64 {
    let interval = settlement_interval_hours.max(1) as f64;
    let configured = min_holding_periods.max(1) as f64;
    let breakeven = if net_single_yield > 0.0 {
        ((round_trip_cost / net_single_yield) - 1e-9)
            .ceil()
            .max(1.0)
    } else {
        configured
    };
    configured.max(breakeven) * interval
}

pub fn settlement_countdown_seconds(time_to_settlement_ms: i64) -> i64 {
    time_to_settlement_ms.max(0) / 1000
}

fn rate_to_bps(rate: f64) -> f64 {
    rate * 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicts_net_funding_in_bps() {
        let p = predict_next_funding(0.0001, 0.0005, 1.2);
        assert_eq!(p.long_bps, 1.0);
        assert_eq!(p.short_bps, 5.0);
        assert_eq!(p.net_bps, 4.0);
        assert_eq!(p.confidence, 1.0);
    }

    #[test]
    fn min_hold_respects_break_even_and_config() {
        assert_eq!(min_hold_hours(1, 8, 0.0015, 0.0003), 40.0);
        assert_eq!(min_hold_hours(4, 8, 0.0001, 0.0003), 32.0);
    }

    #[test]
    fn borrow_cost_is_unknown_until_borrow_api_is_integrated() {
        assert_eq!(
            borrow_cost_bps_per_day(ArbitrageType::CrossExchange),
            Some(0.0)
        );
        assert_eq!(borrow_cost_bps_per_day(ArbitrageType::SpotFutures), None);
        assert_eq!(
            borrow_cost_bps_per_day(ArbitrageType::CrossSpotFutures),
            None
        );
    }
}
