//! 执行 run 的补救动作：撤单（撤销未成交腿订单）与平仓交接语义。
//!
//! “取消”只重置本地提交状态；真正的撤单走这里的 hook 调用
//! `/api/trading/orders/:id/cancel`；平仓不在执行页复刻高风险流程，
//! 而是交接到持仓模块（快照版本 fail-closed 流程在那里）。

use crate::api::rest::with_mutation_timeout;
use crate::state::action_state::ActionState;
use crate::panels::modules::execution_orders::{leg_contains_order, leg_order_ids};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ActionEvidence, ApiProblem, ExecutionRun, ExecutionRunLeg, ExecutionRunState, LiveOrderState,
    OrderRecord,
};

#[path = "remedy/recovery.rs"]
mod recovery;
pub(in crate::panels::modules::execution) use recovery::CancelRecovery;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct CancelRunOrdersAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<ExecutionRun>,
    pub recovery: CancelRecovery,
}

pub(in crate::panels::modules::execution) fn use_cancel_run_orders_action(
    refresh_nonce: RwSignal<u64>,
    recovery: CancelRecovery,
    queue: RwSignal<super::orders::OrderQueue>,
    orders: Memo<Vec<OrderRecord>>,
) -> CancelRunOrdersAction {
    let state = recovery.state;
    Effect::new(move |_| {
        let rows = orders.get();
        if recovery.blocked() {
            recovery.accept(&rows);
            return;
        }
        let current = state.get_untracked();
        if current
            .problem()
            .is_some_and(|problem| matches!(problem.code.as_str(), "CANCEL_BATCH_INTERRUPTED" | shared_types::problem::codes::ORDER_ACCOUNT_MISMATCH))
        {
            return;
        }
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
        if recovery.blocked()
            || !recovery.current()
            || matches!(
                state.get_untracked(),
                ActionState::Pending { .. } | ActionState::Accepted { .. }
            )
        {
            return;
        }
        let order_ids = cancelable_order_ids_with_records(&run, &orders.get_untracked());
        if order_ids.is_empty() {
            state.set(ActionState::failed(
                "暂不能撤单",
                ApiProblem::new(
                    "EXECUTION_RUN_NO_CANCELABLE_ORDERS",
                    "当前 run 没有可撤销的未成交腿订单",
                ),
            ));
            return;
        }
        let Some(mut batch) = recovery.begin(&run, &order_ids) else {
            return;
        };
        let client = recovery.client();
        spawn_local(async move {
            let mut failures = Vec::new();
            for index in 0..batch.requests.len() {
                if !recovery.mark_sent(&mut batch, index) {
                    break;
                }
                let request = &batch.requests[index];
                let result = with_mutation_timeout(
                    "撤单",
                    client.cancel_order_with_context(&request.order_id, &request.context),
                )
                .await;
                if !recovery.current() {
                    break;
                }
                match result {
                    Ok(record) if request.matches(&record) => {
                        queue.update(|queue| queue.apply_receipt(record))
                    }
                    Ok(_) => failures.push("撤单处理结果身份不匹配".to_owned()),
                    Err(error) if error.problem.code == shared_types::problem::codes::ORDER_ACCOUNT_MISMATCH => {
                        recovery.account_rejected(&mut batch, index);
                        failures.push(error.problem.message);
                        break;
                    }
                    Err(error) => failures.push(error.problem.message),
                }
            }
            if state.try_get_untracked().is_none() {
                return;
            }
            recovery.busy.set(false);
            if !recovery.current() {
                return;
            }
            refresh_nonce.update(|value| *value = value.wrapping_add(1));
            let latest = queue.with_untracked(|queue| {
                batch
                    .requests
                    .iter()
                    .filter_map(|request| queue.order(&request.order_id).cloned())
                    .collect::<Vec<_>>()
            });
            if recovery.accept(&latest) {
                return;
            }
            recovery.unknown(if failures.is_empty() {
                "撤单请求已受理，等待订单最终结果与成交量；不会再次发送。".into()
            } else {
                format!("{} 笔反馈待核对：{}", failures.len(), failures.join("；"))
            });
        });
    });
    CancelRunOrdersAction {
        state,
        submit,
        recovery,
    }
}

/// 双腿里仍可撤销（已到 venue 但未终态）的订单 ID。
pub(in crate::panels::modules::execution) fn cancelable_order_ids(
    run: &ExecutionRun,
) -> Vec<String> {
    let mut ids = Vec::new();
    for leg in [&run.long_leg, &run.short_leg] {
        if leg_orders_cancelable(leg) {
            for id in leg_order_ids(leg, &[]) {
                if !ids.contains(&id) {
                    ids.push(id);
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
        // A terminal status alone does not establish that cancellation left no fill.
        let quantity = row
            .filled_quantity
            .filter(|quantity| quantity.is_finite() && *quantity >= 0.0)?;
        if row.state == LiveOrderState::Filled && quantity == 0.0 {
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
    let mut ids = Vec::new();
    for leg in [&run.long_leg, &run.short_leg] {
        for id in leg_order_ids(leg, rows) {
            let record = rows.iter().find(|row| row.intent.id == id && leg_contains_order(leg, row));
            let cancelable = record.map_or_else(|| leg_orders_cancelable(leg), |row| matches!(
                row.state, LiveOrderState::Submitted | LiveOrderState::Accepted | LiveOrderState::PartiallyFilled
            ));
            if cancelable && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

pub(in crate::panels::modules::execution) fn run_orders_have_fill(
    run: &ExecutionRun,
    rows: &[OrderRecord],
) -> bool {
    run.state != ExecutionRunState::Closed
        && rows.iter().any(|row| {
            (leg_contains_order(&run.long_leg, row)
                || leg_contains_order(&run.short_leg, row))
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
