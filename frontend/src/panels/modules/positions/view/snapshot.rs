use crate::state::load_state::LoadState;
use shared_types::{
    ApiProblem, ListStatus, PortfolioSnapshot, VenueBalanceEnvelope, VenuePositionEnvelope,
};

use super::super::components::{AccountSurfaceEvidence, SectionData};
use super::super::data::{
    has_execution_ledger_context, has_execution_projection, is_current_partial_snapshot_problem,
};
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
        } => with_refresh_status(
            balance_envelope_section(&snapshot.account_state.balances),
            state,
        ),
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
        } => with_refresh_status(
            position_envelope_section(
                &snapshot.positions,
                &snapshot.account_state.positions,
                has_execution_ledger_context(snapshot),
            ),
            state,
        ),
    }
}

pub(super) fn position_envelope_section(
    rows: &[shared_types::PositionRow],
    envelope: &VenuePositionEnvelope,
    verified_ledger_context: bool,
) -> SectionData<Vec<shared_types::PositionRow>> {
    if envelope.status == ListStatus::Fresh
        || (!rows.is_empty() && envelope.problems.is_empty())
        || has_execution_projection(rows)
        || (rows.is_empty() && verified_ledger_context)
    {
        return SectionData::ready(rows.to_vec());
    }
    SectionData::stale(rows.to_vec(), &position_envelope_problem(envelope))
}

// A current snapshot may carry a venue/field problem while its other sections
// are healthy. A failed refresh, unlike that partial snapshot, ages every section.
fn refresh_problem(state: &LoadState<PortfolioSnapshot>) -> Option<&ApiProblem> {
    let LoadState::Stale { value, problem } = state else {
        return None;
    };
    let account = &value.account_state;
    let belongs_to_snapshot = is_current_partial_snapshot_problem(problem)
        || value.summary.nav_evidence.problem.as_ref() == Some(problem)
        || value.summary.pnl_breakdown.evidence.problem.as_ref() == Some(problem)
        || account.problems.contains(problem)
        || account.balances.problems.contains(problem)
        || account.positions.problems.contains(problem)
        || account.open_orders.problems.contains(problem)
        || value
            .problems
            .iter()
            .any(|row| row.to_api_problem() == *problem);
    (!belongs_to_snapshot).then_some(problem)
}

fn with_refresh_status<T>(
    mut section: SectionData<T>,
    state: &LoadState<PortfolioSnapshot>,
) -> SectionData<T> {
    if let Some(problem) = refresh_problem(state) {
        section.status =
            super::super::components::section_state::SectionStatus::stale_from(problem);
    }
    section
}

pub(super) fn position_values_known(state: &LoadState<PortfolioSnapshot>) -> bool {
    let section = position_snapshot_section(state);
    if !section.has_fresh_value() {
        return false;
    }
    let Some(snapshot) = loaded_snapshot(state) else {
        return false;
    };
    section.value.iter().all(|row| {
        row.quantity.is_finite()
            && row.mark_price.is_finite()
            && row.mark_price > 0.0
            && (row.origin == shared_types::PositionOrigin::ExecutionLedger
                || !snapshot.account_state.field_quality.iter().any(|quality| {
                    quality.subject.kind == shared_types::AccountFieldSubjectKind::Position
                        && quality
                            .subject
                            .venue
                            .as_deref()
                            .is_some_and(|v| v.eq_ignore_ascii_case(&row.venue))
                        && quality
                            .subject
                            .symbol
                            .as_deref()
                            .is_some_and(|v| v.eq_ignore_ascii_case(&row.symbol))
                        && matches!(quality.field.as_str(), "markPrice" | "quantity")
                        && quality.status != shared_types::AccountFieldQualityStatus::Actual
                }))
    })
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
            if refresh_problem(state).is_some() {
                ListStatus::Degraded
            } else {
                envelope.status
            },
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
            if refresh_problem(state).is_some() {
                ListStatus::Degraded
            } else {
                envelope.status
            },
            envelope.account_bindings.clone(),
        )
    })
}
