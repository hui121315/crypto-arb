use super::*;
use shared_types::{ListEnvelope, ListPage};

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
fn fresh_empty_seed_confirms_missing_local_ticket_restore() {
    let context = ExecutionRunContext {
        ticket_id: Some("ticket-old".into()),
        idempotency_key: Some("idem-old".into()),
        restored_without_selection: true,
        ..ExecutionRunContext::default()
    };
    let result = fresh_empty_seed();

    assert!(missing_local_restore_confirmed(&context, &result));
}

#[test]
fn fresh_empty_seed_confirms_missing_local_run_restore_but_not_explicit_route() {
    let result = fresh_empty_seed();
    let local = ExecutionRunContext {
        run_id: Some("run-old".into()),
        idempotency_key: Some("idem-old".into()),
        restored_without_selection: true,
        ..ExecutionRunContext::default()
    };
    let route = ExecutionRunContext {
        run_id: Some("run-explicit".into()),
        restored_without_selection: true,
        ..ExecutionRunContext::default()
    };

    assert!(missing_local_restore_confirmed(&local, &result));
    assert!(!missing_local_restore_confirmed(&route, &result));
}

#[test]
fn degraded_seed_does_not_discard_local_ticket_restore() {
    let context = ExecutionRunContext {
        ticket_id: Some("ticket-old".into()),
        idempotency_key: Some("idem-old".into()),
        restored_without_selection: true,
        ..ExecutionRunContext::default()
    };
    let result = Ok(ListEnvelope::new(
        Vec::new(),
        ListPage::default(),
        ListStatus::Degraded,
        "execution_run_query",
        10,
        Vec::new(),
    ));

    assert!(!missing_local_restore_confirmed(&context, &result));
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
