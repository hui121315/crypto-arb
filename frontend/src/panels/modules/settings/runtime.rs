use super::data::{create_market_subscriptions, MarketSubscriptionData};
use super::tabs::risk_config::{create_risk_config_runtime, RiskConfigRuntime};
use super::tabs::webhook::{create_webhook_runtime, WebhookRuntime};

mod account;
pub(super) use account::{CredentialRuntime, EnvironmentRuntime};
mod diagnostics;
pub(super) use diagnostics::DiagnosticsRuntime;
mod health;
pub(super) use health::{action_health, PaneState};

#[derive(Clone, Copy)]
pub(in crate::panels) struct SettingsRuntime {
    pub(super) pane: PaneState,
    pub(super) risk: RiskConfigRuntime,
    pub(super) webhook: WebhookRuntime,
    pub(super) market: MarketSubscriptionData,
    pub(super) credentials: CredentialRuntime,
    pub(super) environment: EnvironmentRuntime,
    pub(super) diagnostics: DiagnosticsRuntime,
}

pub(in crate::panels) fn create_settings_runtime() -> SettingsRuntime {
    let runtime = SettingsRuntime {
        pane: PaneState::new(),
        risk: create_risk_config_runtime(),
        webhook: create_webhook_runtime(),
        market: create_market_subscriptions(),
        credentials: account::create_credential_runtime(),
        environment: account::create_environment_runtime(),
        diagnostics: diagnostics::create_diagnostics_runtime(),
    };
    leptos::prelude::provide_context(runtime.risk);
    runtime
}

impl SettingsRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> crate::state::module_runtime::ModuleRuntimeState {
        self.pane.get()
    }
}
