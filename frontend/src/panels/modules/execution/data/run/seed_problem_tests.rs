use super::*;
use shared_types::{ExecutionRunLeg, ExecutionRunState, HedgeLegRole, ListPage, LiveOrderState};

#[test]
fn execution_run_seed_error_clears_mismatched_run() {
    Owner::new().with(|| {
        let run_signal = RwSignal::new(Some(ExecutionRun {
            opportunity_id: "opp-a".into(),
            ..run("run-a", 3)
        }));
        let seed_problem = RwSignal::new(None);
        let context = ExecutionRunContext {
            opportunity_id: Some("opp-b".into()),
            ..ExecutionRunContext::default()
        };

        apply_seed_result(
            run_signal,
            seed_problem,
            &context,
            Err(ApiProblem::new("NETWORK", "seed failed")),
        );

        assert!(run_signal.get_untracked().is_none());
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
fn execution_run_seed_error_keeps_same_context_stale_run() {
    Owner::new().with(|| {
        let run_signal = RwSignal::new(Some(ExecutionRun {
            opportunity_id: "opp-a".into(),
            ..run("run-a", 3)
        }));
        let seed_problem = RwSignal::new(None);
        let context = ExecutionRunContext {
            opportunity_id: Some("opp-a".into()),
            ..ExecutionRunContext::default()
        };

        apply_seed_result(
            run_signal,
            seed_problem,
            &context,
            Err(ApiProblem::new("NETWORK", "seed failed")),
        );

        assert_eq!(
            run_signal
                .get_untracked()
                .as_ref()
                .map(|run| run.run_id.as_str()),
            Some("run-a")
        );
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
fn execution_run_seed_degraded_empty_envelope_surfaces_problem() {
    let problem = seed_problem_from_envelope(&envelope_with(
        vec![ExecutionRun {
            opportunity_id: "opp-a".into(),
            ..run("run-a", 3)
        }],
        ListStatus::Degraded,
        Vec::new(),
    ));

    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some(EXECUTION_RUN_SEED_DEGRADED)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.source.as_deref()),
        Some("execution_run_query")
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("envelopeStatus"))
            .and_then(serde_json::Value::as_str),
        Some("degraded")
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("observedAtMs"))
            .and_then(serde_json::Value::as_i64),
        Some(10)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("returnedCount"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("totalRows"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[test]
fn execution_run_seed_degraded_backend_problem_keeps_request_and_retry() {
    let backend_problem = ApiProblem::new("UPSTREAM_LIMIT", "limited")
        .with_request_id(Some("req-run-seed-1".into()))
        .with_retry_after_ms(Some(2_000));

    let problem = seed_problem_from_envelope(&envelope_with(
        vec![ExecutionRun {
            opportunity_id: "opp-a".into(),
            ..run("run-a", 3)
        }],
        ListStatus::Degraded,
        vec![backend_problem],
    ));

    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some("UPSTREAM_LIMIT")
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.request_id.as_deref()),
        Some("req-run-seed-1")
    );
    assert_eq!(
        problem.as_ref().and_then(|problem| problem.retry_after_ms),
        Some(2_000)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("problemCount"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

fn run(id: &str, updated_at_ms: i64) -> ExecutionRun {
    ExecutionRun {
        run_id: id.to_owned(),
        ticket_id: format!("ticket-{id}"),
        opportunity_id: format!("opp-{id}"),
        state: ExecutionRunState::Previewed,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "seed".to_owned(),
        created_at_ms: updated_at_ms,
        updated_at_ms,
    }
}

fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Created,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

fn envelope_with(
    rows: Vec<ExecutionRun>,
    status: ListStatus,
    problems: Vec<ApiProblem>,
) -> ListEnvelope<ExecutionRun> {
    let returned_count = rows.len();
    ListEnvelope::new(
        rows,
        ListPage {
            limit: 50,
            max_limit: 100,
            start_offset: 0,
            returned_count,
            total_rows: returned_count,
            has_more: false,
            next_cursor: None,
            ..ListPage::default()
        },
        status,
        "execution_run_query",
        10,
        problems,
    )
}
