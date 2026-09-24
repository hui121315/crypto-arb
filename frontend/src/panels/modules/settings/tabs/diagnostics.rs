use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    EnvTemplateLine, EnvTemplateResponse, MarketCacheAccessRow, MarketDataDiagnosticsSnapshot,
    MarketDataRowEvidence, MarketDataSnapshotStatusRow, VenueOperationEvidence,
    VenueOperationHealth, VenueOperationHealthSnapshot, VenueOperationKind, VenueOperationStatus,
    VenueRuntimeHealthSnapshot, VenueRuntimeOperationHealth,
};
use std::cmp::Ordering;

use super::super::data::{
    api_auth_configured, current_api_base, save_api_auth_token, save_api_base, settings_state,
    settings_value, use_api_base_validate_action, use_env_template, use_funding_rates_diagnostics,
    use_market_data_diagnostics, use_spot_debug_query, use_trading_status,
    use_venue_operation_health, use_venue_runtime_health, use_watchlist_alert_state,
    ApiBaseValidateAction, SettingsResource,
};
use super::state_view::action_message;
use super::{problem_cell, problem_message};
use crate::panels::modules::market_evidence::{
    market_health_label, market_quality_label, market_source_label,
};
use crate::panels::modules::pagination::{page_controls, use_table_runtime, TableRuntimeHandle};
use crate::state::module_runtime::persisted_choice_signal;

const ENV_TEMPLATE_PAGE_SIZE: usize = 20;
const HEALTH_PAGE_SIZE: usize = 20;
const ACCESS_PAGE_SIZE: usize = 16;
const STATUS_PAGE_SIZE: usize = 12;
const ROW_EVIDENCE_PAGE_SIZE: usize = 20;
const DIAGNOSTICS_ENV_PAGE_KEY: &str = "crossline.settings.diagnostics.env.page";
const DIAGNOSTICS_HEALTH_PAGE_KEY: &str = "crossline.settings.diagnostics.health.page";
const DIAGNOSTICS_ACCESS_PAGE_KEY: &str = "crossline.settings.diagnostics.marketAccess.page";
const DIAGNOSTICS_STATUS_PAGE_KEY: &str = "crossline.settings.diagnostics.marketStatus.page";
const DIAGNOSTICS_ROW_EVIDENCE_PAGE_KEY: &str = "crossline.settings.diagnostics.rowEvidence.page";
const DIAGNOSTICS_HEALTH_QUERY_KEY: &str = "crossline.settings.diagnostics.health.query";
const DIAGNOSTICS_HEALTH_STATUS_KEY: &str = "crossline.settings.diagnostics.health.status";
const API_BASE_APPLY_CONFIRMATION: &str = "apply";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DiagnosticsTask {
    #[default]
    Connection,
    Market,
    Trading,
    RuntimeEvidence,
}

pub(in crate::panels::modules::settings) fn diagnostics_tab(
    execution_selection: RwSignal<crate::panels::modules::execution::selection::ExecutionSelection>,
) -> impl IntoView {
    let app_context = expect_context::<crate::state::AppContext>();
    let refresh_nonce = RwSignal::new(0_u64);
    let template = use_env_template(refresh_nonce);
    let operation_health = use_venue_operation_health(refresh_nonce);
    let runtime_health = use_venue_runtime_health(refresh_nonce);
    let trading_status = use_trading_status(refresh_nonce);
    let market_diagnostics = use_market_data_diagnostics(refresh_nonce);
    let funding_rates = use_funding_rates_diagnostics(refresh_nonce);
    let watchlist_alerts = use_watchlist_alert_state();
    let api_base = RwSignal::new(current_api_base());
    let api_auth_token = RwSignal::new(String::new());
    let auth_configured = RwSignal::new(api_auth_configured(app_context.api_auth_token));
    let health_query = persisted_choice_signal(
        DIAGNOSTICS_HEALTH_QUERY_KEY,
        String::new(),
        stored_health_query,
        Clone::clone,
    );
    let health_status_filter = persisted_choice_signal(
        DIAGNOSTICS_HEALTH_STATUS_KEY,
        HealthStatusFilter::All,
        stored_health_status_filter,
        |filter| filter.as_key().to_owned(),
    );
    let apply_confirm = RwSignal::new(String::new());
    let validate_action = use_api_base_validate_action();
    let spot_symbol = RwSignal::new(String::new());
    let spot_query = use_spot_debug_query();
    let message = RwSignal::new(String::new());
    let tables = diagnostics_tables(
        template,
        operation_health,
        market_diagnostics,
        funding_rates,
        health_query,
        health_status_filter,
    );
    let env_table = tables.env;
    let health_table = tables.health;
    let access_table = tables.access;
    let status_table = tables.status;
    let row_evidence_table = tables.row_evidence;
    let active = RwSignal::new(DiagnosticsTask::default());

    view! {
        <div class="settings-stack">
            <div class="settings-task-tabs is-four" role="tablist" aria-label="诊断范围">
                {diagnostics_task_tab("连接", DiagnosticsTask::Connection, active)}
                {diagnostics_task_tab("行情", DiagnosticsTask::Market, active)}
                {diagnostics_task_tab("交易", DiagnosticsTask::Trading, active)}
                {diagnostics_task_tab("运行证据", DiagnosticsTask::RuntimeEvidence, active)}
            </div>
            <div class="settings-actions">
                <button type="button" class="row-action" on:click=move |_| {
                    refresh_nonce.update(|value| *value = value.wrapping_add(1));
                    message.set("已请求刷新全部诊断".to_owned());
                }>"刷新全部诊断"</button>
            </div>
            <em class="settings-message">{move || message.get()}</em>
            <section
                class="settings-task-panel settings-stack"
                role="tabpanel"
                aria-label="后端连接诊断"
                hidden=move || active.get() != DiagnosticsTask::Connection
            >
                {api_runtime_editor(ApiRuntimeEditorState {
                    api_base,
                    apply_confirm,
                    api_auth_token,
                    auth_configured,
                    runtime_api_base: app_context.api_base,
                    runtime_api_auth_token: app_context.api_auth_token,
                    refresh_nonce,
                    message,
                    validate_action,
                })}
            </section>
            <section
                class="settings-task-panel settings-stack"
                role="tabpanel"
                aria-label="行情诊断"
                hidden=move || active.get() != DiagnosticsTask::Market
            >
                {move || {
                    market_diagnostics_panel(
                        settings_state(market_diagnostics),
                        &access_table,
                        &status_table,
                    )
                }}
                {move || row_evidence_panel(settings_state(funding_rates), &row_evidence_table)}
                {spot_debug_panel(spot_symbol, spot_query)}
            </section>
            <section
                class="settings-task-panel settings-stack"
                role="tabpanel"
                aria-label="交易诊断"
                hidden=move || active.get() != DiagnosticsTask::Trading
            >
                {move || {
                    let trading_state = settings_state(trading_status);
                    let runtime_state = settings_state(runtime_health);
                    ticket_venue_health_panel(
                        &trading_state,
                        &runtime_state,
                        execution_selection.get().venue_pair(),
                    )
                }}
                {move || venue_runtime_health_panel(settings_state(runtime_health))}
            </section>
            <section
                class="settings-task-panel settings-stack"
                role="tabpanel"
                aria-label="运行证据诊断"
                hidden=move || active.get() != DiagnosticsTask::RuntimeEvidence
            >
                {move || ws_rtt_explain_panel(settings_state(operation_health))}
                {watchlist_alert_runtime_panel(watchlist_alerts)}
                {operation::operation_health_filters(health_query, health_status_filter)}
                {move || {
                    operation_health_panel(
                        settings_state(operation_health),
                        &health_table,
                        health_query,
                        health_status_filter,
                    )
                }}
                {move || env_template_panel(settings_state(template), &env_table)}
            </section>
        </div>
    }
}

fn diagnostics_task_tab(
    label: &'static str,
    task: DiagnosticsTask,
    active: RwSignal<DiagnosticsTask>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            role="tab"
            aria-selected=move || active.get() == task
            class=move || if active.get() == task { "active" } else { "" }
            on:click=move |_| active.set(task)
        >
            {label}
        </button>
    }
}

#[derive(Clone, Copy)]
struct ApiRuntimeEditorState {
    api_base: RwSignal<String>,
    apply_confirm: RwSignal<String>,
    api_auth_token: RwSignal<String>,
    auth_configured: RwSignal<bool>,
    runtime_api_base: RwSignal<String>,
    runtime_api_auth_token: RwSignal<String>,
    refresh_nonce: RwSignal<u64>,
    message: RwSignal<String>,
    validate_action: ApiBaseValidateAction,
}

fn api_runtime_editor(state: ApiRuntimeEditorState) -> impl IntoView {
    let ApiRuntimeEditorState {
        api_base,
        apply_confirm,
        api_auth_token,
        auth_configured,
        runtime_api_base,
        runtime_api_auth_token,
        refresh_nonce,
        message,
        validate_action,
    } = state;
    view! {
        <div class="api-base-editor settings-api-runtime-editor">
            {api_base_task(
                api_base,
                apply_confirm,
                runtime_api_base,
                refresh_nonce,
                message,
                validate_action,
            )}
            {api_token_task(
                api_auth_token,
                auth_configured,
                runtime_api_auth_token,
                refresh_nonce,
                message,
            )}
        </div>
    }
}

fn api_base_task(
    api_base: RwSignal<String>,
    apply_confirm: RwSignal<String>,
    runtime_api_base: RwSignal<String>,
    refresh_nonce: RwSignal<u64>,
    message: RwSignal<String>,
    validate_action: ApiBaseValidateAction,
) -> impl IntoView {
    view! {
        <section class="settings-api-task">
            <header class="settings-api-task-header">
                <div><strong>"后端地址"</strong><span>"REST / WS 共用运行地址"</span></div>
                <em title=move || runtime_api_base.get()>{move || runtime_api_base.get()}</em>
            </header>
            <div class="settings-api-fields">
                <label>
                    <span>"API Base"</span>
                    <input
                        disabled=move || validate_action.state.get().is_pending()
                        prop:value=move || api_base.get()
                        on:input=move |ev| {
                            api_base.set(event_target_value(&ev));
                            validate_action.state.set(crate::state::action_state::ActionState::Idle);
                        }
                    />
                </label>
                <label>
                    <span>"确认应用"</span>
                    <input
                        placeholder=API_BASE_APPLY_CONFIRMATION
                        prop:value=move || apply_confirm.get()
                        on:input=move |ev| apply_confirm.set(event_target_value(&ev))
                    />
                </label>
            </div>
            <div class="settings-api-actions">
                {api_base_validate_button(api_base, validate_action)}
                <button
                    type="button"
                    class=move || if api_base_apply_ready(&api_base.get(), &apply_confirm.get()) {
                        "primary-blue settings-apply-action"
                    } else {
                        "row-action settings-apply-action"
                    }
                    disabled=move || validate_action.state.get().is_pending() || !api_base_apply_ready(&api_base.get(), &apply_confirm.get())
                    on:click=move |_| {
                        let next = normalized(&api_base.get_untracked());
                        if next.is_empty() {
                            return;
                        }
                        save_api_base(runtime_api_base, &next);
                        refresh_nonce.update(|value| *value = value.wrapping_add(1));
                        apply_confirm.set(String::new());
                        message.set("API Base 已应用，REST/WS 正在使用新地址刷新诊断。".into());
                    }
                >
                    "保存并应用"
                </button>
            </div>
            <div class="settings-api-feedback">
                <em>"先验证目标地址；输入 apply 后才会切换 REST / WS 运行时。"</em>
                {api_base_validate_status(validate_action)}
            </div>
        </section>
    }
}

fn api_base_apply_ready(api_base: &str, apply_confirm: &str) -> bool {
    !normalized(api_base).is_empty() && api_base_apply_confirmed(apply_confirm)
}

fn api_token_task(
    api_auth_token: RwSignal<String>,
    auth_configured: RwSignal<bool>,
    runtime_api_auth_token: RwSignal<String>,
    refresh_nonce: RwSignal<u64>,
    message: RwSignal<String>,
) -> impl IntoView {
    view! {
        <section class="settings-api-task settings-api-token-task">
            <header class="settings-api-task-header">
                <div><strong>"后端 Token"</strong><span>"REST Bearer / WS ticket 鉴权"</span></div>
                <em>{move || auth_status_label(auth_configured.get())}</em>
            </header>
            <div class="settings-api-fields">
                <label>
                    <span>"Token"</span>
                    <input
                        type="password"
                        autocomplete="current-password"
                        placeholder=move || auth_status_label(auth_configured.get())
                        prop:value=move || api_auth_token.get()
                        on:input=move |ev| api_auth_token.set(event_target_value(&ev))
                    />
                </label>
            </div>
            <div class="settings-api-actions">
                <button
                    type="button"
                    class="row-action"
                    disabled=move || api_auth_token.get().trim().is_empty()
                    on:click=move |_| {
                        let configured = save_api_auth_token(
                            runtime_api_auth_token,
                            &api_auth_token.get_untracked(),
                        );
                        api_auth_token.set(String::new());
                        auth_configured.set(configured);
                        refresh_nonce.update(|value| *value = value.wrapping_add(1));
                        message.set(auth_apply_message(configured));
                    }
                >
                    "保存 Token"
                </button>
                <button
                    type="button"
                    class="row-action settings-destructive-action"
                    disabled=move || !auth_configured.get()
                    on:click=move |_| {
                        let configured = save_api_auth_token(runtime_api_auth_token, "");
                        api_auth_token.set(String::new());
                        auth_configured.set(configured);
                        refresh_nonce.update(|value| *value = value.wrapping_add(1));
                        message.set(auth_apply_message(configured));
                    }
                >
                    "清空 Token"
                </button>
            </div>
            <div class="settings-api-feedback">
                <em>{move || format!("当前鉴权：{}", auth_status_label(auth_configured.get()))}</em>
            </div>
        </section>
    }
}

fn api_base_validate_button(
    api_base: RwSignal<String>,
    validate_action: ApiBaseValidateAction,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="row-action"
            disabled=move || normalized(&api_base.get()).is_empty()
                || validate_action.state.get().is_pending()
            on:click=move |_| {
                let candidate = normalized(&api_base.get_untracked());
                if candidate.is_empty() {
                    return;
                }
                validate_action.submit.run(candidate);
            }
        >
            "验证连通"
        </button>
    }
}

fn api_base_validate_status(validate_action: ApiBaseValidateAction) -> impl IntoView {
    view! {
        <em>{move || {
            action_message(
                "验证连通会请求目标地址的 /api/system/health，不会切换运行时。",
                &validate_action.state.get(),
            )
        }}</em>
    }
}

mod env;
mod filter;
mod market;
mod operation;
mod operation_text;
mod runtime_matrix;
mod spot_debug;
mod spot_debug_text;
mod tables;
#[cfg(test)]
mod tests;
mod ticket_health;
mod watchlist_alerts;
mod ws_rtt;

use env::*;
use filter::*;
use market::*;
use operation::*;
use operation_text::*;
use runtime_matrix::*;
use spot_debug::*;
use tables::*;
use ticket_health::*;
use watchlist_alerts::*;
use ws_rtt::*;
