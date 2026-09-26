#[path = "data/actions.rs"]
mod actions;
#[path = "data/artifact.rs"]
mod artifact;
#[path = "data/connection.rs"]
mod connection;
#[path = "data/orders.rs"]
mod orders;
#[path = "data/outcome.rs"]
mod outcome;
#[path = "data/preview.rs"]
mod preview;
#[path = "data/remedy.rs"]
mod remedy;
#[path = "data/run.rs"]
mod run;
#[path = "data/runtime.rs"]
mod runtime;
#[path = "data/submission.rs"]
mod submission;
#[path = "data/workflow.rs"]
mod workflow;

pub(super) use actions::{
    use_confirm_hedge_action, ConfirmHedgeAction, ConfirmHedgeRequest, ConfirmHedgeSeed,
};
pub(super) use artifact::{
    artifact_is_ready, artifact_valid_until, artifact_validation_is_ready, use_execution_artifact,
    ExecutionArtifactRuntime,
};
pub(super) use orders::{
    all_orders_memo, order_seed_problem_memo, order_seed_ready_memo, order_stream_problem_memo, orders_for_run_memo,
    use_order_queue, use_run_order_details, OrderDetails, OrderQueue,
};
pub(super) use outcome::{confirm_context_detail, confirm_outcome_detail, confirm_outcome_summary};
pub(super) use preview::{
    default_capital_text, default_leverage_text, default_limit_offset_text,
    quantity_from_notional_text, use_preview, ExecutionPreview, PreviewDepth,
    PreviewFundingWindowEvidence, PreviewOneCycleCost, PreviewReadiness, PreviewSignals,
    TicketClock,
};
#[cfg(test)]
pub(super) use preview::{PreviewLiquidation, PreviewProfitEvidence, PreviewRisk};
pub(super) use remedy::{
    cancelable_order_ids, cancelable_order_ids_with_records, run_is_released,
    run_needs_position_close, run_orders_have_fill, use_cancel_run_orders_action,
    CancelRunOrdersAction,
};
pub(super) use remedy::CancelRecovery;
pub(super) use run::use_execution_run_updates;
pub(in crate::panels) use runtime::create_execution_runtime;
pub(super) use runtime::ConfirmActionRuntime;
pub(in crate::panels) use runtime::ExecutionRuntime;
pub(super) use submission::SubmissionRecovery;
pub(super) use connection::ExecutionConnection;
pub(super) use workflow::WorkflowViewSource;

// Preview/build/validate never submit orders. Their wait limit must not be used for order writes.
pub(super) async fn await_execution_check<T>(
    operation: &str,
    request: impl std::future::Future<Output = Result<T, crate::api::rest::ApiError>>,
) -> Result<T, shared_types::ApiProblem> {
    crate::state::read_scope::bounded_read(request).await.map_err(|problem| {
        if problem.code == "SHARED_READ_TIMEOUT" {
            shared_types::ApiProblem::new(
                "EXECUTION_CHECK_TIMEOUT",
                format!("{operation}超过 15 秒未返回，已停止等待；本次检查不会下单，请重试"),
            ).with_source("execution.check")
        } else {
            problem
        }
    })
}
