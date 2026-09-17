//! 持仓模块的 workstation-owned 运行态信号。
//!
//! workstation shell 只挂载当前激活模块（inactive 模块 unmount，避免 hidden
//! mount 的额外轮询）。为了让「切走再切回持仓」不再先清空成 Loading、再等首帧，
//! 把 portfolio 快照与 NAV 历史的 [`LoadState`] 信号提升到 workstation 持有：
//! 重新挂载时立即以上次成功数据渲染，背景再异步刷新，与 futures/opportunities 的
//! 跨模块状态恢复保持一致。WS 订阅与轮询仍由 `data.rs` hooks 在挂载期重建/清理。

use super::snapshot::PortfolioNavHistoryState;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use leptos::prelude::*;
use shared_types::{ApiProblem, PortfolioSnapshot};

use super::access::portfolio_account_access;

/// 持仓模块跨模块切换保留的运行态信号集合（由 workstation 持有）。
#[derive(Clone, Copy)]
pub(in crate::panels) struct PositionsRuntime {
    pub(in crate::panels::modules::positions) snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    pub(in crate::panels::modules::positions) nav_history: PortfolioNavHistoryState,
}

/// 在 workstation 初始化时创建一次；首帧前为 Loading，之后跨模块切换保留最近数据。
pub(in crate::panels) fn create_positions_runtime() -> PositionsRuntime {
    PositionsRuntime {
        snapshot: RwSignal::new(LoadState::Loading),
        nav_history: RwSignal::new(LoadState::Loading),
    }
}

impl PositionsRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        let snapshot = self.snapshot.with(|state| {
            let runtime = snapshot_module_runtime_state(state);
            state
                .value()
                .map(portfolio_account_access)
                .filter(|access| access.account_data_unavailable())
                .map_or(runtime, |_| ModuleRuntimeState::setup_required())
        });
        ModuleRuntimeState::combine([
            snapshot,
            self.nav_history.with(ModuleRuntimeState::from_load_state),
        ])
    }
}

pub(in crate::panels::modules::positions) fn is_current_partial_snapshot_problem(
    problem: &ApiProblem,
) -> bool {
    problem.code == shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED
}

fn snapshot_module_runtime_state<T>(state: &LoadState<T>) -> ModuleRuntimeState {
    match state {
        LoadState::Stale { problem, .. } if is_current_partial_snapshot_problem(problem) => {
            ModuleRuntimeState::ready()
        }
        _ => ModuleRuntimeState::from_load_state(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_partial_snapshot_does_not_raise_global_stale_state() {
        let state = LoadState::Stale {
            value: 7_u8,
            problem: ApiProblem::new(
                shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED,
                "current snapshot has partial field evidence",
            ),
        };

        assert_eq!(
            snapshot_module_runtime_state(&state).status,
            crate::state::module_runtime::ModuleRuntimeStatus::Ready
        );
    }

    #[test]
    fn transport_stale_problem_still_raises_global_stale_state() {
        let state = LoadState::Stale {
            value: 7_u8,
            problem: ApiProblem::new(
                shared_types::problem::codes::PORTFOLIO_SNAPSHOT_STALE,
                "snapshot transport is stale",
            ),
        };

        assert_eq!(
            snapshot_module_runtime_state(&state).status,
            crate::state::module_runtime::ModuleRuntimeStatus::Stale
        );
    }
}
