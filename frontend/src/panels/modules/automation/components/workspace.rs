use crate::state::load_state::LoadState;
use crate::state::module_runtime::{normalize_choice, store_choice, stored_choice};
use leptos::prelude::*;
use shared_types::{AutoProfitCloseConfig, AutomationRuntimeStatus, WebhookRuntimeStatus};

use super::{decision_log, execution_evidence, runtime_board};

const AUTOMATION_WORKSPACE_TAB_STORAGE_KEY: &str = "crossline.automation.workspaceTab";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutomationWorkspaceTab {
    Runtime,
    Decisions,
    Receipts,
    Evidence,
}

impl AutomationWorkspaceTab {
    const fn slug(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Decisions => "decisions",
            Self::Receipts => "receipts",
            Self::Evidence => "evidence",
        }
    }

    fn from_slug(value: &str) -> Option<Self> {
        Some(match normalize_choice(value) {
            "runtime" => Self::Runtime,
            "decisions" => Self::Decisions,
            "receipts" => Self::Receipts,
            "evidence" => Self::Evidence,
            _ => return None,
        })
    }

    const fn tab_id(self) -> &'static str {
        match self {
            Self::Runtime => "automation-tab-runtime",
            Self::Decisions => "automation-tab-decisions",
            Self::Receipts => "automation-tab-receipts",
            Self::Evidence => "automation-tab-evidence",
        }
    }

    const fn panel_id(self) -> &'static str {
        match self {
            Self::Runtime => "automation-panel-runtime",
            Self::Decisions => "automation-panel-decisions",
            Self::Receipts => "automation-panel-receipts",
            Self::Evidence => "automation-panel-evidence",
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Runtime => Self::Decisions,
            Self::Decisions => Self::Receipts,
            Self::Receipts => Self::Evidence,
            Self::Evidence => Self::Runtime,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::Runtime => Self::Evidence,
            Self::Decisions => Self::Runtime,
            Self::Receipts => Self::Decisions,
            Self::Evidence => Self::Receipts,
        }
    }
}

pub(in crate::panels::modules::automation) fn automation_workspace(
    state: RwSignal<LoadState<AutomationRuntimeStatus>>,
    protection: RwSignal<LoadState<AutoProfitCloseConfig>>,
    webhook: RwSignal<LoadState<WebhookRuntimeStatus>>,
    receipts: super::super::receipts::ReceiptData,
) -> impl IntoView {
    let stored_tab = stored_choice(
        AUTOMATION_WORKSPACE_TAB_STORAGE_KEY,
        AutomationWorkspaceTab::from_slug,
    );
    let initial_preference = prefer_runtime_on_entry(&state.get_untracked());
    let active_tab = RwSignal::new(if initial_preference == Some(true) {
        AutomationWorkspaceTab::Runtime
    } else {
        stored_tab.unwrap_or(AutomationWorkspaceTab::Runtime)
    });
    let entry_state_resolved = RwSignal::new(initial_preference.is_some());
    let runtime_tab_ref = NodeRef::<leptos::html::Button>::new();
    let decisions_tab_ref = NodeRef::<leptos::html::Button>::new();
    let evidence_tab_ref = NodeRef::<leptos::html::Button>::new();
    let receipts_tab_ref = NodeRef::<leptos::html::Button>::new();
    let inspect_run = Callback::new(move |id: String| {
        receipts.choice.set(id);
        active_tab.set(AutomationWorkspaceTab::Receipts);
    });

    Effect::new(move |_| {
        if entry_state_resolved.get_untracked() {
            return;
        }
        let Some(prefer_runtime) = prefer_runtime_on_entry(&state.get()) else {
            return;
        };
        if prefer_runtime {
            active_tab.set(AutomationWorkspaceTab::Runtime);
        }
        entry_state_resolved.set(true);
    });

    Effect::new(move |_| {
        store_choice(
            AUTOMATION_WORKSPACE_TAB_STORAGE_KEY,
            active_tab.get().slug(),
        );
    });

    view! {
        <section class="automation-workspace">
            <nav
                class="automation-workspace-tabs"
                role="tablist"
                aria-label="自动化工作面"
                aria-orientation="horizontal"
                on:keydown=move |event| {
                    let next = match event.key().as_str() {
                        "ArrowRight" => Some(active_tab.get().next()),
                        "ArrowLeft" => Some(active_tab.get().previous()),
                        "Home" => Some(AutomationWorkspaceTab::Runtime),
                        "End" => Some(AutomationWorkspaceTab::Evidence),
                        _ => None,
                    };
                    let Some(next) = next else { return };
                    event.prevent_default();
                    active_tab.set(next);
                    let target = match next {
                        AutomationWorkspaceTab::Runtime => runtime_tab_ref,
                        AutomationWorkspaceTab::Decisions => decisions_tab_ref,
                        AutomationWorkspaceTab::Receipts => receipts_tab_ref,
                        AutomationWorkspaceTab::Evidence => evidence_tab_ref,
                    };
                    if let Some(button) = target.get() {
                        let _ = button.focus();
                    }
                }
            >
                {workspace_tab(
                    active_tab,
                    AutomationWorkspaceTab::Runtime,
                    "当前运行",
                    runtime_tab_ref,
                )}
                {workspace_tab(
                    active_tab,
                    AutomationWorkspaceTab::Decisions,
                    "决策历史",
                    decisions_tab_ref,
                )}
                {workspace_tab(
                    active_tab,
                    AutomationWorkspaceTab::Receipts,
                    "交易记录",
                    receipts_tab_ref,
                )}
                {workspace_tab(
                    active_tab,
                    AutomationWorkspaceTab::Evidence,
                    "处理流程",
                    evidence_tab_ref,
                )}
            </nav>
            <div
                class="automation-workspace-panel"
                role="tabpanel"
                id=move || active_tab.get().panel_id()
                aria-labelledby=move || active_tab.get().tab_id()
                tabindex="0"
            >
                {move || match active_tab.get() {
                    AutomationWorkspaceTab::Runtime => {
                        runtime_board(state, protection, inspect_run).into_any()
                    }
                    AutomationWorkspaceTab::Decisions => decision_log(state, inspect_run).into_any(),
                    AutomationWorkspaceTab::Receipts => super::receipts::receipts_panel(receipts).into_any(),
                    AutomationWorkspaceTab::Evidence => {
                        execution_evidence(state, protection, webhook, receipts.state).into_any()
                    }
                }}
            </div>
        </section>
    }
}

fn prefer_runtime_on_entry(state: &LoadState<AutomationRuntimeStatus>) -> Option<bool> {
    match state {
        LoadState::Loading => None,
        LoadState::Error(_) => Some(true),
        LoadState::Ready(status) | LoadState::Stale { value: status, .. } => {
            Some(!status.config.enabled)
        }
    }
}

fn workspace_tab(
    active_tab: RwSignal<AutomationWorkspaceTab>,
    tab: AutomationWorkspaceTab,
    label: &'static str,
    node_ref: NodeRef<leptos::html::Button>,
) -> impl IntoView {
    view! {
        <button
            node_ref=node_ref
            type="button"
            id=tab.tab_id()
            role="tab"
            aria-controls=tab.panel_id()
            class=move || if active_tab.get() == tab {
                "automation-workspace-tab is-active"
            } else {
                "automation-workspace-tab"
            }
            aria-selected=move || (active_tab.get() == tab).to_string()
            tabindex=move || if active_tab.get() == tab { 0 } else { -1 }
            on:click=move |_| active_tab.set(tab)
        >{label}</button>
    }
}
