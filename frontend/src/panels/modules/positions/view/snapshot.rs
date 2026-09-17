use crate::state::load_state::LoadState;
use shared_types::{
    ApiProblem, ListStatus, PortfolioSnapshot, VenueBalanceEnvelope, VenuePositionEnvelope,
};

use super::super::components::{AccountSurfaceEvidence, SectionData};
use super::super::data::{has_execution_ledger_context, has_execution_projection};
use super::derive::loaded_snapshot;

pub(super) fn snapshot_values<T>(
    state: &LoadState<PortfolioSnapshot>,
    select: impl FnOnce(&PortfolioSnapshot) -> Vec<T>,
) -> Vec<T> {
    match state {
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => select(snapshot),
        LoadState::Loading | LoadState::Error(_) => Vec::new(),
    }
}

pub(super) fn balance_snapshot_section(
    state: &LoadState<PortfolioSnapshot>,
) -> SectionData<Vec<shared_types::VenueBalanceInfo>> {
    match state {
        LoadState::Loading => SectionData::loading(),
        LoadState::Error(problem) => SectionData::error(problem),
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => balance_envelope_section(&snapshot.account_state.balances),
    }
}

pub(super) fn position_snapshot_section(
    state: &LoadState<PortfolioSnapshot>,
) -> SectionData<Vec<shared_types::PositionRow>> {
    match state {
        LoadState::Loading => SectionData::loading(),
        LoadState::Error(problem) => SectionData::error(problem),
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => position_envelope_section(
            &snapshot.positions,
            &snapshot.account_state.positions,
            !snapshot.recent_close_runs.is_empty(),
        ),
    }
}

pub(super) fn position_envelope_section(
    rows: &[shared_types::PositionRow],
    envelope: &VenuePositionEnvelope,
    has_execution_history: bool,
) -> SectionData<Vec<shared_types::PositionRow>> {
    if envelope.status == ListStatus::Fresh
        || (!rows.is_empty() && envelope.problems.is_empty())
        || has_execution_projection(rows)
        || (rows.is_empty() && has_execution_history)
    {
        return SectionData::ready(rows.to_vec());
    }
    SectionData::stale(rows.to_vec(), &position_envelope_problem(envelope))
}

fn position_envelope_problem(envelope: &VenuePositionEnvelope) -> ApiProblem {
    envelope.problems.first().cloned().unwrap_or_else(|| {
        ApiProblem::new(
            shared_types::problem::codes::POSITION_READ_DEGRADED,
            "position envelope degraded",
        )
        .with_source(envelope.source.clone())
    })
}

fn balance_envelope_section(
    envelope: &VenueBalanceEnvelope,
) -> SectionData<Vec<shared_types::VenueBalanceInfo>> {
    if envelope.status == ListStatus::Fresh {
        return SectionData::ready(envelope.rows.clone());
    }
    let problem = balance_envelope_problem(envelope);
    SectionData::stale(envelope.rows.clone(), &problem)
}

fn balance_envelope_problem(envelope: &VenueBalanceEnvelope) -> ApiProblem {
    envelope.problems.first().cloned().unwrap_or_else(|| {
        ApiProblem::new(
            shared_types::problem::codes::BALANCE_READ_DEGRADED,
            "balance envelope degraded",
        )
        .with_source(envelope.source.clone())
    })
}

pub(super) fn position_surface_evidence(
    state: &LoadState<PortfolioSnapshot>,
) -> Option<AccountSurfaceEvidence> {
    loaded_snapshot(state).map(|snapshot| {
        if has_execution_ledger_context(snapshot) {
            return None;
        }
        let envelope = &snapshot.account_state.positions;
        Some(AccountSurfaceEvidence::new(
            envelope.source.clone(),
            envelope.observed_at_ms,
            envelope.status,
            envelope.account_bindings.clone(),
        ))
    })?
}

pub(super) fn balance_surface_evidence(
    state: &LoadState<PortfolioSnapshot>,
) -> Option<AccountSurfaceEvidence> {
    loaded_snapshot(state).map(|snapshot| {
        let envelope = &snapshot.account_state.balances;
        AccountSurfaceEvidence::new(
            envelope.source.clone(),
            envelope.observed_at_ms,
            envelope.status,
            envelope.account_bindings.clone(),
        )
    })
}
