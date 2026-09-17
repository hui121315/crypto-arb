use leptos::prelude::*;

use super::super::components::snapshot_transport::SnapshotTransport;
use super::super::components::snapshot_transport_chip;
use super::super::data::PortfolioAccountAccess;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PositionsDetailTab {
    Positions,
    Accounts,
    Balances,
    Risk,
    Activity,
    Controls,
}

impl PositionsDetailTab {
    const fn next(self) -> Self {
        match self {
            Self::Positions => Self::Balances,
            Self::Balances => Self::Risk,
            Self::Risk => Self::Activity,
            Self::Activity => Self::Accounts,
            Self::Accounts => Self::Controls,
            Self::Controls => Self::Positions,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::Positions => Self::Controls,
            Self::Balances => Self::Positions,
            Self::Risk => Self::Balances,
            Self::Activity => Self::Risk,
            Self::Accounts => Self::Activity,
            Self::Controls => Self::Accounts,
        }
    }
}

pub(super) fn positions_command_header(
    transport: Memo<SnapshotTransport>,
    account_access: Memo<PortfolioAccountAccess>,
) -> impl IntoView {
    view! {
        <header class="positions-command-header">
            <div class="positions-command-identity">
                <h1>"持仓/风控"</h1>
            </div>
            {snapshot_transport_chip(transport)}
            {move || {
                let access = account_access.get();
                let configured = access.configured_venues.len();
                let total = configured.saturating_add(access.unconfigured_venues.len());
                view! {
                    <div class="positions-command-coverage">
                        <span>"账户覆盖"</span>
                        <strong class="num">{format!("{configured}/{total}")}</strong>
                        <em>{if access.coverage_incomplete() { "部分接入" } else { "覆盖完整" }}</em>
                    </div>
                }
            }}
        </header>
    }
}

pub(super) fn positions_detail_tabs(active: RwSignal<PositionsDetailTab>) -> impl IntoView {
    let positions_ref = NodeRef::<leptos::html::Button>::new();
    let balances_ref = NodeRef::<leptos::html::Button>::new();
    let risk_ref = NodeRef::<leptos::html::Button>::new();
    let activity_ref = NodeRef::<leptos::html::Button>::new();
    let accounts_ref = NodeRef::<leptos::html::Button>::new();
    let controls_ref = NodeRef::<leptos::html::Button>::new();
    view! {
        <div
            class="positions-detail-tabs"
            role="tablist"
            aria-label="持仓与风控明细"
            aria-orientation="horizontal"
            on:keydown=move |event| {
                let next = match event.key().as_str() {
                    "ArrowRight" => Some(active.get().next()),
                    "ArrowLeft" => Some(active.get().previous()),
                    "Home" => Some(PositionsDetailTab::Positions),
                    "End" => Some(PositionsDetailTab::Controls),
                    _ => None,
                };
                let Some(next) = next else { return };
                event.prevent_default();
                active.set(next);
                let target = match next {
                    PositionsDetailTab::Positions => positions_ref,
                    PositionsDetailTab::Balances => balances_ref,
                    PositionsDetailTab::Risk => risk_ref,
                    PositionsDetailTab::Activity => activity_ref,
                    PositionsDetailTab::Accounts => accounts_ref,
                    PositionsDetailTab::Controls => controls_ref,
                };
                if let Some(button) = target.get() {
                    let _ = button.focus();
                }
            }
        >
            <button
                id="positions-tab-positions"
                type="button"
                role="tab"
                node_ref=positions_ref
                aria-controls="positions-detail-positions"
                aria-selected=move || if active.get() == PositionsDetailTab::Positions { "true" } else { "false" }
                tabindex=move || if active.get() == PositionsDetailTab::Positions { 0 } else { -1 }
                class:active=move || active.get() == PositionsDetailTab::Positions
                on:click=move |_| active.set(PositionsDetailTab::Positions)
            >"持仓"</button>
            <button
                id="positions-tab-balances"
                type="button"
                role="tab"
                node_ref=balances_ref
                aria-controls="positions-detail-balances"
                aria-selected=move || if active.get() == PositionsDetailTab::Balances { "true" } else { "false" }
                tabindex=move || if active.get() == PositionsDetailTab::Balances { 0 } else { -1 }
                class:active=move || active.get() == PositionsDetailTab::Balances
                on:click=move |_| active.set(PositionsDetailTab::Balances)
            >"资产"</button>
            <button
                id="positions-tab-risk"
                type="button"
                role="tab"
                node_ref=risk_ref
                aria-controls="positions-detail-risk"
                aria-selected=move || if active.get() == PositionsDetailTab::Risk { "true" } else { "false" }
                tabindex=move || if active.get() == PositionsDetailTab::Risk { 0 } else { -1 }
                class:active=move || active.get() == PositionsDetailTab::Risk
                on:click=move |_| active.set(PositionsDetailTab::Risk)
            >"风险"</button>
            <button
                id="positions-tab-activity"
                type="button"
                role="tab"
                node_ref=activity_ref
                aria-controls="positions-detail-activity"
                aria-selected=move || if active.get() == PositionsDetailTab::Activity { "true" } else { "false" }
                tabindex=move || if active.get() == PositionsDetailTab::Activity { 0 } else { -1 }
                class:active=move || active.get() == PositionsDetailTab::Activity
                on:click=move |_| active.set(PositionsDetailTab::Activity)
            >"平仓"</button>
            <button
                id="positions-tab-accounts"
                type="button"
                role="tab"
                node_ref=accounts_ref
                aria-controls="positions-detail-accounts"
                aria-selected=move || if active.get() == PositionsDetailTab::Accounts { "true" } else { "false" }
                tabindex=move || if active.get() == PositionsDetailTab::Accounts { 0 } else { -1 }
                class:active=move || active.get() == PositionsDetailTab::Accounts
                on:click=move |_| active.set(PositionsDetailTab::Accounts)
            >"接入"</button>
            <button
                id="positions-tab-controls"
                type="button"
                role="tab"
                node_ref=controls_ref
                aria-controls="positions-detail-controls"
                aria-selected=move || if active.get() == PositionsDetailTab::Controls { "true" } else { "false" }
                tabindex=move || if active.get() == PositionsDetailTab::Controls { 0 } else { -1 }
                class:active=move || active.get() == PositionsDetailTab::Controls
                on:click=move |_| active.set(PositionsDetailTab::Controls)
            >"控制"</button>
        </div>
    }
}
