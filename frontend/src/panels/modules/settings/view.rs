use crate::panels::shared::{ModuleHeader, Surface};
use crate::state::module_runtime::{normalize_choice, store_choice, stored_choice};
use leptos::prelude::*;

use crate::panels::modules::execution::ExecutionRuntime;

use super::tabs::{
    action_runs_tab, credentials_tab, diagnostics_tab, execution_environment_panel,
    market_subscriptions_tab, risk_config_tab, webhook_tab,
};

const SETTINGS_TAB_STORAGE_KEY: &str = "crossline.settings.activeTab";

pub(in crate::panels) fn select_credentials_tab() {
    store_choice(SETTINGS_TAB_STORAGE_KEY, SettingsTab::Credentials.slug());
}

pub(in crate::panels) fn select_risk_tab() {
    store_choice(SETTINGS_TAB_STORAGE_KEY, SettingsTab::Risk.slug());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsTab {
    Execution,
    MarketData,
    Credentials,
    Risk,
    ActionRuns,
    Diagnostics,
    Webhook,
}

impl SettingsTab {
    const fn slug(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::MarketData => "market-data",
            Self::Credentials => "credentials",
            Self::Risk => "risk",
            Self::ActionRuns => "action-runs",
            Self::Diagnostics => "diagnostics",
            Self::Webhook => "webhook",
        }
    }

    fn from_slug(value: &str) -> Option<Self> {
        Some(match normalize_choice(value) {
            "execution" | "adapters" => Self::Execution,
            "market-data" | "subscriptions" => Self::MarketData,
            "credentials" => Self::Credentials,
            "risk" => Self::Risk,
            "action-runs" => Self::ActionRuns,
            "diagnostics" => Self::Diagnostics,
            "webhook" => Self::Webhook,
            _ => return None,
        })
    }

    const fn tab_id(self) -> &'static str {
        match self {
            Self::Execution => "settings-tab-execution",
            Self::MarketData => "settings-tab-market-data",
            Self::Credentials => "settings-tab-credentials",
            Self::Risk => "settings-tab-risk",
            Self::ActionRuns => "settings-tab-action-runs",
            Self::Diagnostics => "settings-tab-diagnostics",
            Self::Webhook => "settings-tab-webhook",
        }
    }

    const fn panel_id(self) -> &'static str {
        match self {
            Self::Execution => "settings-panel-execution",
            Self::MarketData => "settings-panel-market-data",
            Self::Credentials => "settings-panel-credentials",
            Self::Risk => "settings-panel-risk",
            Self::ActionRuns => "settings-panel-action-runs",
            Self::Diagnostics => "settings-panel-diagnostics",
            Self::Webhook => "settings-panel-webhook",
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Execution => Self::MarketData,
            Self::MarketData => Self::Credentials,
            Self::Credentials => Self::Risk,
            Self::Risk => Self::ActionRuns,
            Self::ActionRuns => Self::Diagnostics,
            Self::Diagnostics => Self::Webhook,
            Self::Webhook => Self::Execution,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::Execution => Self::Webhook,
            Self::MarketData => Self::Execution,
            Self::Credentials => Self::MarketData,
            Self::Risk => Self::Credentials,
            Self::ActionRuns => Self::Risk,
            Self::Diagnostics => Self::ActionRuns,
            Self::Webhook => Self::Diagnostics,
        }
    }
}

pub(in crate::panels) fn settings_module(execution_runtime: ExecutionRuntime) -> impl IntoView {
    let active = RwSignal::new(
        stored_choice(SETTINGS_TAB_STORAGE_KEY, SettingsTab::from_slug)
            .unwrap_or(SettingsTab::Credentials),
    );
    let execution_ref = NodeRef::<leptos::html::Button>::new();
    let market_data_ref = NodeRef::<leptos::html::Button>::new();
    let credentials_ref = NodeRef::<leptos::html::Button>::new();
    let risk_ref = NodeRef::<leptos::html::Button>::new();
    let action_runs_ref = NodeRef::<leptos::html::Button>::new();
    let diagnostics_ref = NodeRef::<leptos::html::Button>::new();
    let webhook_ref = NodeRef::<leptos::html::Button>::new();

    Effect::new(move |_| {
        store_choice(SETTINGS_TAB_STORAGE_KEY, active.get().slug());
    });

    view! {
        <section class="module-page settings-workspace">
            <ModuleHeader title="设置"/>
            <Surface title="配置工作区" meta="后端生效" class_name="full-surface settings-surface">
                <div
                    class="settings-tabs"
                    role="tablist"
                    aria-label="设置分类"
                    aria-orientation="horizontal"
                    on:keydown=move |event| {
                        let next = match event.key().as_str() {
                            "ArrowRight" => Some(active.get().next()),
                            "ArrowLeft" => Some(active.get().previous()),
                            "Home" => Some(SettingsTab::Execution),
                            "End" => Some(SettingsTab::Webhook),
                            _ => None,
                        };
                        let Some(next) = next else { return };
                        event.prevent_default();
                        active.set(next);
                        let target = match next {
                            SettingsTab::Execution => execution_ref,
                            SettingsTab::MarketData => market_data_ref,
                            SettingsTab::Credentials => credentials_ref,
                            SettingsTab::Risk => risk_ref,
                            SettingsTab::ActionRuns => action_runs_ref,
                            SettingsTab::Diagnostics => diagnostics_ref,
                            SettingsTab::Webhook => webhook_ref,
                        };
                        if let Some(button) = target.get() {
                            let _ = button.focus();
                        }
                    }
                >
                    <Tab label="执行环境" tab=SettingsTab::Execution active=active node_ref=execution_ref/>
                    <Tab label="行情" tab=SettingsTab::MarketData active=active node_ref=market_data_ref/>
                    <Tab label="凭证" tab=SettingsTab::Credentials active=active node_ref=credentials_ref/>
                    <Tab label="风控" tab=SettingsTab::Risk active=active node_ref=risk_ref/>
                    <Tab label="动作账本" tab=SettingsTab::ActionRuns active=active node_ref=action_runs_ref/>
                    <Tab label="诊断" tab=SettingsTab::Diagnostics active=active node_ref=diagnostics_ref/>
                    <Tab label="Webhook" tab=SettingsTab::Webhook active=active node_ref=webhook_ref/>
                </div>
                <section
                    class="settings-content-panel"
                    role="tabpanel"
                    id=move || active.get().panel_id()
                    aria-labelledby=move || active.get().tab_id()
                    tabindex="0"
                >
                    {move || match active.get() {
                        SettingsTab::Execution => execution_environment_panel().into_any(),
                        SettingsTab::MarketData => market_subscriptions_tab().into_any(),
                        SettingsTab::Credentials => credentials_tab().into_any(),
                        SettingsTab::Risk => risk_config_tab().into_any(),
                        SettingsTab::ActionRuns => action_runs_tab().into_any(),
                        SettingsTab::Diagnostics => {
                            diagnostics_tab(execution_runtime.selection()).into_any()
                        }
                        SettingsTab::Webhook => webhook_tab().into_any(),
                    }}
                </section>
            </Surface>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_tab_slug_round_trips() {
        for tab in [
            SettingsTab::Execution,
            SettingsTab::MarketData,
            SettingsTab::Credentials,
            SettingsTab::Risk,
            SettingsTab::ActionRuns,
            SettingsTab::Diagnostics,
            SettingsTab::Webhook,
        ] {
            assert_eq!(SettingsTab::from_slug(tab.slug()), Some(tab));
        }
    }

    #[test]
    fn settings_tab_parser_accepts_stored_hash_and_rejects_unknown() {
        assert_eq!(
            SettingsTab::from_slug(" #diagnostics "),
            Some(SettingsTab::Diagnostics)
        );
        assert_eq!(SettingsTab::from_slug("/risk"), Some(SettingsTab::Risk));
        assert_eq!(
            SettingsTab::from_slug("adapters"),
            Some(SettingsTab::Execution)
        );
        assert_eq!(SettingsTab::from_slug("readiness"), None);
    }
}

#[component]
fn Tab(
    #[prop(into)] label: String,
    tab: SettingsTab,
    active: RwSignal<SettingsTab>,
    node_ref: NodeRef<leptos::html::Button>,
) -> impl IntoView {
    view! {
        <button
            node_ref=node_ref
            type="button"
            id=tab.tab_id()
            role="tab"
            aria-controls=tab.panel_id()
            aria-selected=move || (active.get() == tab).to_string()
            tabindex=move || if active.get() == tab { 0 } else { -1 }
            class=move || if active.get() == tab { "active" } else { "" }
            on:click=move |_| active.set(tab)
        >
            {label}
        </button>
    }
}
