//! snapshot / nav 历史 / close-run 回灌的行为测试。

use super::super::*;
use super::support::*;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ApiProblem, CloseRunStatus};

#[test]
fn snapshot_response_accepts_only_latest_token() {
    assert!(snapshot_response_is_latest(9, 9));
    assert!(!snapshot_response_is_latest(9, 8));
    assert!(!snapshot_response_is_latest(9, 10));
}

#[test]
fn close_run_ws_update_waits_for_first_snapshot() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Loading);
        let pending = RwSignal::new(Vec::new());
        let version = RwSignal::new(1);
        let gate = SnapshotRequestGate { version, token: 1 };

        merge_close_run_update(
            state,
            pending,
            close_run_with_id("close-pending", CloseRunStatus::UnwindRequired, 10),
        );

        assert!(matches!(state.get_untracked(), LoadState::Loading));
        assert_eq!(pending.get_untracked().len(), 1);

        apply_snapshot_result(
            state,
            pending,
            gate,
            Ok(portfolio_envelope(portfolio_snapshot("pos-1"))),
        );

        let snapshot_state = state.get_untracked();
        assert!(matches!(snapshot_state, LoadState::Ready(_)));
        if let LoadState::Ready(snapshot) = snapshot_state {
            assert_eq!(snapshot.recent_close_runs.len(), 1);
            assert_eq!(snapshot.recent_close_runs[0].id, "close-pending");
        }
        assert!(pending.get_untracked().is_empty());
    });
}

#[test]
fn close_run_pending_queue_dedupes_before_snapshot_merge() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Error(ApiProblem::new("ERR", "loading failed")));
        let pending = RwSignal::new(Vec::new());
        let version = RwSignal::new(7);
        let gate = SnapshotRequestGate { version, token: 7 };

        merge_close_run_update(
            state,
            pending,
            close_run_with_id("close-1", CloseRunStatus::Submitted, 10),
        );
        merge_close_run_update(
            state,
            pending,
            close_run_with_id("close-1", CloseRunStatus::UnwindRequired, 12),
        );
        merge_close_run_update(
            state,
            pending,
            close_run_with_id("close-2", CloseRunStatus::Submitted, 11),
        );

        apply_snapshot_result(
            state,
            pending,
            gate,
            Ok(portfolio_envelope(portfolio_snapshot("pos-2"))),
        );

        let snapshot_state = state.get_untracked();
        assert!(matches!(snapshot_state, LoadState::Ready(_)));
        if let LoadState::Ready(snapshot) = snapshot_state {
            assert_eq!(snapshot.recent_close_runs.len(), 2);
            assert_eq!(snapshot.recent_close_runs[0].id, "close-1");
            assert_eq!(
                snapshot.recent_close_runs[0].status,
                CloseRunStatus::UnwindRequired
            );
            assert_eq!(snapshot.recent_close_runs[1].id, "close-2");
        }
        assert!(pending.get_untracked().is_empty());
    });
}

#[test]
fn newer_close_receipt_survives_older_ws_event_and_rest_snapshot() {
    Owner::new().with(|| {
        let mut initial = portfolio_snapshot("pos-1");
        initial.recent_close_runs =
            vec![close_run_with_id("close-1", CloseRunStatus::Succeeded, 30)];
        let state = RwSignal::new(LoadState::Ready(initial));
        let pending = RwSignal::new(Vec::new());
        merge_close_run_update(
            state,
            pending,
            close_run_with_id("close-1", CloseRunStatus::Submitted, 10),
        );
        let mut older = portfolio_snapshot("pos-2");
        older.recent_close_runs = vec![close_run_with_id("close-1", CloseRunStatus::Submitted, 20)];
        apply_snapshot_update(state, pending, older.clone());
        let gate = SnapshotRequestGate {
            version: RwSignal::new(1),
            token: 1,
        };
        apply_snapshot_result(state, pending, gate, Ok(portfolio_envelope(older)));
        let current = state.get_untracked();
        let run = &current.value().unwrap().recent_close_runs[0];
        assert_eq!(run.status, CloseRunStatus::Succeeded);
        assert_eq!(run.updated_at_ms, 30);
        merge_close_run_update(
            state,
            pending,
            close_run_with_id("close-1", CloseRunStatus::ManuallyResolved, 40),
        );
        assert_eq!(
            state.get_untracked().value().unwrap().recent_close_runs[0].status,
            CloseRunStatus::ManuallyResolved
        );
    });
}

#[test]
fn receipt_newer_than_snapshot_is_retained_but_old_evicted_history_is_not() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Ready(portfolio_snapshot("pos-1")));
        let pending = RwSignal::new(Vec::new());
        merge_close_run_update(
            state,
            pending,
            close_run_with_id("new", CloseRunStatus::Succeeded, 30),
        );
        merge_close_run_update(
            state,
            pending,
            close_run_with_id("old", CloseRunStatus::Succeeded, 10),
        );
        let mut incoming = portfolio_snapshot("pos-2");
        incoming.server_now_ms = 20;
        incoming.recent_close_runs.clear();
        apply_snapshot_update(state, pending, incoming);
        let current = state.get_untracked();
        assert_eq!(current.value().unwrap().recent_close_runs.len(), 1);
        assert_eq!(current.value().unwrap().recent_close_runs[0].id, "new");
    });
}

#[test]
fn degraded_snapshot_envelope_keeps_snapshot_as_stale() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Loading);
        let pending = RwSignal::new(Vec::new());
        let version = RwSignal::new(3);
        let gate = SnapshotRequestGate { version, token: 3 };
        let mut envelope = portfolio_envelope(portfolio_snapshot("pos-stale"));
        envelope.status = shared_types::PortfolioSnapshotStatus::Degraded;
        envelope.retry_after_ms = Some(2_000);

        apply_snapshot_result(state, pending, gate, Ok(envelope));

        let snapshot_state = state.get_untracked();
        assert!(matches!(snapshot_state, LoadState::Stale { .. }));
        if let LoadState::Stale { value, problem } = snapshot_state {
            assert_eq!(value.snapshot_version, "pos-stale");
            assert_eq!(
                problem.code,
                shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED
            );
            assert_eq!(problem.retry_after_ms, Some(2_000));
        }
    });
}

#[test]
fn degraded_raw_ws_snapshot_keeps_snapshot_as_stale() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Loading);
        let pending = RwSignal::new(Vec::new());
        let mut latest = portfolio_snapshot("pos-ws-degraded");
        latest.degraded = true;
        latest.summary.nav_evidence.problem = Some(ApiProblem::new(
            shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN,
            "wallet equity coverage incomplete",
        ));

        apply_snapshot_update(state, pending, latest);

        let snapshot_state = state.get_untracked();
        assert!(matches!(snapshot_state, LoadState::Stale { .. }));
        if let LoadState::Stale { value, problem } = snapshot_state {
            assert_eq!(value.snapshot_version, "pos-ws-degraded");
            assert_eq!(
                problem.code,
                shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN
            );
        }
    });
}
