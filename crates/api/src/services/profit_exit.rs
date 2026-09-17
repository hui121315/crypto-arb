use crate::services::{
    action_runs::{self, ActionRunStart},
    close_runs, execution_environment, portfolio_actions,
    ws_publish::{publish_close_run_event, publish_order_event},
};
use crate::state::AppState;
use common::AppError;
use portfolio::{ProfitExitCandidate, ProfitExitTrigger};
use shared_types::{
    ActionRunKind, ActionRunStatus, AutoProfitCloseConfig, ClosePositionRequest, CloseRun,
    CloseRunStatus, ExecutionRun, HedgeTicket, StrategyKind,
};
use std::collections::BTreeSet;

const AUTO_ACTOR: &str = "system:auto-pair-exit";
const AUTO_CLOSE_EVENT: &str = "auto_pair_exit_submitted";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SubmitOutcome {
    Submitted(String),
    Replayed(ActionRunStatus),
}

pub(crate) fn candidates(
    state: &AppState,
    config: &AutoProfitCloseConfig,
    now_ms: i64,
) -> Vec<ProfitExitCandidate> {
    let Some(snapshot) = state.portfolio_snapshot().get_arc_now() else {
        return Vec::new();
    };
    let environment = execution_environment::active(state);
    portfolio::profit_exit_candidates(&snapshot.value, config, environment, now_ms, |run_id| {
        let run = state
            .execution_runs()
            .get(run_id)
            .map(|entry| entry.value().clone())?;
        execution_environment::run_matches(state, &run, environment).then_some(run)
    })
    .into_iter()
    .filter(|candidate| cost_recovery_exit_ready(state, candidate, config, now_ms))
    .filter(|candidate| automatic_exit_attempt_ready(state, &candidate.run_id))
    .collect()
}

fn cost_recovery_exit_ready(
    state: &AppState,
    candidate: &ProfitExitCandidate,
    config: &AutoProfitCloseConfig,
    now_ms: i64,
) -> bool {
    let Some(run) = state
        .execution_runs()
        .get(&candidate.run_id)
        .map(|entry| entry.value().clone())
    else {
        return true;
    };
    let Some(ticket) = state
        .hedge_tickets()
        .get(&run.ticket_id)
        .map(|entry| entry.value().clone())
    else {
        return true;
    };
    cost_recovery_exit_ready_for(candidate, config, &run, &ticket, now_ms)
}

fn cost_recovery_exit_ready_for(
    candidate: &ProfitExitCandidate,
    config: &AutoProfitCloseConfig,
    run: &ExecutionRun,
    ticket: &HedgeTicket,
    now_ms: i64,
) -> bool {
    if candidate.trigger != ProfitExitTrigger::StopLoss
        || !cost_recovery_hold_applies(ticket)
        || run.ticket_id != ticket.ticket_id
    {
        return true;
    }
    let Some(hold_ms) = recommended_hold_ms(ticket) else {
        return true;
    };
    if now_ms >= run.created_at_ms.saturating_add(hold_ms) {
        return true;
    }
    let Some(valuation) = candidate.valuation.as_ref() else {
        return false;
    };
    let market_pnl_usd = valuation.gross_unrealized_pnl_usd + valuation.funding_pnl_usd;
    let market_roi_bps = market_pnl_usd / valuation.matched_notional_usd * 10_000.0;
    market_pnl_usd <= -config.max_net_loss_usd || market_roi_bps <= -config.max_loss_roi_bps
}

fn cost_recovery_hold_applies(ticket: &HedgeTicket) -> bool {
    ticket.strategy == Some(StrategyKind::PerpPriceSpread)
        || ticket
            .cost
            .as_ref()
            .is_some_and(|cost| cost.recommended_hold_periods > 1)
}

fn recommended_hold_ms(ticket: &HedgeTicket) -> Option<i64> {
    let hours = ticket.cost.as_ref()?.recommended_hold_hours;
    if !hours.is_finite() || hours <= 0.0 {
        return None;
    }
    let milliseconds = hours * 3_600_000.0;
    (milliseconds.is_finite() && milliseconds <= i64::MAX as f64)
        .then_some(milliseconds.round() as i64)
}

pub(crate) async fn submit_candidate(
    state: &AppState,
    candidate: &ProfitExitCandidate,
) -> Result<SubmitOutcome, AppError> {
    let idempotency_key = idempotency_key(candidate);
    let claim = action_runs::begin_idempotent(
        state,
        ActionRunStart {
            kind: ActionRunKind::PortfolioClosePair,
            actor: AUTO_ACTOR.to_owned(),
            target: Some(candidate.run_id.clone()),
            idempotency_key: Some(idempotency_key.clone()),
            message: accepted_message(candidate),
        },
    )?;
    if claim.is_replayed() {
        return Ok(SubmitOutcome::Replayed(claim.run().status));
    }

    let result = close_pair(state, candidate, &idempotency_key).await;
    let mut close_run = match result {
        Ok(run) => run,
        Err(error) => return action_runs::fail_response(state, &claim.run().id, error),
    };
    close_run.action_run_id = Some(claim.run().id.clone());
    close_run.request_id = claim.run().request_id.clone();
    close_run = close_runs::record(state, close_run);
    action_runs::finish_status_with_payload(
        state,
        &claim.run().id,
        close_runs::action_status(close_run.status),
        close_run.message.clone(),
        close_run.problem.clone(),
        &close_run,
    )?;
    publish_events(state, &close_run)?;
    Ok(SubmitOutcome::Submitted(close_run.id))
}

async fn close_pair(
    state: &AppState,
    candidate: &ProfitExitCandidate,
    idempotency_key: &str,
) -> Result<CloseRun, AppError> {
    let payload = ClosePositionRequest {
        side: Some(candidate.side),
        snapshot_version: Some(candidate.snapshot_version.clone()),
        expected_leg_count: Some(2),
        reason: Some(candidate.close_reason()),
    };
    let context = portfolio_actions::CloseRequestContext::from_position_request(&payload)?
        .with_idempotency_key(idempotency_key.to_owned());
    portfolio_actions::close_position_pair(
        state,
        &candidate.venue,
        &candidate.symbol,
        payload.side,
        context,
    )
    .await
}

fn publish_events(state: &AppState, close_run: &CloseRun) -> Result<(), AppError> {
    for order in close_run.legs.iter().filter_map(|leg| leg.order.as_ref()) {
        publish_order_event(state, AUTO_CLOSE_EVENT, order)?;
    }
    publish_close_run_event(state, AUTO_CLOSE_EVENT, close_run)
}

fn automatic_exit_attempt_ready(state: &AppState, execution_run_id: &str) -> bool {
    let mut linked_action_run_ids = BTreeSet::new();
    for entry in state
        .close_runs()
        .iter()
        .filter(|entry| close_run_matches_execution(entry, execution_run_id))
    {
        if close_run_blocks_new_attempt(entry.status) {
            return false;
        }
        if let Some(action_run_id) = entry.action_run_id.as_ref() {
            linked_action_run_ids.insert(action_run_id.clone());
        }
    }

    !state.action_runs().iter().any(|entry| {
        entry.kind == ActionRunKind::PortfolioClosePair
            && entry.actor == AUTO_ACTOR
            && entry.target.as_deref() == Some(execution_run_id)
            && !linked_action_run_ids.contains(&entry.id)
            && unlinked_action_blocks_new_attempt(entry.status)
    })
}

fn close_run_matches_execution(run: &CloseRun, execution_run_id: &str) -> bool {
    run.legs.iter().any(|leg| {
        leg.pair_evidence
            .as_ref()
            .is_some_and(|evidence| evidence.run_id == execution_run_id)
    })
}

const fn close_run_blocks_new_attempt(status: CloseRunStatus) -> bool {
    !matches!(status, CloseRunStatus::Failed | CloseRunStatus::Compensated)
}

const fn unlinked_action_blocks_new_attempt(status: ActionRunStatus) -> bool {
    !matches!(status, ActionRunStatus::Failed)
}

fn idempotency_key(candidate: &ProfitExitCandidate) -> String {
    format!(
        "auto-pair-exit:{}:{}:{}",
        candidate.run_id,
        candidate.trigger.key(),
        candidate.observed_at_ms
    )
}

fn accepted_message(candidate: &ProfitExitCandidate) -> String {
    let estimated_net_profit_usd = candidate
        .valuation
        .as_ref()
        .map(|valuation| valuation.estimated_net_profit_usd);
    match candidate.trigger {
        ProfitExitTrigger::TakeProfit => estimated_net_profit_usd.map_or_else(
            || "automatic pair take-profit accepted; valuation unavailable".to_owned(),
            |value| {
                format!("automatic pair take-profit accepted; estimated net profit ${value:.4}")
            },
        ),
        ProfitExitTrigger::StopLoss => estimated_net_profit_usd.map_or_else(
            || "automatic pair stop-loss accepted; valuation unavailable".to_owned(),
            |value| format!("automatic pair stop-loss accepted; estimated net PnL ${value:.4}"),
        ),
        ProfitExitTrigger::LiquidationGuard => format!(
            "automatic pair liquidation guard accepted; {} distance {:.4}%",
            candidate.risk_venue.as_deref().unwrap_or("unknown venue"),
            candidate
                .minimum_liquidation_distance_pct
                .unwrap_or_default()
        ),
    }
}

#[cfg(test)]
mod tests;
