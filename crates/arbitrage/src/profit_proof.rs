//! Strategy-specific conservative profit proof for execution tickets.

use crate::algorithms::{funding_timeline, perp_cross_policy};
use shared_types::{SpotLegMode, StrategyKind};

/// Strength of the strategy-level profit evidence attached to a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfitProofClass {
    Locked,
    Statistical,
    Projected,
    Unproven,
}

impl ProfitProofClass {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Statistical => "statistical",
            Self::Projected => "projected",
            Self::Unproven => "unproven",
        }
    }
}

/// Native funding inputs for one bilateral settlement window.
#[derive(Debug, Clone, Copy)]
pub struct FundingWindowInput {
    pub long_funding_bps: Option<f64>,
    pub short_funding_bps: Option<f64>,
    pub long_next_settlement_ms: i64,
    pub short_next_settlement_ms: i64,
    pub long_interval_hours: u32,
    pub short_interval_hours: u32,
}

/// One aligned native-settlement cash flow selected for a funding ticket.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeFundingProjection {
    pub event_count: u32,
    pub settlement_at_ms: i64,
    pub gross_funding_bps: f64,
    pub mismatch_reserve_bps: f64,
    pub conservative_funding_bps: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutablePriceInput {
    pub long_open_price: Option<f64>,
    pub short_open_price: Option<f64>,
    pub long_observed_at_ms: i64,
    pub short_observed_at_ms: i64,
}

/// Inputs needed to prove a positive strategy cash-flow floor.
#[derive(Debug, Clone, Copy)]
pub struct StrategyProfitProofInput {
    pub strategy: Option<StrategyKind>,
    pub spot_leg_mode: Option<SpotLegMode>,
    pub funding: FundingWindowInput,
    pub executable_price: ExecutablePriceInput,
    pub gross_edge_bps: Option<f64>,
    pub total_cost_bps: Option<f64>,
    pub mismatch_buffer_bps: Option<f64>,
    pub target_buffer_bps: Option<f64>,
    pub observed_at_ms: i64,
}

/// Snapshot-bound result used by ticket guards and deterministic artifacts.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyProfitProof {
    pub class: ProfitProofClass,
    pub passed: bool,
    pub gross_cash_flow_bps: Option<f64>,
    pub total_cost_bps: Option<f64>,
    pub target_buffer_bps: Option<f64>,
    pub conservative_net_bps: Option<f64>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct ProofValues {
    gross_bps: Option<f64>,
    cost_bps: Option<f64>,
    target_bps: Option<f64>,
    net_bps: Option<f64>,
}

/// Returns zero only when the two native next-settlement events are aligned.
///
/// The field name is retained for ticket compatibility. A staggered pair has no executable
/// mismatch reserve because it is rejected before ticket construction.
#[must_use]
pub fn funding_window_mismatch_buffer_bps(input: FundingWindowInput) -> Option<f64> {
    funding_timeline::projection_after_input(funding_timeline_input(input)?, 1)
        .map(|projection| projection.mismatch_reserve_yield)
}

/// Returns one venue-native funding payment only while its current settlement schedule is valid.
/// This is used by Spot/Perp ticket repricing so the preview neither drops a proven payment nor
/// carries a stale rate into the projected basis result.
#[must_use]
pub fn current_native_funding_bps(
    rate_bps: Option<f64>,
    next_settlement_ms: i64,
    interval_hours: u32,
    observed_at_ms: i64,
) -> Option<f64> {
    let rate_bps = rate_bps.filter(|value| value.is_finite())?;
    funding_timeline::native_next_settlement_is_plausible(
        next_settlement_ms,
        interval_hours,
        observed_at_ms,
    )
    .then_some(rate_bps)
}

/// Returns the next aligned bilateral settlement only when that one native event exceeds the
/// supplied cost floor. `PerpCross` never accumulates unilateral or later events.
#[must_use]
pub fn first_profitable_funding_projection(
    input: FundingWindowInput,
    required_bps: f64,
) -> Option<NativeFundingProjection> {
    let projection =
        funding_timeline::first_profitable_input(funding_timeline_input(input)?, required_bps)?;
    Some(NativeFundingProjection {
        event_count: projection.event_count,
        settlement_at_ms: projection.settlement_at_ms,
        gross_funding_bps: projection.funding_yield,
        mismatch_reserve_bps: projection.mismatch_reserve_yield,
        conservative_funding_bps: projection.conservative_yield(),
    })
}

/// Evaluates whether the current ticket proves a strictly positive strategy cash-flow floor.
#[must_use]
pub fn evaluate_strategy_profit(input: StrategyProfitProofInput) -> StrategyProfitProof {
    match input.strategy {
        Some(StrategyKind::PerpCross) => funding_cross_proof(input),
        Some(StrategyKind::SpotCross) => spot_cross_proof(input),
        Some(StrategyKind::PerpPriceSpread) => statistical_spread_proof(input),
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp) => projected_basis_proof(input),
        Some(
            StrategyKind::Triangular
            | StrategyKind::FundingCarry
            | StrategyKind::CashAndCarry
            | StrategyKind::OptionsPerpBasis
            | StrategyKind::QuarterlyPerp
            | StrategyKind::OnchainDepeg,
        )
        | None => unproven("当前策略没有可执行的现金流下限证明"),
    }
}

fn funding_cross_proof(input: StrategyProfitProofInput) -> StrategyProfitProof {
    if funding_rates(input.funding).is_none() {
        return unproven("缺少双腿交易所原生资金费率，不能证明结算现金流");
    }
    let Some((long_ms, short_ms)) = settlement_times(input.funding) else {
        return unproven("缺少双腿下一次原生结算时间，不能证明结算顺序");
    };
    if long_ms <= input.observed_at_ms || short_ms <= input.observed_at_ms {
        return unproven("双腿原生结算时间已经过期，必须刷新机会与票据");
    }
    let Some(timeline_input) = funding_timeline_input(input.funding) else {
        return unproven("缺少双腿原生资金费率周期，不能核验共同结算窗口");
    };
    let entry = funding_timeline::assess_entry_input(timeline_input, input.observed_at_ms);
    if !entry.passed {
        return unproven(&entry.detail);
    }
    let Some((total_cost_bps, target_buffer_bps)) = verified_costs(input) else {
        return unproven("缺少完整双腿开平费用、滑点或目标利润缓冲");
    };
    let Some(projection) = funding_timeline::projection_after_input(timeline_input, 1) else {
        return unproven("双腿下一次结算不同窗，不能形成单次双边资金费现金流");
    };
    let expected_mismatch = projection.mismatch_reserve_yield;
    let recorded_mismatch = input
        .mismatch_buffer_bps
        .filter(|value| valid_non_negative(*value));
    if recorded_mismatch.is_none_or(|value| (value - expected_mismatch).abs() > 1e-9) {
        return unproven("共同结算证据未与双腿原生结算事件绑定");
    }

    let gross_bps = projection.funding_yield;
    let net_bps = gross_bps - total_cost_bps - target_buffer_bps;
    let price = perp_cross_policy::evaluate(perp_cross_policy::PriceAlignmentInput {
        long_open_price: input.executable_price.long_open_price,
        short_open_price: input.executable_price.short_open_price,
        long_observed_at_ms: input.executable_price.long_observed_at_ms,
        short_observed_at_ms: input.executable_price.short_observed_at_ms,
        projected_net_funding_bps: Some(net_bps),
    });

    proof(
        ProfitProofClass::Projected,
        net_bps > f64::EPSILON && price.passed,
        ProofValues {
            gross_bps: Some(gross_bps),
            cost_bps: Some(total_cost_bps),
            target_bps: Some(target_buffer_bps),
            net_bps: Some(net_bps),
        },
        format!(
            "共同原生结算 {}：双腿各结算一次，资金费 {:.4}bps - 全周期成本 {:.4}bps - 目标缓冲 {:.4}bps = 投影净值 {:.4}bps；{}",
            projection.settlement_at_ms,
            gross_bps,
            total_cost_bps,
            target_buffer_bps,
            net_bps,
            price.detail
        ),
    )
}

fn spot_cross_proof(input: StrategyProfitProofInput) -> StrategyProfitProof {
    if input.spot_leg_mode != Some(SpotLegMode::SellInventory) {
        return unproven("现货跨所缺少预置库存卖出模式，价差不能锁定为双边现金流");
    }
    let Some(gross_bps) = input
        .gross_edge_bps
        .filter(|value| valid_non_negative(*value))
    else {
        return unproven("缺少现货双边可执行价差");
    };
    let Some((total_cost_bps, target_buffer_bps)) = verified_costs(input) else {
        return unproven("缺少现货双边完整开平与库存再平衡成本");
    };
    let net_bps = gross_bps - total_cost_bps - target_buffer_bps;
    proof(
        ProfitProofClass::Locked,
        net_bps > f64::EPSILON,
        ProofValues {
            gross_bps: Some(gross_bps),
            cost_bps: Some(total_cost_bps),
            target_bps: Some(target_buffer_bps),
            net_bps: Some(net_bps),
        },
        format!(
            "预置库存价差 {:.4}bps - 全周期成本 {:.4}bps - 目标缓冲 {:.4}bps = 保守下限 {:.4}bps",
            gross_bps, total_cost_bps, target_buffer_bps, net_bps
        ),
    )
}

fn statistical_spread_proof(input: StrategyProfitProofInput) -> StrategyProfitProof {
    let values = projected_values(input);
    proof(
        ProfitProofClass::Statistical,
        false,
        values,
        "永续价差依赖未来收敛；历史样本只能形成统计证据，不能锁定本次盈利".to_owned(),
    )
}

fn projected_basis_proof(input: StrategyProfitProofInput) -> StrategyProfitProof {
    let values = projected_values(input);
    proof(
        ProfitProofClass::Projected,
        false,
        values,
        "期现策略尚缺票据绑定的基差退出、借贷与持有现金流下限".to_owned(),
    )
}

fn projected_values(input: StrategyProfitProofInput) -> ProofValues {
    let gross = input
        .gross_edge_bps
        .filter(|value| valid_non_negative(*value));
    let costs = verified_costs(input);
    let total = costs.map(|values| values.0);
    let target = costs.map(|values| values.1);
    let net = gross
        .zip(costs)
        .map(|(gross, (cost, target))| gross - cost - target);
    ProofValues {
        gross_bps: gross,
        cost_bps: total,
        target_bps: target,
        net_bps: net,
    }
}

fn verified_costs(input: StrategyProfitProofInput) -> Option<(f64, f64)> {
    let total = input
        .total_cost_bps
        .filter(|value| valid_non_negative(*value))?;
    let target = input
        .target_buffer_bps
        .filter(|value| valid_non_negative(*value))?;
    Some((total, target))
}

fn funding_rates(input: FundingWindowInput) -> Option<(f64, f64)> {
    Some((
        input.long_funding_bps.filter(|value| value.is_finite())?,
        input.short_funding_bps.filter(|value| value.is_finite())?,
    ))
}

fn settlement_times(input: FundingWindowInput) -> Option<(i64, i64)> {
    (input.long_next_settlement_ms > 0 && input.short_next_settlement_ms > 0).then_some((
        input.long_next_settlement_ms,
        input.short_next_settlement_ms,
    ))
}

fn funding_timeline_input(
    input: FundingWindowInput,
) -> Option<funding_timeline::FundingTimelineInput> {
    let (long_rate, short_rate) = funding_rates(input)?;
    (input.long_interval_hours > 0 && input.short_interval_hours > 0).then_some(
        funding_timeline::FundingTimelineInput {
            long_rate,
            long_next_ms: input.long_next_settlement_ms,
            long_interval_hours: input.long_interval_hours,
            short_rate,
            short_next_ms: input.short_next_settlement_ms,
            short_interval_hours: input.short_interval_hours,
        },
    )
}

fn valid_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn unproven(detail: &str) -> StrategyProfitProof {
    proof(
        ProfitProofClass::Unproven,
        false,
        ProofValues::default(),
        detail.to_owned(),
    )
}

fn proof(
    class: ProfitProofClass,
    passed: bool,
    values: ProofValues,
    detail: String,
) -> StrategyProfitProof {
    StrategyProfitProof {
        class,
        passed,
        gross_cash_flow_bps: values.gross_bps,
        total_cost_bps: values.cost_bps,
        target_buffer_bps: values.target_bps,
        conservative_net_bps: values.net_bps,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    #[test]
    fn aligned_native_funding_passes_as_projected_after_full_costs_and_price_gate() {
        let passing =
            evaluate_strategy_profit(perp_cross_input(2.0, 12.0, 5.0, NOW + 60_000, NOW + 60_010));
        assert!(passing.passed);
        assert_eq!(passing.class, ProfitProofClass::Projected);
        assert_eq!(passing.conservative_net_bps, Some(5.0));

        let failing =
            evaluate_strategy_profit(perp_cross_input(2.0, 2.05, 5.0, NOW + 60_000, NOW + 60_010));
        assert!(!failing.passed);
        assert!(failing
            .conservative_net_bps
            .is_some_and(|value| (value + 4.95).abs() < 1e-9));
    }

    #[test]
    fn staggered_funding_never_accumulates_into_a_profitable_ticket() {
        let funding = FundingWindowInput {
            long_funding_bps: Some(3.0),
            short_funding_bps: Some(14.0),
            long_next_settlement_ms: NOW + 3_600_000,
            short_next_settlement_ms: NOW + 7_200_010,
            long_interval_hours: 1,
            short_interval_hours: 8,
        };
        assert_eq!(funding_window_mismatch_buffer_bps(funding), None);
        let proof = evaluate_strategy_profit(StrategyProfitProofInput {
            strategy: Some(StrategyKind::PerpCross),
            spot_leg_mode: None,
            funding,
            executable_price: executable_price(),
            gross_edge_bps: Some(8.0),
            total_cost_bps: Some(7.0),
            mismatch_buffer_bps: Some(3.0),
            target_buffer_bps: Some(0.0),
            observed_at_ms: NOW,
        });
        assert!(!proof.passed);
        assert_eq!(proof.class, ProfitProofClass::Unproven);
        assert!(proof.detail.contains("下一共同结算"));
    }

    #[test]
    fn aligned_mixed_intervals_use_one_native_payment_per_leg() {
        let settlement = NOW + 60_000;
        let proof = evaluate_strategy_profit(StrategyProfitProofInput {
            strategy: Some(StrategyKind::PerpCross),
            spot_leg_mode: None,
            funding: FundingWindowInput {
                long_funding_bps: Some(1.0),
                short_funding_bps: Some(12.0),
                long_next_settlement_ms: settlement,
                short_next_settlement_ms: settlement,
                long_interval_hours: 1,
                short_interval_hours: 8,
            },
            executable_price: executable_price(),
            gross_edge_bps: Some(11.0),
            total_cost_bps: Some(5.0),
            mismatch_buffer_bps: Some(0.0),
            target_buffer_bps: Some(0.0),
            observed_at_ms: NOW,
        });

        assert!(proof.passed);
        assert_eq!(proof.conservative_net_bps, Some(6.0));
        assert!(proof.detail.contains("双腿各结算一次"));
    }

    #[test]
    fn convergence_spread_remains_statistical() {
        let proof = evaluate_strategy_profit(StrategyProfitProofInput {
            strategy: Some(StrategyKind::PerpPriceSpread),
            spot_leg_mode: None,
            funding: empty_funding(),
            executable_price: ExecutablePriceInput::default(),
            gross_edge_bps: Some(40.0),
            total_cost_bps: Some(20.0),
            mismatch_buffer_bps: Some(0.0),
            target_buffer_bps: Some(2.0),
            observed_at_ms: NOW,
        });
        assert!(!proof.passed);
        assert_eq!(proof.class, ProfitProofClass::Statistical);
    }

    #[test]
    fn prefunded_spot_cross_can_lock_a_positive_floor() {
        let proof = evaluate_strategy_profit(StrategyProfitProofInput {
            strategy: Some(StrategyKind::SpotCross),
            spot_leg_mode: Some(SpotLegMode::SellInventory),
            funding: empty_funding(),
            executable_price: ExecutablePriceInput::default(),
            gross_edge_bps: Some(35.0),
            total_cost_bps: Some(20.0),
            mismatch_buffer_bps: Some(0.0),
            target_buffer_bps: Some(5.0),
            observed_at_ms: NOW,
        });
        assert!(proof.passed);
        assert_eq!(proof.conservative_net_bps, Some(10.0));
    }

    #[test]
    fn native_single_leg_funding_rejects_stale_or_impossible_schedules() {
        assert_eq!(
            current_native_funding_bps(Some(4.0), NOW + 3_600_000, 1, NOW),
            Some(4.0)
        );
        assert_eq!(current_native_funding_bps(Some(4.0), NOW, 1, NOW), None);
        assert_eq!(
            current_native_funding_bps(Some(4.0), NOW + 7_200_000, 1, NOW),
            None
        );
    }

    fn perp_cross_input(
        long_bps: f64,
        short_bps: f64,
        cost_bps: f64,
        long_ms: i64,
        short_ms: i64,
    ) -> StrategyProfitProofInput {
        StrategyProfitProofInput {
            strategy: Some(StrategyKind::PerpCross),
            spot_leg_mode: None,
            funding: FundingWindowInput {
                long_funding_bps: Some(long_bps),
                short_funding_bps: Some(short_bps),
                long_next_settlement_ms: long_ms,
                short_next_settlement_ms: short_ms,
                long_interval_hours: 8,
                short_interval_hours: 8,
            },
            executable_price: executable_price(),
            gross_edge_bps: Some(short_bps - long_bps),
            total_cost_bps: Some(cost_bps),
            mismatch_buffer_bps: Some(0.0),
            target_buffer_bps: Some(0.0),
            observed_at_ms: NOW,
        }
    }

    fn empty_funding() -> FundingWindowInput {
        FundingWindowInput {
            long_funding_bps: None,
            short_funding_bps: None,
            long_next_settlement_ms: 0,
            short_next_settlement_ms: 0,
            long_interval_hours: 0,
            short_interval_hours: 0,
        }
    }

    fn executable_price() -> ExecutablePriceInput {
        ExecutablePriceInput {
            long_open_price: Some(100.0),
            short_open_price: Some(100.02),
            long_observed_at_ms: NOW,
            short_observed_at_ms: NOW + 10,
        }
    }
}
