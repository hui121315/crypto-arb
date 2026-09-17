use shared_types::{
    AutoProfitCloseConfig, ExecutionEnvironment, ExecutionRun, PortfolioSnapshot, PositionRow,
    PositionSide,
};

#[path = "profit_exit/evidence.rs"]
mod evidence;
use evidence::{
    eligible_execution_run, is_canonical_anchor, pair_position_evidence_observed_at_ms,
    unique_partner,
};

#[path = "profit_exit/trigger.rs"]
mod trigger;
pub use trigger::ProfitExitTrigger;
use trigger::{config_enabled_and_valid, decide, TriggerInput};

#[path = "profit_exit/valuation.rs"]
mod valuation;
use valuation::profit_exit_valuation;
pub use valuation::ProfitExitValuation;

const MAX_SNAPSHOT_AGE_MS: i64 = 6_000;

#[derive(Debug, Clone, PartialEq)]
pub struct ProfitExitCandidate {
    pub run_id: String,
    pub venue: String,
    pub symbol: String,
    pub side: PositionSide,
    pub snapshot_version: String,
    pub observed_at_ms: i64,
    pub valuation: Option<ProfitExitValuation>,
    pub trigger: ProfitExitTrigger,
    pub minimum_liquidation_distance_pct: Option<f64>,
    pub risk_venue: Option<String>,
}

impl ProfitExitCandidate {
    pub fn close_reason(&self) -> String {
        let valuation = self.valuation.as_ref().map_or_else(
            || "netUsd=unknown roiBps=unknown".to_owned(),
            |value| {
                format!(
                    "netUsd={:.4} roiBps={:.4}",
                    value.estimated_net_profit_usd, value.estimated_roi_bps
                )
            },
        );
        format!(
            "auto_pair_exit trigger={} {} minLiqDistancePct={}",
            self.trigger.key(),
            valuation,
            self.minimum_liquidation_distance_pct
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "unknown".to_owned())
        )
    }

    pub fn confirmation_key(&self) -> String {
        format!("{}:{}", self.run_id, self.trigger.key())
    }
}

pub fn profit_exit_candidates<F>(
    snapshot: &PortfolioSnapshot,
    config: &AutoProfitCloseConfig,
    environment: ExecutionEnvironment,
    now_ms: i64,
    mut execution_run: F,
) -> Vec<ProfitExitCandidate>
where
    F: FnMut(&str) -> Option<ExecutionRun>,
{
    if !snapshot_eligible(snapshot, config, now_ms) {
        return Vec::new();
    }
    let mut candidates = snapshot
        .positions
        .iter()
        .filter_map(|anchor| {
            pair_candidate(snapshot, config, environment, anchor, &mut execution_run)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .trigger
            .priority()
            .cmp(&left.trigger.priority())
            .then_with(|| trigger_metric(left).total_cmp(&trigger_metric(right)))
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    candidates.dedup_by(|left, right| left.run_id == right.run_id);
    candidates
}

fn snapshot_eligible(
    snapshot: &PortfolioSnapshot,
    config: &AutoProfitCloseConfig,
    now_ms: i64,
) -> bool {
    let snapshot_age_ms = now_ms.checked_sub(snapshot.server_now_ms);
    config_enabled_and_valid(config)
        && !snapshot.snapshot_version.trim().is_empty()
        && snapshot.server_now_ms > 0
        && snapshot_age_ms.is_some_and(|age_ms| (0..=MAX_SNAPSHOT_AGE_MS).contains(&age_ms))
}

fn pair_candidate<F>(
    snapshot: &PortfolioSnapshot,
    config: &AutoProfitCloseConfig,
    environment: ExecutionEnvironment,
    anchor: &PositionRow,
    execution_run: &mut F,
) -> Option<ProfitExitCandidate>
where
    F: FnMut(&str) -> Option<ExecutionRun>,
{
    let evidence = anchor.pair_evidence.as_ref()?;
    if !is_canonical_anchor(anchor, evidence) {
        return None;
    }
    let partner = unique_partner(&snapshot.positions, anchor)?;
    let run = execution_run(&evidence.run_id)?;
    evaluate_pair(snapshot, config, environment, anchor, partner, &run)
}

fn evaluate_pair(
    snapshot: &PortfolioSnapshot,
    config: &AutoProfitCloseConfig,
    environment: ExecutionEnvironment,
    anchor: &PositionRow,
    partner: &PositionRow,
    run: &ExecutionRun,
) -> Option<ProfitExitCandidate> {
    let evidence = anchor.pair_evidence.as_ref()?;
    eligible_execution_run(run, evidence, anchor, partner)?;
    let position_evidence_observed_at_ms =
        pair_position_evidence_observed_at_ms(snapshot, environment, anchor, partner)?;
    let valuation = profit_exit_valuation(run, evidence, anchor, partner, config);
    let decision = decide(TriggerInput {
        snapshot,
        config,
        environment,
        anchor,
        partner,
        valuation: valuation.as_ref(),
    })?;
    Some(ProfitExitCandidate {
        run_id: evidence.run_id.clone(),
        venue: anchor.venue.clone(),
        symbol: anchor.symbol.clone(),
        side: anchor.side,
        snapshot_version: snapshot.snapshot_version.clone(),
        observed_at_ms: decision
            .trigger_observed_at_ms
            .unwrap_or(position_evidence_observed_at_ms),
        valuation,
        trigger: decision.trigger,
        minimum_liquidation_distance_pct: decision.minimum_liquidation_distance_pct,
        risk_venue: decision.risk_venue,
    })
}

fn trigger_metric(candidate: &ProfitExitCandidate) -> f64 {
    match candidate.trigger {
        ProfitExitTrigger::LiquidationGuard => candidate
            .minimum_liquidation_distance_pct
            .unwrap_or(f64::MAX),
        ProfitExitTrigger::StopLoss => candidate
            .valuation
            .as_ref()
            .map_or(f64::MAX, |value| value.estimated_net_profit_usd),
        ProfitExitTrigger::TakeProfit => candidate
            .valuation
            .as_ref()
            .map_or(f64::MAX, |value| -value.estimated_net_profit_usd),
    }
}

#[cfg(test)]
#[path = "profit_exit/tests.rs"]
mod tests;
