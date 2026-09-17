//! Positions 提交类动作 hooks（平仓 / 一键清仓 / 补偿 / kill-switch）。
//!
//! 每个 hook 落 `ActionState`、带幂等键、fail-closed（缺证据不下单）。订单反馈
//! 与账户快照分别由 `AppWS` 推进；请求构造与网络任务见 [`super::requests`]，
//! close-run 状态/文案派生见 [`super::runs`]。

use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ActionRunKind, PortfolioSnapshot, PositionRow, TradingStatusResponse};
use std::collections::BTreeMap;

use super::requests::*;
use super::runs::*;

#[path = "actions/context.rs"]
mod context;
#[cfg(test)]
pub(in crate::panels::modules::positions) use context::close_all_request_context;
use context::{
    close_all_attempt_anchor, position_attempt_anchor, remember_position_attempt_anchor,
};
pub(in crate::panels::modules::positions) use context::{
    close_all_request_context_with_attempt, close_request_context,
    close_request_context_with_attempt, portfolio_scope_evidence, position_scope_evidence,
};

#[path = "actions/compensation.rs"]
mod compensation;
pub(in crate::panels::modules::positions) use compensation::{
    use_close_run_compensation_action, CloseRunCompensationAction, CloseRunCompensationCancelInput,
    CloseRunCompensationInput, CloseRunManualTerminalInput,
};

#[path = "actions/kill_switch.rs"]
mod kill_switch;
pub(in crate::panels::modules::positions) use kill_switch::{
    use_positions_kill_switch_action, PositionsKillSwitchAction,
};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PositionCloseAction {
    pub state: RwSignal<ActionState>,
    pub active_key: RwSignal<Option<String>>,
    pub close_one: Callback<PositionRow>,
    pub close_pair: Callback<PositionRow>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct CloseAllPositionsAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<String>,
}

pub(in crate::panels::modules::positions) fn use_position_close_action(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
    trading_status: RwSignal<LoadState<TradingStatusResponse>>,
) -> PositionCloseAction {
    let client = use_global().client;
    let pair_client = client.clone();
    let state = RwSignal::new(ActionState::Idle);
    let active_key = RwSignal::new(None::<String>);
    let attempt_anchors = RwSignal::new(BTreeMap::<String, (String, i64)>::new());
    recover_position_close_state(state, snapshot_state);
    let close_one = Callback::new(move |row: PositionRow| {
        if state.get_untracked().is_pending() {
            return;
        }
        let execution_scope = match close_execution_scope(
            trading_status,
            row.origin == shared_types::PositionOrigin::AccountPrivate,
        ) {
            Ok(scope) => scope,
            Err(problem) => {
                state.set(ActionState::failed("平仓失败", *problem));
                return;
            }
        };
        let request = match close_position_request(snapshot_state, &row, 1, "positions.close_one") {
            Ok(request) => request,
            Err(problem) => {
                state.set(ActionState::failed("平仓失败", *problem));
                return;
            }
        };
        let key = position_key(&row);
        let label = close_label(&row);
        let target = format!("{key}:{execution_scope}");
        let attempt_anchor = position_attempt_anchor(
            attempt_anchors,
            &target,
            snapshot_state,
            &row,
            request.expected_leg_count.unwrap_or(1),
        );
        let context = close_request_context_with_attempt(
            "single",
            &target,
            request.snapshot_version.as_deref(),
            attempt_anchor.as_deref(),
        );
        let pending_evidence =
            position_scope_evidence(&context, ActionRunKind::PortfolioClosePosition, &row);
        active_key.set(Some(key));
        state.set(
            ActionState::pending(format!("正在提交 {label} 平仓"))
                .with_evidence(pending_evidence.clone()),
        );
        let client = client.clone();
        spawn_local(async move {
            let result = close_position_task(client, row, request, context).await;
            active_key.set(None);
            match result {
                Ok(run) => {
                    remember_position_attempt_anchor(attempt_anchors, &target, &run);
                    apply_close_run_result(state, "平仓", &run, pending_evidence)
                }
                Err(error) => state.set(
                    ActionState::failed("平仓失败", error.problem).with_evidence(pending_evidence),
                ),
            }
        });
    });
    let close_pair = Callback::new(move |row: PositionRow| {
        if state.get_untracked().is_pending() {
            return;
        }
        let Some(key) = pair_close_key(&row) else {
            state.set(ActionState::failed(
                "配对平仓失败",
                missing_pair_problem(&row),
            ));
            return;
        };
        let execution_scope = match close_execution_scope(
            trading_status,
            row.origin == shared_types::PositionOrigin::AccountPrivate,
        ) {
            Ok(scope) => scope,
            Err(problem) => {
                state.set(ActionState::failed("配对平仓失败", *problem));
                return;
            }
        };
        let request = match close_position_request(snapshot_state, &row, 2, "positions.close_pair")
        {
            Ok(request) => request,
            Err(problem) => {
                state.set(ActionState::failed("配对平仓失败", *problem));
                return;
            }
        };
        let label = format!("{} {} 配对", row.venue, row.symbol);
        let target = format!("{key}:{execution_scope}");
        let attempt_anchor = position_attempt_anchor(
            attempt_anchors,
            &target,
            snapshot_state,
            &row,
            request.expected_leg_count.unwrap_or(2),
        );
        let context = close_request_context_with_attempt(
            "pair",
            &target,
            request.snapshot_version.as_deref(),
            attempt_anchor.as_deref(),
        );
        let pending_evidence =
            position_scope_evidence(&context, ActionRunKind::PortfolioClosePair, &row);
        active_key.set(Some(key));
        state.set(
            ActionState::pending(format!("正在提交 {label} 平仓"))
                .with_evidence(pending_evidence.clone()),
        );
        let client = pair_client.clone();
        spawn_local(async move {
            let result = close_position_pair_task(client, row, request, context).await;
            active_key.set(None);
            match result {
                Ok(run) => {
                    remember_position_attempt_anchor(attempt_anchors, &target, &run);
                    apply_close_run_result(state, "配对平仓", &run, pending_evidence)
                }
                Err(error) => state.set(
                    ActionState::failed("配对平仓失败", error.problem)
                        .with_evidence(pending_evidence),
                ),
            }
        });
    });

    PositionCloseAction {
        state,
        active_key,
        close_one,
        close_pair,
    }
}

pub(in crate::panels::modules::positions) fn use_close_all_positions_action(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
    trading_status: RwSignal<LoadState<TradingStatusResponse>>,
) -> CloseAllPositionsAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    let attempt_anchor = RwSignal::new(None::<(String, i64)>);
    recover_close_all_state(state, snapshot_state);
    let submit = Callback::new(move |confirmation_phrase: String| {
        if state.get_untracked().is_pending() {
            return;
        }
        let execution_scope = match close_execution_scope(
            trading_status,
            portfolio_close_requires_live(snapshot_state),
        ) {
            Ok(scope) => scope,
            Err(problem) => {
                state.set(ActionState::failed("全部平仓失败", *problem));
                return;
            }
        };
        let request =
            match close_all_request(snapshot_state, confirmation_phrase, "positions.close_all") {
                Ok(request) => request,
                Err(problem) => {
                    state.set(ActionState::failed("全部平仓失败", *problem));
                    return;
                }
            };
        let snapshot_attempt_anchor = close_all_attempt_anchor(attempt_anchor, snapshot_state);
        let context = close_all_request_context_with_attempt(
            request.snapshot_version.as_deref(),
            &request.confirmation_phrase,
            &execution_scope,
            snapshot_attempt_anchor.as_deref(),
        );
        let pending_evidence = portfolio_scope_evidence(&context, snapshot_state);
        state.set(ActionState::pending("正在提交全部平仓").with_evidence(pending_evidence.clone()));
        let client = client.clone();
        spawn_local(async move {
            match close_all_positions_task(client, request, context).await {
                Ok(run) => {
                    if let Some(anchor) = close_run_next_attempt_anchor(&run) {
                        attempt_anchor.set(Some(anchor));
                    }
                    apply_close_run_result(state, "全部平仓", &run, pending_evidence)
                }
                Err(error) => state.set(
                    ActionState::failed("全部平仓失败", error.problem)
                        .with_evidence(pending_evidence),
                ),
            }
        });
    });
    CloseAllPositionsAction { state, submit }
}
