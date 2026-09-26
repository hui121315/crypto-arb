use super::super::data::SettingsJournal;
use crate::state::{action_state::ActionState, load_state::LoadState, module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus}};
use leptos::prelude::*;
use shared_types::ApiProblem;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct PaneState(RwSignal<ModuleRuntimeState>);

impl PaneState {
    pub(super) fn new() -> Self {
        Self(RwSignal::new(ModuleRuntimeState::from_load_state(&LoadState::<()>::Loading)))
    }

    pub(in crate::panels::modules::settings) fn child() -> Self { Self::new() }

    pub(in crate::panels::modules::settings) fn get(self) -> ModuleRuntimeState { self.0.get() }

    pub(in crate::panels::modules::settings) fn callback(self) -> Callback<ModuleRuntimeState> {
        Callback::new(move |state| {
            if self.0.get_untracked() != state { self.0.set(state); }
        })
    }

    pub(in crate::panels::modules::settings) fn track(self, state: impl Fn() -> ModuleRuntimeState + 'static) {
        let report = self.callback();
        Effect::new(move |_| report.run(state()));
    }
}

pub(in crate::panels::modules::settings) fn action_health(
    journal: SettingsJournal,
    action: &ActionState,
) -> ModuleRuntimeState {
    if journal.pending.with(Option::is_some) || journal.busy.get() {
        return ModuleRuntimeState {
            status: ModuleRuntimeStatus::Pending,
            problem: ModuleRuntimeState::from_action_state(action).problem,
            pending_label: Some(if journal.busy.get() { "操作进行中" } else { "原操作结果待核对" }.into()),
        };
    }
    if let Some(message) = journal.problem.get() {
        return ModuleRuntimeState::from_problem(Some(
            ApiProblem::new("SETTINGS_RECOVERY_UNAVAILABLE", message).with_source("frontend.settings"),
        ));
    }
    let mut state = ModuleRuntimeState::from_action_state(action);
    if let ActionState::Failed { label, .. } = action {
        state.pending_label = Some(label.clone());
    }
    state
}
