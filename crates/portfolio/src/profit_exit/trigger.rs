use shared_types::{
    normalized_venue_name, AccountFieldQualityStatus, AccountFieldSubjectKind,
    AutoProfitCloseConfig, ExecutionEnvironment, PortfolioSnapshot, PositionRow, PositionSide,
};

use super::ProfitExitValuation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfitExitTrigger {
    TakeProfit,
    StopLoss,
    LiquidationGuard,
}

impl ProfitExitTrigger {
    pub(super) const fn priority(self) -> u8 {
        match self {
            Self::TakeProfit => 1,
            Self::StopLoss => 2,
            Self::LiquidationGuard => 3,
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::TakeProfit => "take_profit",
            Self::StopLoss => "stop_loss",
            Self::LiquidationGuard => "liquidation_guard",
        }
    }
}

pub(super) struct TriggerDecision {
    pub(super) trigger: ProfitExitTrigger,
    pub(super) trigger_observed_at_ms: Option<i64>,
    pub(super) minimum_liquidation_distance_pct: Option<f64>,
    pub(super) risk_venue: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) struct TriggerInput<'a> {
    pub(super) snapshot: &'a PortfolioSnapshot,
    pub(super) config: &'a AutoProfitCloseConfig,
    pub(super) environment: ExecutionEnvironment,
    pub(super) anchor: &'a PositionRow,
    pub(super) partner: &'a PositionRow,
    pub(super) valuation: Option<&'a ProfitExitValuation>,
}

pub(super) fn config_enabled_and_valid(config: &AutoProfitCloseConfig) -> bool {
    let any_enabled =
        config.enabled || config.stop_loss_enabled || config.liquidation_guard_enabled;
    any_enabled
        && non_negative_finite(config.exit_buffer_bps)
        && (!config.enabled
            || (positive_finite(config.min_net_profit_usd) && positive_finite(config.min_roi_bps)))
        && (!config.stop_loss_enabled
            || (positive_finite(config.max_net_loss_usd)
                && positive_finite(config.max_loss_roi_bps)))
        && (!config.liquidation_guard_enabled
            || valid_liquidation_threshold(config.liquidation_exit_distance_pct))
        && (2..=30).contains(&config.confirmation_samples)
        && (10..=3_600).contains(&config.cooldown_secs)
}

pub(super) fn decide(input: TriggerInput<'_>) -> Option<TriggerDecision> {
    let TriggerInput {
        snapshot,
        config,
        environment,
        anchor,
        partner,
        valuation,
    } = input;
    let pair_liquidation = pair_liquidation_risk(snapshot, environment, anchor, partner);
    if config.liquidation_guard_enabled
        && pair_liquidation
            .as_ref()
            .is_some_and(|risk| risk.distance_pct <= config.liquidation_exit_distance_pct)
    {
        return pair_liquidation.map(|risk| TriggerDecision {
            trigger: ProfitExitTrigger::LiquidationGuard,
            trigger_observed_at_ms: Some(risk.observed_at_ms),
            minimum_liquidation_distance_pct: Some(risk.distance_pct),
            risk_venue: Some(risk.venue),
        });
    }
    let valuation = valuation?;
    if config.stop_loss_enabled
        && (valuation.estimated_net_profit_usd <= -config.max_net_loss_usd
            || valuation.estimated_roi_bps <= -config.max_loss_roi_bps)
    {
        return Some(TriggerDecision {
            trigger: ProfitExitTrigger::StopLoss,
            trigger_observed_at_ms: None,
            minimum_liquidation_distance_pct: pair_liquidation
                .as_ref()
                .map(|risk| risk.distance_pct),
            risk_venue: pair_liquidation.map(|risk| risk.venue),
        });
    }
    (config.enabled
        && valuation.estimated_net_profit_usd >= config.min_net_profit_usd
        && valuation.estimated_roi_bps >= config.min_roi_bps)
        .then(|| TriggerDecision {
            trigger: ProfitExitTrigger::TakeProfit,
            trigger_observed_at_ms: None,
            minimum_liquidation_distance_pct: pair_liquidation
                .as_ref()
                .map(|risk| risk.distance_pct),
            risk_venue: pair_liquidation.map(|risk| risk.venue),
        })
}

struct PairLiquidationRisk {
    distance_pct: f64,
    venue: String,
    observed_at_ms: i64,
}

fn pair_liquidation_risk(
    snapshot: &PortfolioSnapshot,
    environment: ExecutionEnvironment,
    anchor: &PositionRow,
    partner: &PositionRow,
) -> Option<PairLiquidationRisk> {
    [anchor, partner]
        .into_iter()
        .filter_map(|row| liquidation_risk_for_row(snapshot, environment, row))
        .min_by(|left, right| left.distance_pct.total_cmp(&right.distance_pct))
}

fn liquidation_risk_for_row(
    snapshot: &PortfolioSnapshot,
    environment: ExecutionEnvironment,
    row: &PositionRow,
) -> Option<PairLiquidationRisk> {
    let distance_pct = valid_distance(row.liquidation_distance_pct?)?;
    let observed_at_ms = if environment == ExecutionEnvironment::Live {
        actual_liquidation_distance_observed_at_ms(snapshot, row)?
    } else {
        snapshot.server_now_ms
    };
    Some(PairLiquidationRisk {
        distance_pct,
        venue: row.venue.clone(),
        observed_at_ms,
    })
}

fn actual_liquidation_distance_observed_at_ms(
    snapshot: &PortfolioSnapshot,
    row: &PositionRow,
) -> Option<i64> {
    snapshot
        .account_state
        .field_quality
        .iter()
        .filter(|quality| {
            quality.subject.kind == AccountFieldSubjectKind::Position
                && quality.field == "liquidationDistancePct"
                && quality.status == AccountFieldQualityStatus::Actual
                && quality.subject.venue.as_deref().is_some_and(|venue| {
                    normalized_venue_name(venue) == normalized_venue_name(&row.venue)
                })
                && quality
                    .subject
                    .symbol
                    .as_deref()
                    .is_some_and(|symbol| symbol.eq_ignore_ascii_case(&row.symbol))
                && quality
                    .subject
                    .side
                    .as_deref()
                    .is_some_and(|side| side.eq_ignore_ascii_case(side_key(row.side)))
        })
        .filter_map(|quality| quality.observed_at_ms)
        .filter(|observed_at_ms| {
            snapshot
                .server_now_ms
                .checked_sub(*observed_at_ms)
                .is_some_and(|age_ms| (0..=super::MAX_SNAPSHOT_AGE_MS).contains(&age_ms))
        })
        .max()
}

const fn side_key(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "long",
        PositionSide::Short => "short",
    }
}

fn valid_distance(value: f64) -> Option<f64> {
    // Signed distance is intentional: a negative value means the mark has
    // already crossed the reported liquidation price and is the highest risk.
    value.is_finite().then_some(value)
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn non_negative_finite(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn valid_liquidation_threshold(value: f64) -> bool {
    value.is_finite() && (0.1..=100.0).contains(&value)
}
