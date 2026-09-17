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
    view! {
        <div class="balance-panel">
            {move || if account_access.get().account_data_unavailable() {
                ().into_any()
            } else {
                render_account_surface_evidence(account_evidence.get())
            }}
            {move || {
                if account_access.get().account_data_unavailable() {
                    return account_data_placeholder(
                        "余额等待账户接入",
                        "配置账户读取权限后显示可用余额、占用保证金与未实现盈亏。",
                    );
                }
                let section = balances.get();
                let health = balance_health_rows(operation_health.get());
                let quality = balance_field_quality_rows(field_quality.get());
                let rows_health = row_health.get();
                if section.value.is_empty() {
                    return empty_balance(&section, health, &quality, &rows_health);
                }
                let groups = balance_groups(
                    section.value,
                    asset_valuations.get(),
                    account_summaries.get(),
                );
                render_balances(
                    groups,
                    section.status.stale_note("余额刷新失败，显示上次快照"),
                    health,
                    &quality,
                    &rows_health,
                    selected_venue,
                )
            }}
        </div>
    }
}

fn render_balances(
    groups: Vec<VenueBalanceGroup>,
    stale_note: Option<String>,
    health: Vec<VenueOperationHealth>,
    quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
    selected_venue: RwSignal<Option<String>>,
) -> AnyView {
    view! {
        {stale_note.map(balance_status_note)}
        {balance_account_workbench(groups, quality, row_health, selected_venue)}
        {render_balance_diagnostics(health, &account_level_quality_rows(quality))}
    }
    .into_any()
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
            {render_unknown_valuations(unknown_rows, field_quality, row_health)}
            {render_hidden_dust(hidden_dust_count)}
        </section>
    }
    .into_any()
}

fn render_unknown_valuations(
    rows: Vec<BalanceDisplayRow>,
    field_quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    let count = rows.len();
    view! {
        <details class="balance-unvalued-disclosure">
            <summary>
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
                <strong>{quantity(row.total)}</strong>
                <span>"总额"</span>
            </div>
            <div class="balance-amount-cell" role="cell">
                <strong>{quantity(row.available)}</strong>
                <span>"可用"</span>
            </div>
            <div class="balance-amount-cell" role="cell">
                <strong>{quantity(row.frozen)}</strong>
                <span>{signed_money(row.unrealized_pnl)}</span>
            </div>
            <div class="balance-row-meter meter">
                <i style=format!("width:{utilization:.1}%;")></i>
            </div>
            {render_balance_row_diagnostics(field_quality, row_health)}
        </div>
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
