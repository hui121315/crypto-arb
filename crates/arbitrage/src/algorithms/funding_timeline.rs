//! Native funding-window proof for cross-venue perpetual funding.
//!
//! `PerpCross` executes around one bilateral funding event. It never waits through unilateral
//! settlements to accumulate a later profit.

use crate::models::RawOpportunity;

const SETTLEMENT_ALIGNMENT_TOLERANCE_MS: u64 = 1_000;
const MAX_ALIGNMENT_STEPS: u32 = 64;
const HOUR_MS: i64 = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FundingProjection {
    pub(crate) event_count: u32,
    pub(crate) settlement_at_ms: i64,
    pub(crate) funding_yield: f64,
    pub(crate) worst_prefix_yield: f64,
    pub(crate) mismatch_reserve_yield: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FundingTimelineInput {
    pub(crate) long_rate: f64,
    pub(crate) long_next_ms: i64,
    pub(crate) long_interval_hours: u32,
    pub(crate) short_rate: f64,
    pub(crate) short_next_ms: i64,
    pub(crate) short_interval_hours: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FundingEntryAssessment {
    pub(crate) passed: bool,
    pub(crate) detail: String,
}

impl FundingProjection {
    pub(crate) fn conservative_yield(self) -> f64 {
        self.funding_yield
    }
}

pub(crate) fn first_funding_projection(raw: &RawOpportunity) -> Option<FundingProjection> {
    projection_after_input(input_from_raw(raw), 1)
}

pub(crate) fn current_settlements_aligned(raw: &RawOpportunity) -> bool {
    settlements_aligned(input_from_raw(raw))
}

pub(crate) fn projection_after_input(
    input: FundingTimelineInput,
    _boundaries: u32,
) -> Option<FundingProjection> {
    if !valid_input(input) || !settlements_aligned(input) {
        return None;
    }
    let funding_yield = input.short_rate - input.long_rate;
    Some(FundingProjection {
        event_count: 1,
        settlement_at_ms: input.long_next_ms.max(input.short_next_ms),
        funding_yield,
        worst_prefix_yield: funding_yield.min(0.0),
        mismatch_reserve_yield: 0.0,
    })
}

pub(crate) fn current_joint_event_input(
    input: FundingTimelineInput,
    observed_at_ms: i64,
) -> Option<FundingProjection> {
    if !valid_input(input)
        || input.long_next_ms <= observed_at_ms
        || input.short_next_ms <= observed_at_ms
        || !native_next_settlement_is_plausible(
            input.long_next_ms,
            input.long_interval_hours,
            observed_at_ms,
        )
        || !native_next_settlement_is_plausible(
            input.short_next_ms,
            input.short_interval_hours,
            observed_at_ms,
        )
        || !settlements_aligned(input)
    {
        return None;
    }
    projection_after_input(input, 1)
}

pub(crate) fn first_profitable_input(
    input: FundingTimelineInput,
    total_cost: f64,
) -> Option<FundingProjection> {
    let total_cost = finite_non_negative(total_cost)?;
    let projection = projection_after_input(input, 1)?;
    (projection.conservative_yield() > total_cost + f64::EPSILON).then_some(projection)
}

pub(crate) fn assess_entry(raw: &RawOpportunity, observed_at_ms: i64) -> FundingEntryAssessment {
    assess_entry_input(input_from_raw(raw), observed_at_ms)
}

pub(crate) fn assess_entry_input(
    input: FundingTimelineInput,
    observed_at_ms: i64,
) -> FundingEntryAssessment {
    if !valid_input(input) {
        return blocked("缺少双腿原生资金费率周期或下一结算时间，仅观察不执行");
    }
    if input.long_next_ms <= observed_at_ms || input.short_next_ms <= observed_at_ms {
        return blocked("双腿原生资金费结算时间已过期，等待刷新后再执行");
    }
    if !native_next_settlement_is_plausible(
        input.long_next_ms,
        input.long_interval_hours,
        observed_at_ms,
    ) || !native_next_settlement_is_plausible(
        input.short_next_ms,
        input.short_interval_hours,
        observed_at_ms,
    ) {
        return blocked(format!(
            "{}h/{}h 下一结算时间超过对应原生周期，行情时间表可能陈旧，等待刷新后再执行",
            input.long_interval_hours, input.short_interval_hours
        ));
    }
    let skew_ms = input.long_next_ms.abs_diff(input.short_next_ms);
    if skew_ms > SETTLEMENT_ALIGNMENT_TOLERANCE_MS {
        return next_common_settlement(input).map_or_else(
            || {
                blocked(format!(
                    "{}h/{}h 原生结算时间表在可验证窗口内没有共同结算点，仅观察不执行",
                    input.long_interval_hours, input.short_interval_hours
                ))
            },
            |common_ms| {
                let wait_minutes = common_ms.saturating_sub(observed_at_ms) as f64 / 60_000.0;
                blocked(format!(
                    "{}h/{}h 当前下一结算不同窗；下一共同结算约 {:.1} 分钟后，等待两腿下一结算时间同时指向该时刻",
                    input.long_interval_hours, input.short_interval_hours, wait_minutes
                ))
            },
        );
    }
    FundingEntryAssessment {
        passed: true,
        detail: format!(
            "{}h/{}h 双腿下一次原生资金费结算同窗",
            input.long_interval_hours, input.short_interval_hours
        ),
    }
}

pub(crate) fn hold_hours(
    raw: &RawOpportunity,
    _boundaries: u32,
    observed_at_ms: i64,
) -> Option<f64> {
    let projection = first_funding_projection(raw)?;
    let duration_ms = projection.settlement_at_ms.saturating_sub(observed_at_ms);
    (duration_ms > 0).then_some(duration_ms as f64 / HOUR_MS as f64)
}

fn input_from_raw(raw: &RawOpportunity) -> FundingTimelineInput {
    FundingTimelineInput {
        long_rate: raw.long_rate.rate,
        long_next_ms: raw.long_rate.next_funding_time,
        long_interval_hours: raw.long_rate.funding_interval,
        short_rate: raw.short_rate.rate,
        short_next_ms: raw.short_rate.next_funding_time,
        short_interval_hours: raw.short_rate.funding_interval,
    }
}

fn valid_input(input: FundingTimelineInput) -> bool {
    input.long_rate.is_finite()
        && input.short_rate.is_finite()
        && input.long_next_ms > 0
        && input.short_next_ms > 0
        && input.long_interval_hours > 0
        && input.short_interval_hours > 0
}

fn settlements_aligned(input: FundingTimelineInput) -> bool {
    input.long_next_ms.abs_diff(input.short_next_ms) <= SETTLEMENT_ALIGNMENT_TOLERANCE_MS
}

pub(crate) fn native_next_settlement_is_plausible(
    next_ms: i64,
    interval_hours: u32,
    observed_at_ms: i64,
) -> bool {
    let Some(interval_ms) = i64::from(interval_hours).checked_mul(HOUR_MS) else {
        return false;
    };
    let remaining_ms = next_ms.saturating_sub(observed_at_ms);
    remaining_ms > 0
        && remaining_ms <= interval_ms.saturating_add(SETTLEMENT_ALIGNMENT_TOLERANCE_MS as i64)
}

fn next_common_settlement(input: FundingTimelineInput) -> Option<i64> {
    let long_step = i64::from(input.long_interval_hours).checked_mul(HOUR_MS)?;
    let short_step = i64::from(input.short_interval_hours).checked_mul(HOUR_MS)?;
    let mut long_ms = input.long_next_ms;
    let mut short_ms = input.short_next_ms;
    for _ in 0..MAX_ALIGNMENT_STEPS {
        if long_ms.abs_diff(short_ms) <= SETTLEMENT_ALIGNMENT_TOLERANCE_MS {
            return Some(long_ms.max(short_ms));
        }
        if long_ms < short_ms {
            long_ms = long_ms.checked_add(long_step)?;
        } else {
            short_ms = short_ms.checked_add(short_step)?;
        }
    }
    None
}

fn finite_non_negative(value: f64) -> Option<f64> {
    value.is_finite().then_some(value.max(0.0))
}

fn blocked(detail: impl Into<String>) -> FundingEntryAssessment {
    FundingEntryAssessment {
        passed: false,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
