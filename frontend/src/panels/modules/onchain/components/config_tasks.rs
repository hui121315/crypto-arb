use leptos::prelude::*;
use shared_types::OnchainComparisonQuality;

use super::super::data::OnchainData;
use super::super::draft::OnchainConfigDraft;
use super::spread_alert::webhook_ready;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::panels::modules::onchain) enum OnchainConfigTask {
    Market,
    Connectivity,
    Alerts,
}

impl OnchainConfigTask {
    const fn next(self) -> Self {
        match self {
            Self::Market => Self::Connectivity,
            Self::Connectivity => Self::Alerts,
            Self::Alerts => Self::Market,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::Market => Self::Alerts,
            Self::Connectivity => Self::Market,
            Self::Alerts => Self::Connectivity,
        }
    }
}

pub(super) fn configuration_task_tabs(
    active: RwSignal<OnchainConfigTask>,
    draft: OnchainConfigDraft,
    data: OnchainData,
) -> AnyView {
    let market_ref = NodeRef::<leptos::html::Button>::new();
    let connectivity_ref = NodeRef::<leptos::html::Button>::new();
    let alerts_ref = NodeRef::<leptos::html::Button>::new();
    let market_status = Memo::new(move |_| market_tab_status(draft, data));
    let access_status = Memo::new(move |_| access_tab_status(data));
    let alert_status = Memo::new(move |_| alert_tab_status(draft, data));

    view! {
        <div
            class="onchain-command-tabs"
            role="tablist"
            aria-label="链上配置任务"
            aria-orientation="horizontal"
            on:keydown=move |event| {
                let next = match event.key().as_str() {
                    "ArrowRight" => Some(active.get().next()),
                    "ArrowLeft" => Some(active.get().previous()),
                    "Home" => Some(OnchainConfigTask::Market),
                    "End" => Some(OnchainConfigTask::Alerts),
                    _ => None,
                };
                let Some(next) = next else { return };
                event.prevent_default();
                active.set(next);
                let target = match next {
                    OnchainConfigTask::Market => market_ref,
                    OnchainConfigTask::Connectivity => connectivity_ref,
                    OnchainConfigTask::Alerts => alerts_ref,
                };
                if let Some(button) = target.get() {
                    let _ = button.focus();
                }
            }
        >
            <button
                id="onchain-config-tab-market"
                type="button"
                role="tab"
                node_ref=market_ref
                aria-controls="onchain-config-panel-market"
                aria-selected=move || selected(active, OnchainConfigTask::Market)
                tabindex=move || tab_index(active, OnchainConfigTask::Market)
                class:active=move || active.get() == OnchainConfigTask::Market
                on:click=move |_| active.set(OnchainConfigTask::Market)
            >
                <span>"市场"</span>
                <small class=move || market_status.get().tone>{move || market_status.get().label}</small>
            </button>
            <button
                id="onchain-config-tab-connectivity"
                type="button"
                role="tab"
                node_ref=connectivity_ref
                aria-controls="onchain-config-panel-connectivity"
                aria-selected=move || selected(active, OnchainConfigTask::Connectivity)
                tabindex=move || tab_index(active, OnchainConfigTask::Connectivity)
                class:active=move || active.get() == OnchainConfigTask::Connectivity
                on:click=move |_| active.set(OnchainConfigTask::Connectivity)
            >
                <span>"接入"</span>
                <small class=move || access_status.get().tone>{move || access_status.get().label}</small>
            </button>
            <button
                id="onchain-config-tab-alerts"
                type="button"
                role="tab"
                node_ref=alerts_ref
                aria-controls="onchain-config-panel-alerts"
                aria-selected=move || selected(active, OnchainConfigTask::Alerts)
                tabindex=move || tab_index(active, OnchainConfigTask::Alerts)
                class:active=move || active.get() == OnchainConfigTask::Alerts
                on:click=move |_| active.set(OnchainConfigTask::Alerts)
            >
                <span>"提醒"</span>
                <small class=move || alert_status.get().tone>{move || alert_status.get().label}</small>
            </button>
        </div>
    }
    .into_any()
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TabStatus {
    label: &'static str,
    tone: &'static str,
}

const fn tab_status(label: &'static str, tone: &'static str) -> TabStatus {
    TabStatus { label, tone }
}

fn market_tab_status(draft: OnchainConfigDraft, data: OnchainData) -> TabStatus {
    data.state.with(|state| {
        let Some(snapshot) = state.value() else {
            return tab_status("读取", "is-neutral");
        };
        if !draft.matches_applied_config(&snapshot.config) {
            return tab_status("草稿", "is-warning");
        }
        if !snapshot.config.enabled {
            return tab_status("暂停", "is-neutral");
        }
        match snapshot.quality {
            OnchainComparisonQuality::Pending => tab_status("读取", "is-neutral"),
            OnchainComparisonQuality::ValuationPending => tab_status("估值", "is-warning"),
            OnchainComparisonQuality::Stale => tab_status("过期", "is-warning"),
            OnchainComparisonQuality::MappingInvalid => tab_status("阻断", "is-danger"),
            OnchainComparisonQuality::UpstreamUnavailable => tab_status("断流", "is-danger"),
            OnchainComparisonQuality::RawCrossQuote
            | OnchainComparisonQuality::RawCustomPair
            | OnchainComparisonQuality::LowLiquidity => tab_status("观察", "is-warning"),
            OnchainComparisonQuality::Fresh | OnchainComparisonQuality::NoNetProfit => {
                tab_status("实时", "is-positive")
            }
            OnchainComparisonQuality::Disabled => tab_status("暂停", "is-neutral"),
        }
    })
}

fn access_tab_status(data: OnchainData) -> TabStatus {
    data.state.with(|state| {
        let Some(snapshot) = state.value() else {
            return tab_status("读取", "is-neutral");
        };
        if !snapshot.execution_readiness.wallet_address_configured {
            tab_status("钱包待填", "is-warning")
        } else if !snapshot.execution_readiness.chain_submission_ready {
            tab_status("签名待接", "is-warning")
        } else {
            tab_status("就绪", "is-positive")
        }
    })
}

fn alert_tab_status(draft: OnchainConfigDraft, data: OnchainData) -> TabStatus {
    data.state.with(|state| {
        let Some(snapshot) = state.value() else {
            return tab_status("读取", "is-neutral");
        };
        if !draft.alert_matches_applied_config(&snapshot.config) {
            return tab_status("草稿", "is-warning");
        }
        if !snapshot.config.spread_alert.enabled {
            return tab_status("已关", "is-neutral");
        }
        if webhook_ready(&data.webhook_status.get()) {
            tab_status("实时", "is-positive")
        } else {
            tab_status("待推送", "is-warning")
        }
    })
}

fn selected(active: RwSignal<OnchainConfigTask>, task: OnchainConfigTask) -> &'static str {
    if active.get() == task {
        "true"
    } else {
        "false"
    }
}

fn tab_index(active: RwSignal<OnchainConfigTask>, task: OnchainConfigTask) -> i32 {
    if active.get() == task {
        0
    } else {
        -1
    }
}
