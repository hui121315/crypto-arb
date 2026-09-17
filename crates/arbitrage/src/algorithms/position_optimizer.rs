//! 候选阶段的仓位预算。
//!
//! 这里不预测胜率，也不把单次收益年化。实际可执行数量由后续构建阶段结合余额、
//! instrument spec、双边深度和完整成本重新计算。

use crate::models::PositionSizing;

#[derive(Debug, Clone)]
pub struct PositionConstraints {
    pub max_position_ratio: f64,
    pub max_exchange_ratio: f64,
    pub max_symbol_ratio: f64,
    pub allocation_scaling: f64,
}

impl Default for PositionConstraints {
    fn default() -> Self {
        Self {
            max_position_ratio: 0.1,
            max_exchange_ratio: 0.3,
            max_symbol_ratio: 0.2,
            allocation_scaling: 0.5,
        }
    }
}

/// 计算候选阶段的计划预算（USD）。
pub fn calculate_position(
    verified_net_return: f64,
    total_capital: f64,
    risk_tolerance: f64,
    constraints: &PositionConstraints,
) -> PositionSizing {
    if !verified_net_return.is_finite()
        || verified_net_return <= 0.0
        || !total_capital.is_finite()
        || total_capital <= 0.0
    {
        return PositionSizing::default();
    }

    let max_position = total_capital * effective_cap_ratio(constraints);
    let allocation = max_position
        * constraints.allocation_scaling.clamp(0.0, 1.0)
        * risk_tolerance.clamp(0.0, 1.0);

    PositionSizing {
        // 旧字段保留线协议兼容；没有成交分布证据就不能声称 Kelly 已被证明。
        kelly_fraction: 0.0,
        optimal_position: allocation,
        max_position,
        risk_adjusted_position: allocation,
    }
}

fn effective_cap_ratio(constraints: &PositionConstraints) -> f64 {
    constraints
        .max_position_ratio
        .min(constraints.max_exchange_ratio)
        .min(constraints.max_symbol_ratio)
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn position_zero_without_positive_finite_verified_net() {
        let constraints = PositionConstraints::default();
        let negative = calculate_position(-0.000_2, 100_000.0, 1.0, &constraints);
        let infinite = calculate_position(f64::INFINITY, 100_000.0, 1.0, &constraints);

        assert_eq!(negative.optimal_position, 0.0);
        assert_eq!(infinite.optimal_position, 0.0);
    }

    #[test]
    fn position_zero_capital_returns_zero() {
        let p = calculate_position(0.000_2, 0.0, 1.0, &PositionConstraints::default());
        assert_eq!(p.optimal_position, 0.0);
    }

    #[test]
    fn position_capped_by_max_per_opportunity() {
        let constraints = PositionConstraints {
            max_position_ratio: 0.1,
            allocation_scaling: 1.0,
            ..Default::default()
        };
        let p = calculate_position(0.000_2, 1_000_000.0, 1.0, &constraints);

        assert!((p.optimal_position - 100_000.0).abs() < 1e-6);
        assert!((p.max_position - 100_000.0).abs() < 1e-6);
    }

    #[test]
    fn position_capped_by_exchange_ratio() {
        let constraints = PositionConstraints {
            max_position_ratio: 0.5,
            max_exchange_ratio: 0.08,
            max_symbol_ratio: 0.5,
            allocation_scaling: 1.0,
        };
        let p = calculate_position(0.000_2, 1_000_000.0, 1.0, &constraints);

        assert!((p.max_position - 80_000.0).abs() < 1e-6);
        assert!((p.optimal_position - 80_000.0).abs() < 1e-6);
    }

    #[test]
    fn position_capped_by_symbol_ratio() {
        let constraints = PositionConstraints {
            max_position_ratio: 0.5,
            max_exchange_ratio: 0.5,
            max_symbol_ratio: 0.06,
            allocation_scaling: 1.0,
        };
        let p = calculate_position(0.000_2, 1_000_000.0, 1.0, &constraints);

        assert!((p.max_position - 60_000.0).abs() < 1e-6);
        assert!((p.optimal_position - 60_000.0).abs() < 1e-6);
    }

    #[test]
    fn allocation_and_risk_tolerance_scale_the_budget() {
        let constraints = PositionConstraints::default();
        let full = calculate_position(0.000_2, 100_000.0, 1.0, &constraints);
        let half = calculate_position(0.000_2, 100_000.0, 0.5, &constraints);

        assert!((full.max_position - 10_000.0).abs() < 1e-6);
        assert!((full.optimal_position - 5_000.0).abs() < 1e-6);
        assert!((half.optimal_position - 2_500.0).abs() < 1e-6);
        assert_eq!(half.risk_adjusted_position, half.optimal_position);
        assert_eq!(half.kelly_fraction, 0.0);
    }
}
