use crate::api::rest::MutationRequestContext;
use crate::panels::modules::kill_switch_idempotency;
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ActionRunKind, KillSwitchRequest};

use super::super::requests::set_kill_switch_task;
use super::super::runs::{
    bump_refresh, kill_switch_response_evidence, kill_switch_success_message,
};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PositionsKillSwitchAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<KillSwitchRequest>,
}

pub(in crate::panels::modules::positions) fn use_positions_kill_switch_action(
    refresh_nonce: RwSignal<u64>,
) -> PositionsKillSwitchAction {
    let client = use_global().client;
    let state = RwSignal::new(ActionState::Idle);
    let replay = RwSignal::new(None::<kill_switch_idempotency::KillSwitchReplaySlot>);
    let submit = Callback::new(move |request: KillSwitchRequest| {
        if state.get_untracked().is_pending() {
            return;
        }
        let slot = kill_switch_idempotency::replay_slot(replay.get_untracked(), &request);
        replay.set(Some(slot.clone()));
        let context = MutationRequestContext::with_idempotency_key(slot.key.clone());
        let pending_evidence = context
            .evidence()
            .with_action_kind(ActionRunKind::TradingKillSwitch);
        state.set(ActionState::pending("正在更新风控总闸").with_evidence(pending_evidence.clone()));
        let client = client.clone();
        spawn_local(async move {
            match set_kill_switch_task(client, request, context).await {
                Ok(response) => {
                    replay.set(None);
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::succeeded(kill_switch_success_message("风控总闸", &response))
                            .with_evidence(kill_switch_response_evidence(
                                &response,
                                pending_evidence,
                            )),
                    );
                }
                Err(error) => {
                    if kill_switch_idempotency::should_reuse_replay_key(&error) {
                        replay.set(Some(slot));
                    } else {
                        replay.set(None);
                    }
                    state.set(
                        ActionState::failed("总闸更新失败", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    PositionsKillSwitchAction { state, submit }
}
