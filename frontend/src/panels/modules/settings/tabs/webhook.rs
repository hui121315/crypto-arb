use crate::panels::shared::webhook_delivery_message;
use crate::panels::shared::webhook_monitor::delivery_label;
use crate::state::load_state::LoadState;
use crate::state::{action_state::ActionState, module_runtime::ModuleRuntimeState};
use super::super::runtime::{action_health, PaneState};
use leptos::prelude::*;
use shared_types::{
    WebhookConfigPatch, WebhookEventKind, WebhookProvider,
    WebhookRuntimeStatus, WebhookTestRequest,
};

use super::super::data::{use_webhook_data, WebhookData};

mod runtime;
pub(in crate::panels::modules::settings) use runtime::{create_webhook_runtime, WebhookRuntime};

pub(in crate::panels::modules::settings) fn webhook_tab(runtime: WebhookRuntime, pane: PaneState) -> impl IntoView {
    let data = use_webhook_data(runtime.data);
    data.test_runtime.watch(data.state);
    pane.track(move || {
        let status = data.state.get();
        let config = status.value();
        let action = data.action_problem.get().map_or(ActionState::Idle, |problem| ActionState::failed("Webhook 配置操作失败", problem));
        let test = data.test_runtime.problem.get().map_or(ActionState::Idle, |problem| ActionState::failed("Webhook 测试失败", problem));
        ModuleRuntimeState::combine([
            ModuleRuntimeState::from_load_state(&status),
            ModuleRuntimeState::from_problem(config.and_then(|value| value.configuration_problem.clone())),
            if config.is_some_and(|value| !value.config.url_configured
                || value.config.provider == WebhookProvider::Generic && !value.config.secret_configured) {
                ModuleRuntimeState::setup_required()
            } else { ModuleRuntimeState::ready() },
            action_health(data.journal, &action),
            action_health(data.test_runtime.journal, &test),
        ])
    });
    let url = RwSignal::new(String::new());
    let secret = RwSignal::new(String::new());
    let WebhookRuntime { provider, timeout, attempts, backoff, capacity, event_kinds, hydrated, .. } = runtime;
    let draft = WebhookDraft {
        provider,
        url,
        secret,
        timeout,
        attempts,
        backoff,
        capacity,
        event_kinds,
    };
    let confirm_clear = RwSignal::new(false);
    let blocked =
        Memo::new(move |_| data.pending.get() || data.test_runtime.pending.get() || data.journal.locked() || !matches!(data.state.get(), LoadState::Ready(status) if status.configuration_problem.is_none()));
    Effect::new(move |_| {
        data.journal.connection.track();
        url.set(String::new()); secret.set(String::new()); confirm_clear.set(false);
    });
    on_cleanup(move || {
        if url.try_get_untracked().is_some_and(|value| !value.is_empty())
            || secret.try_get_untracked().is_some_and(|value| !value.is_empty()) {
            runtime.credentials_cleared.set(true);
        }
    });
    Effect::new(move |_| {
        if data.saved.get().is_some() {
            url.set(String::new());
            secret.set(String::new());
        }
    });
    view! {
        <div class="settings-panel webhook-settings">
            {super::super::data::settings_recovery_panel(data.journal, data.recheck)}
            <div class="settings-section-heading">
                <div><h3>"Webhook"</h3><p>{move || if data.state.with(|state| state.value().is_some_and(|status| status.configuration_problem.is_some())) {
                    "配置恢复失败，当前不可修改"
                } else { "已保存的投递配置" }}</p></div>
                <div class="webhook-heading-actions">
                    <button class="icon-button" title="刷新 Webhook 状态" aria-label="刷新 Webhook 状态" disabled=move || data.refreshing.get() || data.pending.get() on:click=move |_| data.refresh.run(())>"↻"</button>
                    <button class="row-action" disabled=move || blocked.get() on:click=move |_| toggle(data)>{move || toggle_label(&data.state.get())}</button>
                </div>
            </div>
            {status_view(data.state)}
            <Show when=move || runtime.credentials_cleared.get()>
                <p class="state-note">"离开页面后，未保存的地址和密钥已清空；其他输入仍保留。"</p>
            </Show>
            <Show when=move || !data.state.with(|state| state.value().is_some_and(|status| status.configuration_problem.is_some()))>
            <fieldset class="webhook-config-editor" disabled=move || blocked.get() || !hydrated.get()
                on:input=move |_| runtime.edit() on:change=move |_| runtime.edit()>
            <div class="webhook-core-grid">
                <label class="field-inline"><span>"投递提供方"</span><select bind:value=provider><option value="generic">"通用签名 Webhook"</option><option value="bark">"Bark 推送"</option></select></label>
                <label class="field-inline field-wide"><span>"公网 HTTPS URL"</span><input type="url" placeholder="留空保留当前地址" bind:value=url on:input=move |_| runtime.credentials_cleared.set(false) /></label>
                <label class="field-inline">
                    <span>{move || if provider.get() == "bark" { "签名密钥（Bark 不需要）" } else { "签名密钥" }}</span>
                    <input
                        type="password"
                        autocomplete="new-password"
                        placeholder=move || if provider.get() == "bark" { "Bark 使用 URL 中的 device key" } else { "通用 Webhook 必填" }
                        disabled=move || provider.get() == "bark"
                        bind:value=secret
                    />
                </label>
            </div>
            <details class="webhook-advanced-settings">
                <summary>
                    <div><strong>"事件与投递限制"</strong><span>"重试、队列与事件范围"</span></div>
                    <em>{move || format!("{}ms · {} 次 · 队列 {}", timeout.get(), attempts.get(), capacity.get())}</em>
                    <span class="webhook-details-action" aria-hidden="true"></span>
                </summary>
                <div class="webhook-advanced-grid">
                    <label class="field-inline"><span>"超时 ms"</span><input type="number" min="100" max="30000" bind:value=timeout /></label>
                    <label class="field-inline"><span>"最大尝试"</span><input type="number" min="1" max="5" bind:value=attempts /></label>
                    <label class="field-inline"><span>"退避基数 ms"</span><input type="number" min="100" max="10000" bind:value=backoff /></label>
                    <label class="field-inline"><span>"队列上限"</span><input type="number" min="1" max="256" bind:value=capacity /></label>
                    <fieldset class="webhook-event-kinds">
                        <legend>"事件类型"</legend>
                        {event_kind_options().into_iter().map(|(kind, label)| view! {
                            <label>
                                <input
                                    type="checkbox"
                                    prop:checked=move || event_kinds.with(|rows| rows.contains(&kind))
                                    on:change=move |event| event_kinds.update(|rows| {
                                        let checked = event_target_checked(&event);
                                        rows.retain(|row| row != &kind);
                                        if checked { rows.push(kind); }
                                    })
                                />
                                <span>{label}</span>
                            </label>
                        }).collect_view()}
                    </fieldset>
                </div>
            </details>
            </fieldset>
            </Show>
            <div class="webhook-settings-actions">
                <button class="workbench-save workbench-primary" disabled=move || blocked.get() on:click=move |_| save_config(data, draft)>{move || if data.pending.get() { "处理中" } else { "保存配置" }}</button>
                <button class="row-action" disabled=move || blocked.get() on:click=move |_| data.test.run(WebhookTestRequest { message: Some("CROSSLINE UI test".to_owned()) })>"发送测试"</button>
            </div>
            {crate::panels::shared::webhook_test::webhook_test_feedback(data.test_runtime)}
            {move || data.message.get().map(|message| view! { <p class="settings-message" role="status">{message}</p> })}
            {move || data.action_problem.get().map(|problem| view! { <p class="state-note is-error" role="alert">{super::problem_message("操作未确认", &problem)}</p> })}
            <details class="webhook-danger-zone">
                <summary>"停用与凭证清除"</summary>
                <div>
                    <label><input type="checkbox" bind:checked=confirm_clear disabled=move || blocked.get()/>"确认清除投递地址与密钥"</label>
                    <button class="btn-danger" disabled=move || blocked.get() || !confirm_clear.get() on:click=move |_| {
                        if !confirm_clear.get_untracked() || blocked.get_untracked() { return; }
                        confirm_clear.set(false);
                        data.update.run(WebhookConfigPatch { enabled: Some(false), url: Some(String::new()), clear_secret: Some(true), ..WebhookConfigPatch::default() });
                    }>"停用并清除"</button>
                </div>
            </details>
        </div>
    }
}

#[derive(Clone, Copy)]
struct WebhookDraft {
    provider: RwSignal<String>,
    url: RwSignal<String>,
    secret: RwSignal<String>,
    timeout: RwSignal<String>,
    attempts: RwSignal<String>,
    backoff: RwSignal<String>,
    capacity: RwSignal<String>,
    event_kinds: RwSignal<Vec<WebhookEventKind>>,
}

fn save_config(data: WebhookData, draft: WebhookDraft) {
    if data.pending.get_untracked() || !matches!(data.state.get_untracked(), LoadState::Ready(_)) {
        return;
    }
    let limits = (|| {
        Ok::<_, String>((
            parse_limit("超时", &draft.timeout.get_untracked(), 100, 30_000)?,
            parse_limit("最大尝试", &draft.attempts.get_untracked(), 1, 5)?,
            parse_limit("退避基数", &draft.backoff.get_untracked(), 100, 10_000)?,
            parse_limit("队列上限", &draft.capacity.get_untracked(), 1, 256)?,
        ))
    })();
    let (timeout, attempts, backoff, capacity) = match limits {
        Ok(limits) => limits,
        Err(message) => {
            data.action_problem.set(Some(shared_types::ApiProblem::new(
                "INVALID_WEBHOOK_CONFIG",
                message,
            )));
            return;
        }
    };
    let target_url = draft.url.get_untracked();
    let target_url = target_url.trim();
    data.save.run(WebhookConfigPatch {
        provider: Some(if draft.provider.get_untracked() == "bark" {
            WebhookProvider::Bark
        } else {
            WebhookProvider::Generic
        }),
        url: (!target_url.is_empty()).then(|| target_url.to_owned()),
        secret: (draft.provider.get_untracked() != "bark"
            && !draft.secret.get_untracked().trim().is_empty())
        .then(|| draft.secret.get_untracked()),
        event_kinds: Some(draft.event_kinds.get_untracked()),
        timeout_ms: Some(timeout),
        max_attempts: Some(attempts as u8),
        base_backoff_ms: Some(backoff),
        queue_capacity: Some(capacity as usize),
        ..WebhookConfigPatch::default()
    });
}

fn parse_limit(label: &str, value: &str, min: u64, max: u64) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| format!("{label}需要填写 {min}–{max} 之间的整数"))
}

fn event_kind_options() -> [(WebhookEventKind, &'static str); 9] {
    [
        (WebhookEventKind::Opportunity, "确定性机会"),
        (WebhookEventKind::OpportunityMonitor, "价差/充提监控"),
        (WebhookEventKind::AutomationDecision, "自动化决策"),
        (WebhookEventKind::ExecutionResult, "执行结果"),
        (WebhookEventKind::Compensation, "补偿"),
        (WebhookEventKind::RiskAlert, "风险告警"),
        (WebhookEventKind::SystemDegradation, "系统降级"),
        (WebhookEventKind::OnchainSpread, "链上价差"),
        (WebhookEventKind::StockSpread, "股票价差观察"),
    ]
}

fn toggle(data: WebhookData) {
    let enabled = data
        .state
        .with(|state| state.value().is_some_and(|value| value.config.enabled));
    data.update.run(WebhookConfigPatch {
        enabled: Some(!enabled),
        ..WebhookConfigPatch::default()
    });
}

fn toggle_label(state: &LoadState<WebhookRuntimeStatus>) -> &'static str {
    if state.value().is_some_and(|value| value.config.enabled) {
        "停用"
    } else {
        "启用"
    }
}

fn status_view(state: RwSignal<LoadState<WebhookRuntimeStatus>>) -> impl IntoView {
    let recent = Memo::new(move |_| {
        state.with(|state| {
            state
                .value()
                .map(|status| {
                    status
                        .recent_deliveries
                        .iter()
                        .take(8)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    view! {
        {move || match state.get() {
            LoadState::Loading => Some(view! { <p class="state-note">"读取 Webhook 状态…"</p> }.into_any()),
            LoadState::Error(problem) | LoadState::Stale { problem, .. } => Some(view! { <p class="state-note is-error" role="alert">{super::problem_message("Webhook 状态读取失败，旧数据仅供参考", &problem)}</p> }.into_any()),
            LoadState::Ready(_) => None,
        }}
        {move || state.with(|state| state.value().and_then(|status| status.configuration_problem.clone()))
            .map(|problem| view! { <p class="state-note is-error" role="alert">{problem.message}</p> })}
        <Show when=move || state.with(|state| state.value().is_some_and(|status| status.configuration_problem.is_none()))>
            <div class="webhook-runtime">
                <div class="webhook-summary">
                    {move || state.get().value().cloned().map(|status| view! { <>
                        <strong>{if status.config.enabled { "已启用" } else { "已停用" }}</strong>
                        <span>{provider_label(status.config.provider)}</span>
                        <span>{if status.config.url_configured { status.config.url.clone() } else { "未配置投递地址".to_owned() }}</span>
                        <span>{credential_label(&status)}</span>
                        <span>{format!("队列 {}/{}", status.queue_depth, status.config.queue_capacity)}</span>
                        <span>{format!("成功 {} · 失败 {} · 丢弃 {}", status.delivered_total, status.failed_total, status.dropped_total)}</span>
                    </> })}
                </div>
                <details class="webhook-delivery-history">
                    <summary>
                        <strong>{move || recent.with(|rows| rows.first().map_or_else(|| "尚无投递记录".into(), |row| format!("最近投递 · {}", delivery_label(row.status, row.application_ack, row.response_status))))}</strong>
                        <span>{move || format!("{} 条", recent.get().len())}</span>
                    </summary>
                    <div class="webhook-deliveries">
                        {move || recent.get().into_iter().map(|row| view! {
                            <div class="webhook-delivery-row"><strong>{format!("{:?}", row.kind)}</strong><span>{delivery_label(row.status, row.application_ack, row.response_status)}</span><span>{format!("{} 次", row.attempts)}</span><span>{webhook_delivery_message(row.error.as_deref(), row.response_message.as_deref()).unwrap_or_else(|| "无附加消息".to_owned())}</span></div>
                        }).collect_view()}
                    </div>
                </details>
            </div>
        </Show>
    }
}

fn provider_label(provider: WebhookProvider) -> &'static str {
    match provider {
        WebhookProvider::Generic => "通用签名 Webhook",
        WebhookProvider::Bark => "Bark 推送",
    }
}

fn credential_label(status: &WebhookRuntimeStatus) -> &'static str {
    match status.config.provider {
        WebhookProvider::Bark if status.config.url_configured => "Bark device key 已隐藏",
        WebhookProvider::Bark => "需要填写包含 device key 的 Bark 地址",
        WebhookProvider::Generic if status.config.secret_configured => "签名密钥已保存（不回显）",
        WebhookProvider::Generic => "需要填写签名密钥",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_limits_reject_blank_fractional_and_out_of_range_values() {
        for value in ["", "0", "1.5", "-1", "6", "NaN"] {
            assert!(parse_limit("尝试", value, 1, 5).is_err());
        }
        assert_eq!(parse_limit("尝试", " 3 ", 1, 5).unwrap(), 3);
        assert!(parse_limit("超时", "30001", 100, 30_000).is_err());
    }
    #[test]
    fn empty_bark_address_does_not_claim_saved_device_key() {
        let mut status = WebhookRuntimeStatus::default();
        status.config.provider = WebhookProvider::Bark;
        assert!(credential_label(&status).starts_with("需要填写"));
    }
}
