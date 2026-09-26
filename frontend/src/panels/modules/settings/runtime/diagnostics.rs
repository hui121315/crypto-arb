use super::super::data::{current_api_base, use_api_base_validate_action, ApiBaseValidateAction};
use leptos::prelude::*;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct DiagnosticsRuntime {
    pub api_base: RwSignal<String>,
    pub validate: ApiBaseValidateAction,
}

pub(super) fn create_diagnostics_runtime() -> DiagnosticsRuntime {
    let api_base = RwSignal::new(current_api_base());
    let app = expect_context::<crate::state::AppContext>();
    Effect::new(move |previous: Option<String>| {
        let current = app.api_base.get();
        if previous.as_ref().is_some_and(|value| value == &api_base.get_untracked()) {
            api_base.set(current.clone());
        }
        current
    });
    DiagnosticsRuntime { api_base, validate: use_api_base_validate_action() }
}
