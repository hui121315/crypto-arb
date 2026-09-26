use crate::panels::modules::settings::tabs::risk_config::RiskConfigRuntime;
use crate::panels::shared::operation_journal::OperationJournal;
use crate::state::action_state::ActionState;
use leptos::prelude::*;
use shared_types::KillSwitchRequest;

use super::super::runs::bump_refresh;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PositionsKillSwitchAction {
    pub state: RwSignal<ActionState>,
    pub journal: OperationJournal,
    pub recheck: Callback<()>,
    pub submit: Callback<KillSwitchRequest>,
}

pub(in crate::panels::modules::positions) fn use_positions_kill_switch_action(
    refresh_nonce: RwSignal<u64>,
) -> PositionsKillSwitchAction {
    let runtime = expect_context::<RiskConfigRuntime>();
    // The request outlives this page; only a mounted page refreshes its portfolio.
    Effect::new(move |previous: Option<bool>| {
        let pending = runtime.pending();
        if previous == Some(true) && !pending {
            bump_refresh(refresh_nonce);
        }
        pending
    });
    PositionsKillSwitchAction {
        state: runtime.kill.state,
        journal: runtime.kill.journal,
        recheck: runtime.recheck,
        submit: runtime.kill.submit,
    }
}
