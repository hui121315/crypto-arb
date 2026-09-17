use crate::state::action_state::{action_state_from_action_run, latest_action_run, ActionState};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ActionRun, ActionRunKind};

use super::resources::SettingsResource;

pub(in crate::panels::modules::settings) fn use_action_run_recovery(
    state: RwSignal<ActionState>,
    runs: SettingsResource<Vec<ActionRun>>,
    kinds: Vec<ActionRunKind>,
) {
    Effect::new(move |_| {
        let runs = runs.get();
        if state.get_untracked().is_pending() {
            return;
        }
        let rows = match &runs {
            LoadState::Ready(rows) | LoadState::Stale { value: rows, .. } => rows,
            LoadState::Loading | LoadState::Error(_) => return,
        };
        if let Some(run) = latest_action_run(rows, &kinds) {
            state.set(action_state_from_action_run(run));
        }
    });
}
