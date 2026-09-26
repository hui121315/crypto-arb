use super::events::{
    first_leg_event, publish_order_event, refresh_order_status, valuation_failed_response,
};
use super::evidence::record_leg_orderbook_evidence;
use super::legs::{
    intent_for_role, quote_for_role, records_by_role, recovery_action_for_role, run_leg_mut,
};
use super::responses::second_leg_failed_response;
use super::run_model::{
    apply_leg_record, ledger_context, run_state_after_second, update_exposure, update_run,
};
use super::submission::ticket_submission_context;
use super::*;

pub(super) async fn submit_first_leg(
    state: &AppState,
    preview: &HedgePreviewResponse,
    run: &mut ExecutionRun,
    first_role: HedgeLegRole,
    engine: &ExecutionEngine,
) -> Result<OrderRecord, ApiProblem> {
    if let Err(error) = crate::services::hedge_preview::runtime::ensure_current(state, preview).await {
        run.state = ExecutionRunState::FailedSafe;
        update_run(state, run, "执行账户或环境已改变，未提交第一腿");
        return Err(ApiProblem::new(codes::HEDGE_EXECUTION_CONTEXT_CHANGED, error.to_string())
            .with_status(409).with_source("execution_orchestrator.execution_binding"));
    }
    let submission_context = match ticket_submission_context(preview, first_role) {
        Ok(context) => context,
        Err(problem) => {
            run.state = ExecutionRunState::FailedSafe;
            run.recovery_action = Some(RecoveryAction::ManualReview);
            update_run(state, run, "票据执行证据无效，未提交第一腿");
            return Err(*problem);
        }
    };
    let intent = intent_for_role(preview, first_role).clone();
    let quote = quote_for_role(&preview.ticket, first_role);
    match state
        .trading_service()
        .submit_with_ledger_context_on_engine(
            intent,
            ledger_context(run, first_role),
            submission_context,
            engine,
        )
        .await
    {
        Ok(record) => {
            record_leg_orderbook_evidence(state, &record, quote);
            publish_order_event(state, first_leg_event(&record), &record);
            Ok(refresh_order_status(state, record, "hedge_first_leg_status_backfilled", engine).await)
        }
        Err(error) => {
            run.state = ExecutionRunState::FailedSafe;
            run.recovery_action = Some(RecoveryAction::ManualReview);
            update_run(state, run, "第一腿提交失败");
            Err(crate::trading_errors::trading_error_problem(
                &error,
                &quote.exchange,
                "submit_order",
            ))
        }
    }
}

pub(super) async fn submit_second_leg(
    state: &AppState,
    preview: &HedgePreviewResponse,
    idempotency_key: String,
    mut run: ExecutionRun,
    first_record: OrderRecord,
    execution_order: HedgeExecutionOrder,
    engine: &ExecutionEngine,
) -> HedgeConfirmResponse {
    if let Err(error) = crate::services::hedge_preview::runtime::ensure_current(state, preview).await {
        return super::responses::recheck_blocked_response(
            state, idempotency_key, run, first_record, execution_order.first,
            error.to_string(), engine,
        ).await;
    }
    let submission_context = match ticket_submission_context(preview, execution_order.second) {
        Ok(context) => context,
        Err(problem) => {
            return second_leg_failed_response(
                state,
                idempotency_key,
                run,
                first_record,
                execution_order.first,
                *problem,
                engine,
            )
            .await;
        }
    };
    let second_intent = intent_for_role(preview, execution_order.second).clone();
    let second_quote = quote_for_role(&preview.ticket, execution_order.second);
    match state
        .trading_service()
        .submit_with_ledger_context_on_engine(
            second_intent,
            ledger_context(&run, execution_order.second),
            submission_context,
            engine,
        )
        .await
    {
        Ok(second_record) => {
            record_leg_orderbook_evidence(state, &second_record, second_quote);
            publish_order_event(state, "hedge_second_leg_submitted", &second_record);
            let second_record =
                refresh_order_status(state, second_record, "hedge_second_leg_status_backfilled", engine)
                    .await;
            if let Some(problem) = apply_leg_record(
                run_leg_mut(&mut run, execution_order.second),
                &second_record,
            ) {
                let (long_record, short_record) = records_by_role(
                    execution_order.first,
                    Some(first_record),
                    Some(second_record),
                );
                return valuation_failed_response(
                    state,
                    idempotency_key,
                    run,
                    long_record,
                    short_record,
                    problem,
                );
            }
            crate::services::execution_runs::append_order_update_evidence(&mut run, &second_record);
            refresh_cost_reconciliation(&mut run);
            run.state = run_state_after_second(&first_record, &second_record);
            if run.state == ExecutionRunState::UnwindRequired {
                run.recovery_action = Some(recovery_action_for_role(execution_order.first));
            }
            update_exposure(&mut run);
            update_run(state, &mut run, "双腿提交完成");
            let (long_record, short_record) = records_by_role(
                execution_order.first,
                Some(first_record),
                Some(second_record),
            );
            HedgeConfirmResponse {
                idempotency_key,
                status: HedgeConfirmStatus::Submitted,
                context: shared_types::HedgeConfirmContext::default(),
                execution_run: Some(run),
                long_record,
                short_record,
                unwind_record: None,
                problem: None,
                partial_outcome: None,
                error: None,
            }
        }
        Err(error) => {
            let problem = crate::trading_errors::trading_error_problem(
                &error,
                &second_quote.exchange,
                "submit_order",
            );
            second_leg_failed_response(
                state,
                idempotency_key,
                run,
                first_record,
                execution_order.first,
                problem,
                engine,
            )
            .await
        }
    }
}
