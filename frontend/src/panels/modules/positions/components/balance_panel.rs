//! 余额面板组件装配：余额行/证据 chip/字段质量/数据健康渲染。
//! 纯派生（筛选/匹配/文案/样式）见 `derive.rs`，单测见 `testing.rs`。

#[path = "balance_panel/derive.rs"]
mod derive;
#[path = "balance_panel/evidence.rs"]
mod evidence;
#[cfg(test)]
#[path = "balance_panel/testing.rs"]
mod testing;
#[path = "balance_panel/workbench.rs"]
mod workbench;

use leptos::prelude::*;
use shared_types::{
    AccountDataHealth, AccountFieldQuality, VenueAccountSummary, VenueAssetValuation,
    VenueBalanceInfo, VenueOperationHealth,
};

use super::super::data::PortfolioAccountAccess;
use super::account_evidence::{render_account_surface_evidence, AccountSurfaceEvidence};
use super::account_setup::account_data_placeholder;
use super::format::{precise_money, quantity, signed_money};
use super::section_state::SectionData;
use derive::{
    account_level_quality_rows, balance_field_quality_rows, balance_groups, balance_health_rows,
    balance_quality_for_row, balance_row_health_for_row, utilization_pct, BalanceDisplayRow,
    VenueBalanceGroup,
};
use evidence::{
    render_account_field_quality, render_balance_data_health, render_balance_diagnostics,
    render_balance_evidence, render_balance_row_diagnostics,
};
use workbench::{balance_account_workbench, balance_status_note};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct BalancePanelInput {
    pub(in crate::panels::modules::positions) balances: Memo<SectionData<Vec<VenueBalanceInfo>>>,
    pub(in crate::panels::modules::positions) asset_valuations: Memo<Vec<VenueAssetValuation>>,
    pub(in crate::panels::modules::positions) account_summaries: Memo<Vec<VenueAccountSummary>>,
    pub(in crate::panels::modules::positions) operation_health: Memo<Vec<VenueOperationHealth>>,
    pub(in crate::panels::modules::positions) field_quality: Memo<Vec<AccountFieldQuality>>,
    pub(in crate::panels::modules::positions) row_health: Memo<Vec<AccountDataHealth>>,
    pub(in crate::panels::modules::positions) account_evidence:
        Memo<Option<AccountSurfaceEvidence>>,
    pub(in crate::panels::modules::positions) account_access: Memo<PortfolioAccountAccess>,
}

pub(in crate::panels::modules::positions) fn balance_panel(
    input: BalancePanelInput,
) -> impl IntoView {
    let selected_venue = RwSignal::new(None::<String>);
    let BalancePanelInput {
        balances,
        asset_valuations,
        account_summaries,
        operation_health,
        field_quality,
        row_health,
        account_evidence,
        account_access,
    } = input;
    let quality = Memo::new(move |_| balance_field_quality_rows(field_quality.get()));
    let groups = Memo::new(move |_| {
        balance_groups(
            balances.get().value,
            asset_valuations.get(),
            account_summaries.get(),
            &quality.get(),
        )
    });
    let has_groups = Memo::new(move |_| !groups.get().is_empty());
    let unavailable = Memo::new(move |_| account_access.get().account_data_unavailable());
    view! {
        <div class="balance-panel">
            {move || if unavailable.get() {
                ().into_any()
            } else {
                render_account_surface_evidence(account_evidence.get())
            }}
            <Show when=move || !unavailable.get() fallback=move || {
                    account_data_placeholder(
                        "余额等待账户接入",
                        "配置账户读取权限后显示可用余额、占用保证金与未实现盈亏。",
                    )
            }>
                <Show when=move || has_groups.get() fallback=move || empty_balance(
                    &balances.get(), balance_health_rows(operation_health.get()), &quality.get(), &row_health.get(),
                )>
                    {move || balances.get().status.stale_note("余额刷新失败，显示上次快照").map(balance_status_note)}
                    {balance_account_workbench(groups, quality, row_health, selected_venue)}
                    {move || render_balance_diagnostics(balance_health_rows(operation_health.get()), &account_level_quality_rows(&quality.get()))}
                </Show>
            </Show>
        </div>
    }
}

fn empty_balance(
    section: &SectionData<Vec<VenueBalanceInfo>>,
    health: Vec<VenueOperationHealth>,
    quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
) -> AnyView {
    let text = section
        .status
        .empty_text("暂无可用余额", "读取余额中", "余额读取失败");
    view! {
        {render_balance_evidence(health)}
        {render_account_field_quality(quality)}
        {render_balance_data_health(row_health)}
        <div class="risk-empty">{text}</div>
    }
    .into_any()
}

fn balance_group(
    group: VenueBalanceGroup,
    field_quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
    unvalued_open: RwSignal<bool>,
) -> AnyView {
    let total_count = group.rows.len().saturating_add(group.hidden_dust_count);
    let account_meta = group
        .summary
        .as_ref()
        .map(|summary| summary.account_type.trim().to_ascii_uppercase())
        .filter(|account_type| !account_type.is_empty())
        .unwrap_or_else(|| "账户".to_owned());
    let meta = if group.unknown_valuation_count == 0 {
        format!("{account_meta} · {total_count} 项资产")
    } else {
        format!(
            "{account_meta} · {total_count} 项资产 · {} 项待估值",
            group.unknown_valuation_count
        )
    };
    let equity = group
        .summary
        .as_ref()
        .filter(|summary| summary.total_equity_usd.is_finite())
        .map(|summary| precise_money(summary.total_equity_usd));
    let equity_unknown = equity.is_none();
    let equity_label = equity.unwrap_or_else(|| "待确认".to_owned());
    let venue = group.venue.trim().to_ascii_uppercase();
    let (valued_rows, unknown_rows): (Vec<_>, Vec<_>) = group
        .rows
        .into_iter()
        .partition(|row| row.valuation.is_some());
    let hidden_dust_count = group.hidden_dust_count;
    view! {
        <section class="balance-venue-group">
            <header class="balance-venue-header">
                <div>
                    <strong>{venue}</strong>
                    <span>{meta}</span>
                </div>
                <div>
                    <span>"账户权益"</span>
                    <strong class:unknown=equity_unknown>
                        {equity_label}
                    </strong>
                </div>
            </header>
            {if valued_rows.is_empty() {
                view! {
                    <div class="balance-group-empty">"暂无不低于 $1 的已估值资产"</div>
                }
                .into_any()
            } else {
                render_balance_table(valued_rows, field_quality, row_health)
            }}
            {render_unknown_valuations(unknown_rows, field_quality, row_health, unvalued_open)}
            {render_hidden_dust(hidden_dust_count)}
        </section>
    }
    .into_any()
}

fn render_unknown_valuations(
    rows: Vec<BalanceDisplayRow>,
    field_quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
    open: RwSignal<bool>,
) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    let count = rows.len();
    view! {
        <details class="balance-unvalued-disclosure" open=move || open.get()>
            <summary on:click=move |event| { event.prevent_default(); open.update(|value| *value = !*value); }>
                <span>"待估值资产"</span>
                <strong>{format!("{count} 项 · 展开原始余额")}</strong>
            </summary>
            {render_balance_table(rows, field_quality, row_health)}
        </details>
    }
    .into_any()
}

fn render_balance_table(
    rows: Vec<BalanceDisplayRow>,
    field_quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
) -> AnyView {
    view! {
        <div class="balance-table" role="table" aria-label="交易所资产余额">
            <div class="balance-table-head" role="row">
                <span>"资产"</span>
                <span>"美元估值"</span>
                <span>"总额"</span>
                <span>"可用"</span>
                <span>"占用 / 未实现"</span>
            </div>
            {rows.into_iter().map(|display| {
                let quality = balance_quality_for_row(&display.balance, field_quality);
                let health = balance_row_health_for_row(&display.balance, row_health);
                balance_row(display, &quality, &health)
            }).collect_view()}
        </div>
    }
    .into_any()
}

fn balance_row(
    display: BalanceDisplayRow,
    field_quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
) -> impl IntoView {
    let row = display.balance;
    let utilization = utilization_pct(&row);
    let total = balance_number(row.total, "total", field_quality, false);
    let available = balance_number(row.available, "available", field_quality, false);
    let frozen = balance_number(row.frozen, "frozen", field_quality, false);
    let pnl = balance_number(row.unrealized_pnl, "unrealizedPnl", field_quality, true);
    let utilization_known = field_quality
        .iter()
        .all(|q| !matches!(q.field.as_str(), "total" | "frozen"));
    let valuation = display.valuation;
    let valuation_title = valuation
        .as_ref()
        .map(|value| format!("{} · {}", value.source, value.observed_at_ms))
        .unwrap_or_else(|| "交易所未提供可信逐币种美元估值".to_owned());
    let valuation_label = valuation
        .as_ref()
        .map(|value| precise_money(value.usd_value))
        .unwrap_or_else(|| "待估值".to_owned());
    let valuation_unknown = valuation.is_none();
    view! {
        <div class="balance-row" role="row">
            <div class="balance-asset-cell" role="cell">
                <strong>{row.currency.clone()}</strong>
                <span>{row.venue.clone()}</span>
            </div>
            <div class="balance-value-cell" role="cell" title=valuation_title>
                <strong class:unknown=valuation_unknown>{valuation_label}</strong>
                <span>"美元估值"</span>
            </div>
            <div class="balance-amount-cell" role="cell">
                <strong>{total}</strong>
                <span>"总额"</span>
            </div>
            <div class="balance-amount-cell" role="cell">
                <strong>{available}</strong>
                <span>"可用"</span>
            </div>
            <div class="balance-amount-cell" role="cell">
                <strong>{frozen}</strong>
                <span>{pnl}</span>
            </div>
            <div class="balance-row-meter meter" hidden=!utilization_known>
                <i style=format!("width:{utilization:.1}%;")></i>
            </div>
            {render_balance_row_diagnostics(field_quality, row_health)}
        </div>
    }
}

fn balance_number(
    value: f64,
    field: &str,
    quality: &[AccountFieldQuality],
    signed: bool,
) -> String {
    let status = quality.iter().find(|q| q.field == field).map(|q| q.status);
    if !value.is_finite()
        || matches!(
            status,
            Some(
                shared_types::AccountFieldQualityStatus::Unknown
                    | shared_types::AccountFieldQualityStatus::Missing
                    | shared_types::AccountFieldQualityStatus::Invalid
            )
        )
    {
        return "未知".to_owned();
    }
    let value = if signed {
        signed_money(value)
    } else {
        quantity(value)
    };
    if status == Some(shared_types::AccountFieldQualityStatus::Estimated) {
        format!("约 {value}")
    } else {
        value
    }
}

fn render_hidden_dust(count: usize) -> AnyView {
    if count == 0 {
        return ().into_any();
    }
    view! {
        <div class="balance-dust-note">
            <span>"小额资产已隐藏"</span>
            <strong>{format!("{count} 项 · 单项 < $1")}</strong>
        </div>
    }
    .into_any()
}
