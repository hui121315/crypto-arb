//! 执行模块 workstation-owned 运行态信号。
//!
//! workstation 只挂载激活模块，离开执行会 unmount 其 WS 订阅与轮询 Effect（不留
//! hidden 轮询）。把订单队列与最近 [`ExecutionRun`] feed 提升到 workstation 持有，
//! 切回执行即以上次成功的订单/run 渲染，背景 WS+REST 再刷新，与
//! futures/opportunities/positions/review 的跨模块状态恢复一致。草案输入本就走
//! localStorage 恢复；preview、confirm action 与刷新 nonce 也由 workstation 持有，
//! 所以离开模块不会丢失 pending/outcome/problem 证据。

use leptos::prelude::*;
use shared_types::{ActionState, HedgeConfirmContext, HedgeConfirmResponse};

use crate::api::ws::WsChannelState;
use crate::panels::modules::execution::draft::inputs::DraftInputs;
use crate::panels::routing::WorkspaceRoute;
use crate::panels::workstation::ModuleId;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;

use super::orders::{OrderQueue, ORDERS_CHANNEL};
use super::preview::ExecutionPreview;
use super::run::{
    clear_execution_run_context, store_workspace_route_context, ExecutionRunFeed, EXECUTION_CHANNEL,
};
use super::submission::SubmissionRecovery;
use super::workflow::WorkflowViewFeed;
use crate::panels::modules::execution::selection::{ExecutionSelection, ExecutionSelectionSeed};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct ConfirmActionRuntime {
    pub state: RwSignal<ActionState>,
    pub last_outcome: RwSignal<Option<HedgeConfirmResponse>>,
    pub context: RwSignal<Option<HedgeConfirmContext>>,
    pub recovery: SubmissionRecovery,
}

#[derive(Clone, Copy)]
pub(in crate::panels) struct ExecutionRuntime {
    selection: RwSignal<ExecutionSelection>,
    draft_inputs: DraftInputs,
    pub(in crate::panels::modules::execution) preview_nonce: RwSignal<u64>,
    pub(in crate::panels::modules::execution) runtime_refresh_nonce: RwSignal<u64>,
    pub(in crate::panels::modules::execution) preview_state: RwSignal<LoadState<ExecutionPreview>>,
    pub(in crate::panels::modules::execution) confirm: ConfirmActionRuntime,
    pub(in crate::panels::modules::execution) cancel_state: RwSignal<ActionState>,
    pub(in crate::panels::modules::execution) order_queue: RwSignal<OrderQueue>,
    pub(in crate::panels::modules::execution) order_channel_state: RwSignal<WsChannelState>,
    pub(in crate::panels::modules::execution) run: ExecutionRunFeed,
    pub(in crate::panels::modules::execution) workflow: WorkflowViewFeed,
}

impl ExecutionRuntime {
    pub(in crate::panels) fn selection(self) -> RwSignal<ExecutionSelection> {
        self.selection
    }

    pub(in crate::panels) fn seed_selection(self, seed: ExecutionSelectionSeed) {
        let selection = seed.into_selection();
        let reset_settled = !self.confirm.recovery.blocked()
            && should_reset_confirm_for_new_draft(&self.confirm.state.get_untracked())
            && self
                .run
                .run
                .with_untracked(|row| row.as_ref().is_none_or(super::remedy::run_is_released));
        if reset_settled {
            self.confirm.state.set(ActionState::Idle);
            self.confirm.last_outcome.set(None);
            self.confirm.context.set(None);
            reset_settled_workflow(self.run, self.workflow);
        } else if !self.confirm.recovery.blocked()
            && !self.workflow.matches_opportunity(&selection.opportunity_id)
        {
            self.workflow.clear();
        }
        self.draft_inputs.apply_selection(&selection);
        self.selection.set(selection);
        self.runtime_refresh_nonce
            .update(|value| *value = value.wrapping_add(1));
    }

    pub(in crate::panels::modules::execution) fn draft_inputs(
        self,
        selection: &ExecutionSelection,
    ) -> DraftInputs {
        self.draft_inputs.apply_selection(selection);
        self.draft_inputs
    }

    pub(in crate::panels) fn apply_workspace_route(self, route: &WorkspaceRoute) {
        if route.module != ModuleId::Execution
            || (route.opportunity_id.is_none() && route.run_id.is_none())
        {
            return;
        }
        if !self.confirm.recovery.blocked() {
            store_workspace_route_context(route.opportunity_id.as_deref(), route.run_id.as_deref());
        }
        self.runtime_refresh_nonce
            .update(|value| *value = value.wrapping_add(1));
    }

    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        let order_problem = self.order_queue.with(|queue| queue.runtime_problem());
        let run_problem = self
            .run
            .seed_problem
            .get()
            .or_else(|| self.run.stream_problem.get())
            .or_else(|| self.run.channel_state.get().last_error)
            .or_else(|| self.order_channel_state.get().last_error);
        ModuleRuntimeState::combine([
            self.preview_state.with(ModuleRuntimeState::from_load_state),
            self.confirm
                .state
                .with(ModuleRuntimeState::from_action_state),
            ModuleRuntimeState::from_problem(order_problem),
            ModuleRuntimeState::from_problem(run_problem),
        ])
    }
}

fn should_reset_confirm_for_new_draft(state: &ActionState) -> bool {
    !matches!(
        state,
        ActionState::Pending { .. } | ActionState::Accepted { .. }
    )
}

fn reset_settled_workflow(run: ExecutionRunFeed, workflow: WorkflowViewFeed) {
    clear_execution_run_context();
    reset_run_feed(run);
    workflow.clear();
}

fn reset_run_feed(run: ExecutionRunFeed) {
    run.run.set(None);
    run.seed_problem.set(None);
    run.stream_problem.set(None);
}

/// workstation 初始化时创建一次；首帧前为空，之后跨模块切换保留最近订单与 run。
pub(in crate::panels) fn create_execution_runtime() -> ExecutionRuntime {
    let selection = RwSignal::new(ExecutionSelection::empty());
    let draft_inputs = DraftInputs::new(&selection.get_untracked());
    let recovery = SubmissionRecovery::new(crate::state::context::use_global().client.base_url());
    let pending = recovery.pending.get_untracked();
    ExecutionRuntime {
        selection,
        draft_inputs,
        preview_nonce: RwSignal::new(0),
        runtime_refresh_nonce: RwSignal::new(0),
        preview_state: RwSignal::new(LoadState::Loading),
        confirm: ConfirmActionRuntime {
            state: RwSignal::new(if pending.is_some() {
                ActionState::accepted("原提交结果待核验")
            } else {
                ActionState::Idle
            }),
            last_outcome: RwSignal::new(None),
            context: RwSignal::new(pending),
            recovery,
        },
        order_queue: RwSignal::new(OrderQueue::default()),
        cancel_state: RwSignal::new(ActionState::Idle),
        order_channel_state: RwSignal::new(WsChannelState::new(ORDERS_CHANNEL)),
        run: ExecutionRunFeed {
            run: RwSignal::new(None),
            seed_problem: RwSignal::new(None),
            stream_problem: RwSignal::new(None),
            channel_state: RwSignal::new(WsChannelState::new(EXECUTION_CHANNEL)),
        },
        workflow: WorkflowViewFeed::restored(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::ApiProblem;

    #[test]
    fn selection_change_resets_only_settled_confirm_state() {
        assert!(should_reset_confirm_for_new_draft(&ActionState::succeeded(
            "done"
        ),));
        assert!(!should_reset_confirm_for_new_draft(&ActionState::pending(
            "submitting",
        )));
    }

    #[test]
    fn new_draft_clears_stale_run_failures() {
        Owner::new().with(|| {
            let run = ExecutionRunFeed {
                run: RwSignal::new(None),
                seed_problem: RwSignal::new(None),
                stream_problem: RwSignal::new(None),
                channel_state: RwSignal::new(WsChannelState::new(EXECUTION_CHANNEL)),
            };
            run.seed_problem
                .set(Some(ApiProblem::new("EXECUTION_RUN_SEED_STALE", "stale")));
            run.stream_problem
                .set(Some(ApiProblem::new("WS_STALE", "stale")));

            reset_run_feed(run);

            assert!(run.run.get_untracked().is_none());
            assert!(run.seed_problem.get_untracked().is_none());
            assert!(run.stream_problem.get_untracked().is_none());
        });
    }
}
