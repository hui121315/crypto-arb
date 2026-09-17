//! 执行 run 的补救动作：撤单（撤销未成交腿订单）与平仓交接语义。
//!
//! “取消”只重置本地提交状态；真正的撤单走这里的 hook 调用
//! `/api/trading/orders/:id/cancel`；平仓不在执行页复刻高风险流程，
//! 而是交接到持仓模块（快照版本 fail-closed 流程在那里）。

use crate::api::rest::{ApiClient, ApiError, MutationRequestContext};
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ActionEvidence, ApiProblem, ExecutionRun, ExecutionRunLeg, ExecutionRunState, LiveOrderState,
};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct CancelRunOrdersAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<ExecutionRun>,
}

pub(in crate::panels::modules::execution) fn use_cancel_run_orders_action(
    refresh_nonce: RwSignal<u64>,
) -> CancelRunOrdersAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    let submit = Callback::new(move |run: ExecutionRun| {
        if state.get_untracked().is_pending() {
            return;
        }
        let order_ids = cancelable_order_ids(&run);
        if order_ids.is_empty() {
            state.set(ActionState::failed(
                "撤单阻断",
                ApiProblem::new(
                    "EXECUTION_RUN_NO_CANCELABLE_ORDERS",
                    "当前 run 没有可撤销的未成交腿订单",
                ),
            ));
            return;
        }
        let requests = cancel_request_contexts(&run, &order_ids);
        let pending_evidence = requests.iter().fold(
            ActionEvidence::from_execution_run(&run),
            |mut evidence, (_, context)| {
                evidence.merge(context.evidence());
                evidence
            },
        );
        state.set(ActionState::pending("撤单提交中").with_evidence(pending_evidence.clone()));
        let client = client.clone();
        spawn_local(async move {
            match cancel_orders_task(client, &requests).await {
                Ok(cancelled) => {
                    refresh_nonce.update(|value| *value = value.wrapping_add(1));
                    state.set(
                        ActionState::succeeded(format!(
                            "已提交撤单 {} 笔 · Run {}",
                            cancelled, run.run_id
                        ))
                        .with_evidence(pending_evidence),
                    );
                }
                Err((order_id, error)) => {
                    state.set(
                        ActionState::failed(
                            "撤单失败",
                            error
                                .problem
                                .with_source(format!("execution.cancel_order:{order_id}")),
                        )
                        .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    CancelRunOrdersAction { state, submit }
}

async fn cancel_orders_task(
    client: ApiClient,
    requests: &[(String, MutationRequestContext)],
) -> Result<usize, (String, ApiError)> {
    let mut cancelled = 0usize;
    for (order_id, context) in requests {
        client
            .cancel_order_with_context(order_id, context)
            .await
            .map_err(|error| (order_id.clone(), error))?;
        cancelled += 1;
    }
    Ok(cancelled)
}

fn cancel_request_contexts(
    run: &ExecutionRun,
    order_ids: &[String],
) -> Vec<(String, MutationRequestContext)> {
    order_ids
        .iter()
        .map(|order_id| {
            (
                order_id.clone(),
                MutationRequestContext::new_idempotent_attempt(format!(
                    "execution-cancel:{}:{order_id}",
                    run.run_id
                )),
            )
        })
        .collect()
}

/// 双腿里仍可撤销（已到 venue 但未终态）的订单 ID。
pub(in crate::panels::modules::execution) fn cancelable_order_ids(
    run: &ExecutionRun,
) -> Vec<String> {
    let mut ids = Vec::new();
    for leg in [&run.long_leg, &run.short_leg] {
        if leg_orders_cancelable(leg) {
            ids.extend(leg.order_ids.iter().cloned());
        }
    }
    ids
}

fn leg_orders_cancelable(leg: &ExecutionRunLeg) -> bool {
    matches!(
        leg.state,
        LiveOrderState::Submitted | LiveOrderState::Accepted | LiveOrderState::PartiallyFilled
    ) && !leg.order_ids.is_empty()
}

/// run 是否已有成交敞口，需要去持仓模块平仓（而不是撤单）。
pub(in crate::panels::modules::execution) fn run_needs_position_close(run: &ExecutionRun) -> bool {
    if run.state == ExecutionRunState::Closed {
        return false;
    }
    matches!(
        run.state,
        ExecutionRunState::Hedged
            | ExecutionRunState::UnwindRequired
            | ExecutionRunState::Unwinding
            | ExecutionRunState::FailedSafe
    ) || leg_has_fill(&run.long_leg)
        || leg_has_fill(&run.short_leg)
}

fn leg_has_fill(leg: &ExecutionRunLeg) -> bool {
    matches!(
        leg.state,
        LiveOrderState::Filled | LiveOrderState::PartiallyFilled
    ) || leg.filled_quantity.is_some_and(|qty| qty > 0.0)
}

#[cfg(test)]
#[path = "remedy/tests.rs"]
mod tests;
