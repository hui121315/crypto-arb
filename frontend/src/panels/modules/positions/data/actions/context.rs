use crate::api::rest::MutationRequestContext;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    ActionEvidence, ActionRunKind, CloseRun, PortfolioSnapshot, PositionRow,
    CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE,
};
use std::collections::BTreeMap;

use super::super::runs::{
    close_run_next_attempt_anchor, latest_close_all_attempt_anchor,
    latest_position_close_attempt_anchor,
};

pub(super) type AttemptAnchor = (String, i64);

pub(in crate::panels::modules::positions) fn close_request_context(
    operation: &str,
    target: &str,
    snapshot_version: Option<&str>,
) -> MutationRequestContext {
    close_request_context_with_attempt(operation, target, snapshot_version, None)
}

pub(in crate::panels::modules::positions) fn close_request_context_with_attempt(
    operation: &str,
    target: &str,
    snapshot_version: Option<&str>,
    attempt_anchor: Option<&str>,
) -> MutationRequestContext {
    let attempt_scope = attempt_anchor
        .filter(|anchor| !anchor.trim().is_empty())
        .map(|anchor| format!(":after={anchor}"))
        .unwrap_or_default();
    MutationRequestContext::with_idempotency_key(format!(
        "positions-{operation}:{}:{target}{attempt_scope}",
        snapshot_version.unwrap_or("missing-snapshot"),
    ))
}

#[cfg(test)]
pub(in crate::panels::modules::positions) fn close_all_request_context(
    snapshot_version: Option<&str>,
    confirmation_phrase: &str,
    execution_scope: &str,
) -> MutationRequestContext {
    close_all_request_context_with_attempt(
        snapshot_version,
        confirmation_phrase,
        execution_scope,
        None,
    )
}

pub(in crate::panels::modules::positions) fn close_all_request_context_with_attempt(
    snapshot_version: Option<&str>,
    confirmation_phrase: &str,
    execution_scope: &str,
    attempt_anchor: Option<&str>,
) -> MutationRequestContext {
    let confirmation_scope =
        if confirmation_phrase.trim() == CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE {
            "confirmed"
        } else {
            "unconfirmed"
        };
    close_request_context_with_attempt(
        "all",
        &format!("portfolio:{confirmation_scope}:{execution_scope}"),
        snapshot_version,
        attempt_anchor,
    )
}

pub(super) fn position_attempt_anchor(
    anchors: RwSignal<BTreeMap<String, AttemptAnchor>>,
    target: &str,
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
    row: &PositionRow,
    expected_leg_count: usize,
) -> Option<String> {
    let local = anchors.with_untracked(|anchors| anchors.get(target).cloned());
    let snapshot = snapshot_state.get_untracked();
    let selected = newest_anchor(
        local,
        snapshot.value().and_then(|snapshot| {
            latest_position_close_attempt_anchor(snapshot, row, expected_leg_count)
        }),
    );
    if let Some(candidate) = selected.as_ref() {
        remember_newest_anchor(anchors, target, candidate.clone());
    }
    selected.map(|(anchor, _)| anchor)
}

pub(super) fn remember_position_attempt_anchor(
    anchors: RwSignal<BTreeMap<String, AttemptAnchor>>,
    target: &str,
    run: &CloseRun,
) {
    let Some(candidate) = close_run_next_attempt_anchor(run) else {
        return;
    };
    remember_newest_anchor(anchors, target, candidate);
}

pub(super) fn close_all_attempt_anchor(
    anchor: RwSignal<Option<AttemptAnchor>>,
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) -> Option<String> {
    let snapshot = snapshot_state.get_untracked();
    let selected = newest_anchor(
        anchor.get_untracked(),
        snapshot.value().and_then(latest_close_all_attempt_anchor),
    );
    if let Some(candidate) = selected.as_ref() {
        anchor.set(Some(candidate.clone()));
    }
    selected.map(|(anchor, _)| anchor)
}

fn newest_anchor(
    left: Option<AttemptAnchor>,
    right: Option<AttemptAnchor>,
) -> Option<AttemptAnchor> {
    match (left, right) {
        (Some(left), Some(right)) if right.1 > left.1 => Some(right),
        (Some(left), Some(_)) | (Some(left), None) => Some(left),
        (None, right) => right,
    }
}

fn remember_newest_anchor(
    anchors: RwSignal<BTreeMap<String, AttemptAnchor>>,
    target: &str,
    candidate: AttemptAnchor,
) {
    anchors.update(|anchors| {
        let current = anchors.get(target).cloned();
        if newest_anchor(current, Some(candidate.clone())).as_ref() == Some(&candidate) {
            anchors.insert(target.to_owned(), candidate);
        }
    });
}

pub(in crate::panels::modules::positions) fn position_scope_evidence(
    context: &MutationRequestContext,
    action_kind: ActionRunKind,
    row: &PositionRow,
) -> ActionEvidence {
    let mut venues = vec![row.venue.clone()];
    let mut symbols = vec![row.symbol.clone()];
    if let Some(pair) = &row.pair_evidence {
        venues.push(pair.partner_venue.clone());
        symbols.push(pair.partner_symbol.clone());
    }
    context
        .evidence()
        .with_action_kind(action_kind)
        .with_venues(venues)
        .with_symbols(symbols)
}

pub(in crate::panels::modules::positions) fn portfolio_scope_evidence(
    context: &MutationRequestContext,
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) -> ActionEvidence {
    let (venues, symbols): (Vec<String>, Vec<String>) = snapshot_state
        .get_untracked()
        .value()
        .map(|snapshot| {
            (
                snapshot
                    .positions
                    .iter()
                    .map(|position| position.venue.clone())
                    .collect(),
                snapshot
                    .positions
                    .iter()
                    .map(|position| position.symbol.clone())
                    .collect(),
            )
        })
        .unwrap_or_default();
    context
        .evidence()
        .with_action_kind(ActionRunKind::PortfolioCloseAll)
        .with_venues(venues)
        .with_symbols(symbols)
}
