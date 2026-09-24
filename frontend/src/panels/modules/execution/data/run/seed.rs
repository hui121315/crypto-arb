//! 执行 run 的 seed/stream 应用逻辑：把 REST seed 与 WS 推送收敛成单一最新 run。
//!
//! 从 `run.rs` 拆出；上下文解析见 `context.rs`，订阅与兜底装配见 `run.rs`。
//! fail-closed：缺显式 run/ticket 命中时写 stale problem，绝不静默清空成空数据。

use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionRun, HedgeTicketView, ListEnvelope, ListStatus};

use super::context::ExecutionRunContext;

const EXECUTION_RUN_SEED_STALE: &str = "EXECUTION_RUN_SEED_STALE";
const EXECUTION_RUN_SEED_DEGRADED: &str = "EXECUTION_RUN_SEED_DEGRADED";
const EXECUTION_RUN_SEED_SOURCE: &str = "frontend-execution-run-seed";

pub(in crate::panels::modules::execution::data::run) fn apply_seed_result(
    run: RwSignal<Option<ExecutionRun>>,
    seed_problem: RwSignal<Option<ApiProblem>>,
    context: &ExecutionRunContext,
    result: Result<ListEnvelope<ExecutionRun>, ApiProblem>,
) -> Option<HedgeTicketView> {
    clear_mismatched_run(run, context);
    match result {
        Ok(envelope) => {
            let envelope_problem = seed_problem_from_envelope(&envelope);
            if let Some(next) = latest_run_for_context(envelope.rows, context) {
                let candidate = HedgeTicketView::from_execution_run(&next);
                seed_problem.set(envelope_problem);
                return apply_run_update(run, next, false).then_some(candidate);
            } else {
                seed_problem.set(missing_explicit_run_seed_problem(context).or(envelope_problem));
                clear_mismatched_run(run, context);
            }
        }
        Err(problem) => {
            clear_mismatched_run(run, context);
            seed_problem.set(Some(problem));
        }
    }
    None
}

fn seed_problem_from_envelope(envelope: &ListEnvelope<ExecutionRun>) -> Option<ApiProblem> {
    if envelope.status == ListStatus::Fresh && envelope.problems.is_empty() {
        return None;
    }
    let mut problem = envelope.problems.first().cloned().unwrap_or_else(|| {
        ApiProblem::new(
            EXECUTION_RUN_SEED_DEGRADED,
            "execution run seed envelope degraded without a backend problem",
        )
    });
    if problem.source.is_none() {
        problem.source = Some(envelope.source.clone());
    }
    let problem_details = problem.details.take();
    problem.details = Some(seed_envelope_problem_details(
        envelope,
        problem_details.as_ref(),
    ));
    Some(problem)
}

fn seed_envelope_problem_details(
    envelope: &ListEnvelope<ExecutionRun>,
    problem_details: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "envelopeStatus": envelope.status,
        "envelopeSource": envelope.source.clone(),
        "observedAtMs": envelope.observed_at_ms,
        "returnedCount": envelope.page.returned_count,
        "totalRows": envelope.page.total_rows,
        "problemCount": envelope.problems.len(),
        "problems": envelope.problems.clone(),
        "problemDetails": problem_details,
    })
}

fn latest_run_for_context(
    rows: Vec<ExecutionRun>,
    context: &ExecutionRunContext,
) -> Option<ExecutionRun> {
    rows.into_iter()
        .filter(|run| context.matches(run))
        .max_by_key(|run| run.updated_at_ms)
}

pub(in crate::panels::modules::execution::data) fn apply_run_update(
    run: RwSignal<Option<ExecutionRun>>,
    next: ExecutionRun,
    ordered_push: bool,
) -> bool {
    let mut applied = false;
    let mut next = Some(next);
    run.update(|current| {
        let should_replace = match current.as_ref() {
            Some(current) => next.as_ref().is_some_and(|next| {
                newer_run_should_replace(current, next)
                    && (ordered_push || next.updated_at_ms != current.updated_at_ms)
            }),
            None => true,
        };
        if should_replace {
            *current = next.take();
            applied = true;
        }
    });
    applied
}

fn clear_mismatched_run(run: RwSignal<Option<ExecutionRun>>, context: &ExecutionRunContext) {
    run.update(|current| {
        if current.as_ref().is_some_and(|row| !context.matches(row)) {
            *current = None;
        }
    });
}

fn newer_run_should_replace(current: &ExecutionRun, next: &ExecutionRun) -> bool {
    if current.run_id == next.run_id {
        next.updated_at_ms >= current.updated_at_ms
    } else {
        next.updated_at_ms > current.updated_at_ms
    }
}

fn missing_explicit_run_seed_problem(context: &ExecutionRunContext) -> Option<ApiProblem> {
    if context.run_id.is_none() && context.ticket_id.is_none() {
        return None;
    }
    let mut problem = ApiProblem::new(
        EXECUTION_RUN_SEED_STALE,
        missing_explicit_run_seed_message(context),
    )
    .with_source(EXECUTION_RUN_SEED_SOURCE);
    problem.details = Some(serde_json::json!({
        "runId": context.run_id.clone(),
        "ticketId": context.ticket_id.clone(),
        "opportunityId": context.opportunity_id.clone(),
        "idempotencyKey": context.idempotency_key.clone(),
    }));
    Some(problem)
}

fn missing_explicit_run_seed_message(context: &ExecutionRunContext) -> String {
    if let Some(run_id) = &context.run_id {
        return format!("execution run seed returned no row for run_id={run_id}");
    }
    if let Some(ticket_id) = &context.ticket_id {
        return format!("execution run seed returned no row for ticket_id={ticket_id}");
    }
    "execution run seed returned no row for explicit context".to_owned()
}

#[cfg(test)]
#[path = "seed_problem_tests.rs"]
mod problem_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionRunLeg, ExecutionRunState, HedgeLegRole, LiveOrderState};

    #[test]
    fn latest_run_for_context_filters_opportunity() {
        let context = ExecutionRunContext {
            opportunity_id: Some("opp-a".into()),
            ..ExecutionRunContext::default()
        };
        let latest = latest_run_for_context(
            vec![
                ExecutionRun {
                    opportunity_id: "opp-a".into(),
                    ..run("old-match", 1)
                },
                ExecutionRun {
                    opportunity_id: "opp-b".into(),
                    ..run("new-wrong", 9)
                },
                ExecutionRun {
                    opportunity_id: "opp-a".into(),
                    ..run("new-match", 3)
                },
            ],
            &context,
        );

        assert_eq!(latest.map(|run| run.run_id), Some("new-match".into()));
    }

    #[test]
    fn newer_run_update_rejects_older_same_run() {
        assert!(!newer_run_should_replace(&run("same", 3), &run("same", 2)));
        assert!(newer_run_should_replace(&run("same", 3), &run("same", 3)));
        assert!(newer_run_should_replace(&run("same", 3), &run("same", 4)));
        assert!(!newer_run_should_replace(
            &run("current", 3),
            &run("other", 2)
        ));
    }

    #[test]
    fn http_tie_cannot_overwrite_ws_terminal_but_next_ws_can_correct_it() {
        Owner::new().with(|| {
            let mut current = run("same", 10);
            current.state = ExecutionRunState::Hedged;
            let signal = RwSignal::new(Some(current));
            assert!(!apply_run_update(signal, run("same", 10), false));
            assert_eq!(
                signal.get_untracked().unwrap().state,
                ExecutionRunState::Hedged
            );
            assert!(apply_run_update(signal, run("same", 10), true));
        });
    }

    #[test]
    fn empty_seed_for_explicit_run_context_is_stale_problem() {
        let context = ExecutionRunContext {
            run_id: Some("run-a".into()),
            ticket_id: Some("ticket-a".into()),
            ..ExecutionRunContext::default()
        };

        let problem = missing_explicit_run_seed_problem(&context);

        assert_eq!(
            problem.as_ref().map(|problem| problem.code.as_str()),
            Some(EXECUTION_RUN_SEED_STALE)
        );
        assert!(problem
            .as_ref()
            .is_some_and(|problem| problem.message.contains("run_id=run-a")));
        assert_eq!(
            problem
                .as_ref()
                .and_then(|problem| problem.details.as_ref())
                .and_then(|details| details.get("runId"))
                .and_then(serde_json::Value::as_str),
            Some("run-a")
        );
    }

    #[test]
    fn empty_seed_for_explicit_ticket_context_is_stale_problem() {
        let context = ExecutionRunContext {
            ticket_id: Some("ticket-a".into()),
            ..ExecutionRunContext::default()
        };

        let problem = missing_explicit_run_seed_problem(&context);

        assert_eq!(
            problem.as_ref().map(|problem| problem.code.as_str()),
            Some(EXECUTION_RUN_SEED_STALE)
        );
        assert!(problem
            .as_ref()
            .is_some_and(|problem| problem.message.contains("ticket_id=ticket-a")));
    }

    #[test]
    fn empty_seed_for_opportunity_only_context_stays_empty_not_stale() {
        let context = ExecutionRunContext {
            opportunity_id: Some("opp-a".into()),
            ..ExecutionRunContext::default()
        };

        assert!(missing_explicit_run_seed_problem(&context).is_none());
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
}
