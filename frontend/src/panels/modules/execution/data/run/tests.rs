use super::*;
use shared_types::{ListEnvelope, ListPage, ListStatus};

#[test]
fn execution_run_stream_accepts_fill_and_close_projections() {
    assert!(is_execution_run_update_event("execution_run_updated"));
    assert!(is_execution_run_update_event("private_ws_fill_event"));
    assert!(is_execution_run_update_event("execution_run_closed"));
    assert!(!is_execution_run_update_event("private_ws_order_update"));
}

#[test]
fn execution_stream_ignores_updates_without_explicit_context() {
    assert!(explicit_stream_context(ExecutionRunContext::default()).is_none());
}

#[test]
fn disposed_request_version_is_never_current() {
    assert!(request_token_is_latest(Some(2), 2));
    assert!(!request_token_is_latest(Some(2), 1));
    assert!(!request_token_is_latest(None, 2));
}

#[test]
fn execution_run_fallback_context_ignores_stale_result() {
    Owner::new().with(|| {
        let run = RwSignal::new(None);
        let seed_problem = RwSignal::new(None);
        let requested_context = ExecutionRunContext {
            opportunity_id: Some("opp-a".into()),
            ..ExecutionRunContext::default()
        };
        let current_context = ExecutionRunContext {
            opportunity_id: Some("opp-b".into()),
            ..ExecutionRunContext::default()
        };

        assert!(apply_fallback_result(
            run,
            seed_problem,
            &requested_context,
            &current_context,
            Err(ApiProblem::new("OLD_REQUEST", "stale fallback")),
        )
        .is_none());
        assert!(run.get_untracked().is_none());
        assert!(seed_problem.get_untracked().is_none());
    });
}

#[test]
fn execution_run_fallback_context_applies_current_error() {
    Owner::new().with(|| {
        let run = RwSignal::new(None);
        let seed_problem = RwSignal::new(None);
        let context = ExecutionRunContext {
            opportunity_id: Some("opp-a".into()),
            ..ExecutionRunContext::default()
        };

        assert!(apply_fallback_result(
            run,
            seed_problem,
            &context,
            &context,
            Err(ApiProblem::new("NETWORK", "fallback failed")),
        )
        .is_none());
        assert_eq!(
            seed_problem
                .get_untracked()
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some("NETWORK")
        );
    });
}

#[test]
fn fresh_empty_seed_preserves_pending_ticket_and_reports_missing_evidence() {
    let context = ExecutionRunContext {
        ticket_id: Some("ticket-old".into()),
        idempotency_key: Some("idem-old".into()),
        restored_without_selection: true,
        ..ExecutionRunContext::default()
    };
    Owner::new().with(|| {
        let run = RwSignal::new(None);
        let problem = RwSignal::new(None);
        assert!(apply_seed_result(run, problem, &context, fresh_empty_seed()).is_none());
        assert_eq!(
            problem.get_untracked().unwrap().code,
            "EXECUTION_RUN_SEED_STALE"
        );
        assert_eq!(context.ticket_id.as_deref(), Some("ticket-old"));
    });
}

fn fresh_empty_seed() -> Result<ListEnvelope<ExecutionRun>, ApiProblem> {
    Ok(ListEnvelope::new(
        Vec::new(),
        ListPage::default(),
        ListStatus::Fresh,
        "execution_run_query",
        10,
        Vec::new(),
    ))
}
