//! 执行 run 的补救动作：撤单（撤销未成交腿订单）与平仓交接语义。
//!
//! “取消”只重置本地提交状态；真正的撤单走这里的 hook 调用
//! `/api/trading/orders/:id/cancel`；平仓不在执行页复刻高风险流程，
//! 而是交接到持仓模块（快照版本 fail-closed 流程在那里）。

use crate::api::rest::{with_mutation_timeout, ApiClient, ApiError, MutationRequestContext};
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ActionEvidence, ApiProblem, ExecutionRun, ExecutionRunLeg, ExecutionRunState, LiveOrderState,
    OrderRecord,
};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct CancelRunOrdersAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<ExecutionRun>,
}

pub(in crate::panels::modules::execution) fn use_cancel_run_orders_action(
    refresh_nonce: RwSignal<u64>,
    state: RwSignal<ActionState>,
    queue: RwSignal<super::orders::OrderQueue>,
    orders: Memo<Vec<OrderRecord>>,
) -> CancelRunOrdersAction {
    let client = use_global().client;
    Effect::new(move |_| {
        let rows = orders.get();
        let current = state.get_untracked();
        if !matches!(
            current,
            ActionState::Accepted { .. } | ActionState::Failed { .. }
        ) {
            return;
        }
        let Some(evidence) = current.evidence() else {
            return;
        };
        if let Some(resolved) = settled_cancel_state(evidence, &rows) {
            if resolved != current {
                state.set(resolved);
            }
        }
    });
    let submit = Callback::new(move |run: ExecutionRun| {
        if matches!(
            state.get_untracked(),
            ActionState::Pending { .. } | ActionState::Accepted { .. }
        ) {
            return;
        }
        let order_ids = cancelable_order_ids_with_records(&run, &orders.get_untracked());
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
        let mut pending_evidence = requests.iter().fold(
            ActionEvidence::from_execution_run(&run),
            |mut evidence, (_, context)| {
                evidence.merge(context.evidence());
                evidence
            },
        );
        pending_evidence.order_ids.clone_from(&order_ids);
        state.set(ActionState::pending("撤单提交中").with_evidence(pending_evidence.clone()));
        let client = client.clone();
        spawn_local(async move {
            let (records, failures) = cancel_orders_task(client, &requests).await;
            if state.try_get_untracked().is_none() {
                return;
            }
            queue.update(|queue| {
                for record in &records {
                    queue.apply_receipt(record.clone());
                }
            });
            refresh_nonce.update(|value| *value = value.wrapping_add(1));
            let latest = queue.with_untracked(|queue| {
                pending_evidence
                    .order_ids
                    .iter()
                    .filter_map(|id| queue.order(id).cloned())
                    .collect::<Vec<_>>()
            });
            if let Some(resolved) = settled_cancel_state(&pending_evidence, &latest) {
                state.set(resolved);
                return;
            }
            if let Some((order_id, error)) = failures.first() {
                let mut problem = error
                    .problem
                    .clone()
                    .with_source(format!("execution.cancel_order:{order_id}"));
                problem.details = Some(
                    serde_json::json!({ "failures": failures.iter().map(|(id, error)|
                    serde_json::json!({ "orderId": id, "problem": error.problem })
                ).collect::<Vec<_>>() }),
                );
                state.set(
                    ActionState::failed(
                        format!(
                            "撤单反馈：{} 笔收到回执，{} 笔失败或待核验",
                            records.len(),
                            failures.len(),
                        ),
                        problem,
                    )
                    .with_evidence(pending_evidence),
                );
            } else {
                state.set(
                    ActionState::accepted(format!("撤单请求已受理，{} 笔等待终态", records.len()))
                        .with_evidence(pending_evidence),
                );
            }
        });
    });
    CancelRunOrdersAction { state, submit }
}

async fn cancel_orders_task(
    client: ApiClient,
    requests: &[(String, MutationRequestContext)],
) -> (Vec<OrderRecord>, Vec<(String, ApiError)>) {
    let mut records = Vec::new();
    let mut failures = Vec::new();
    for (order_id, context) in requests {
        match with_mutation_timeout("撤单", client.cancel_order_with_context(order_id, context))
            .await
        {
            Ok(record) if record.intent.id == *order_id => records.push(record),
            Ok(_) => failures.push((
                order_id.clone(),
                ApiError::from_problem(ApiProblem::new(
                    "CANCEL_RECEIPT_MISMATCH",
                    "撤单回执不属于原订单，等待核验",
                )),
            )),
            Err(error) => failures.push((order_id.clone(), error)),
        }
    }
    (records, failures)
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
            for id in &leg.order_ids {
                if !id.is_empty() && !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
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
    ) || leg_has_fill(&run.long_leg)
        || leg_has_fill(&run.short_leg)
}

fn terminal_order(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Cancelled | LiveOrderState::Filled | LiveOrderState::Rejected
    )
}

fn settled_cancel_state(evidence: &ActionEvidence, rows: &[OrderRecord]) -> Option<ActionState> {
    if evidence.order_ids.is_empty() {
        return None;
    }
    let mut cancelled = 0;
    let mut filled = 0;
    let mut rejected = 0;
    for id in &evidence.order_ids {
        let row = rows.iter().find(|row| row.intent.id == *id)?;
        if !terminal_order(row.state) {
            return None;
        }
        if row.state == LiveOrderState::Cancelled {
            cancelled += 1;
        }
        if row.state == LiveOrderState::Rejected {
            rejected += 1;
        }
        if row.state == LiveOrderState::Filled || row.filled_quantity.is_some_and(|qty| qty > 0.0) {
            filled += 1;
        }
    }
    let label = format!("撤单结果已核对：{cancelled} 笔撤销，{rejected} 笔原单拒绝");
    Some(
        if filled > 0 {
            ActionState::failed(
                format!("{label}；{filled} 笔已有成交"),
                ApiProblem::new(
                    "CANCEL_ORDER_HAS_FILLS",
                    "已有成交，撤单不等于平仓，请核对持仓",
                )
                .with_source("execution.cancel_order.finality"),
            )
        } else {
            ActionState::succeeded(label)
        }
        .with_evidence(evidence.clone()),
    )
}

pub(in crate::panels::modules::execution) fn cancelable_order_ids_with_records(
    run: &ExecutionRun,
    rows: &[OrderRecord],
) -> Vec<String> {
    cancelable_order_ids(run)
        .into_iter()
        .filter(|id| {
            !rows.iter().any(|row| {
                row.intent.id == *id
                    && (terminal_order(row.state) || row.state == LiveOrderState::CancelRequested)
            })
        })
        .collect()
}

pub(in crate::panels::modules::execution) fn run_orders_have_fill(
    run: &ExecutionRun,
    rows: &[OrderRecord],
) -> bool {
    run.state != ExecutionRunState::Closed
        && rows.iter().any(|row| {
            (run.long_leg.order_ids.contains(&row.intent.id)
                || run.short_leg.order_ids.contains(&row.intent.id))
                && (matches!(
                    row.state,
                    LiveOrderState::Filled | LiveOrderState::PartiallyFilled
                ) || row.filled_quantity.is_some_and(|qty| qty > 0.0))
        })
}

pub(in crate::panels::modules::execution) fn run_is_released(run: &ExecutionRun) -> bool {
    if run.state == ExecutionRunState::Closed {
        return true;
    }
    run.state == ExecutionRunState::FailedSafe
        && run.net_exposure_usd == 0.0
        && run.finality_problem.is_none()
        && run.unwind_problem.is_none()
        && run.valuation_problem.is_none()
        && [&run.long_leg, &run.short_leg].iter().all(|leg| {
            (leg.state == LiveOrderState::Created && leg.order_ids.is_empty())
                || (matches!(
                    leg.state,
                    LiveOrderState::Cancelled | LiveOrderState::Rejected
                ) && leg.filled_quantity == Some(0.0))
        })
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
