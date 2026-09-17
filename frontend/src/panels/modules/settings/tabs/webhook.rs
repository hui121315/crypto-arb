use crate::state::load_state::LoadState;
use leptos::prelude::*;
use crate::panels::shared::webhook_delivery_message;
use shared_types::{
    WebhookApplicationAck, WebhookConfigPatch, WebhookEventKind, WebhookProvider,
    WebhookRuntimeStatus, WebhookTestRequest,
};

use super::super::data::{use_webhook_data, WebhookData};

pub(in crate::panels::modules::settings) fn webhook_tab() -> impl IntoView {
    let data = use_webhook_data();
    let url = RwSignal::new(String::new());
    let provider = RwSignal::new("generic".to_owned());
    let secret = RwSignal::new(String::new());
    let timeout = RwSignal::new("15000".to_owned());
    let attempts = RwSignal::new("3".to_owned());
    let backoff = RwSignal::new("500".to_owned());
    let capacity = RwSignal::new("128".to_owned());
    let event_kinds = RwSignal::new(Vec::<WebhookEventKind>::new());
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
    let hydrated = RwSignal::new(false);
    Effect::new(move |_| {
        if hydrated.get() {
            return;
        }
        let Some(status) = data.state.get().value().cloned() else {
            return;
        };
        provider.set(
            match status.config.provider {
                WebhookProvider::Generic => "generic",
                WebhookProvider::Bark => "bark",
            }
            .to_owned(),
        );
        timeout.set(status.config.timeout_ms.to_string());
        attempts.set(status.config.max_attempts.to_string());
        backoff.set(status.config.base_backoff_ms.to_string());
        capacity.set(status.config.queue_capacity.to_string());
        event_kinds.set(status.config.event_kinds);
        hydrated.set(true);
    });
    view! {
        <div class="settings-panel webhook-settings">
            <div class="settings-section-heading">
                <div><h3>"Webhook"</h3><p>"通用签名或 Bark 应用确认；仅允许公网 HTTPS，失败使用有限重试。"</p></div>
                <button class="btn-secondary" on:click=move |_| toggle(data)>{move || toggle_label(&data.state.get())}</button>
            </div>
            {move || status_view(&data.state.get())}
            <div class="webhook-core-grid">
                <label class="field-inline"><span>"投递提供方"</span><select bind:value=provider><option value="generic">"通用签名 Webhook"</option><option value="bark">"Bark 推送"</option></select></label>
                <label class="field-inline field-wide"><span>"公网 HTTPS URL"</span><input type="url" placeholder="留空保留当前地址" bind:value=url /></label>
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
                    <div><strong>"高级投递"</strong><span>"重试、队列与事件范围"</span></div>
                    <em>{move || format!("{}ms · {} 次 · 队列 {}", timeout.get(), attempts.get(), capacity.get())}</em>
                    <span class="webhook-details-action">"展开"</span>
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
            <div class="webhook-settings-actions">
                <button class="workbench-save" on:click=move |_| save_config(data, draft)>"保存配置"</button>
                <button class="btn-secondary" on:click=move |_| data.test.run(WebhookTestRequest { message: Some("CROSSLINE UI test".to_owned()) })>"发送测试"</button>
            </div>
            <details class="webhook-danger-zone">
                <summary>"停用与密钥管理"</summary>
                <div>
                    <p>"该操作会停止投递并清除已保存的签名密钥或 Bark device key。"</p>
                    <button class="btn-danger" on:click=move |_| data.update.run(WebhookConfigPatch { enabled: Some(false), clear_secret: Some(true), ..WebhookConfigPatch::default() })>"停用并清除密钥"</button>
                </div>
            </details>
            {move || data.action_problem.get().map(|problem| view! { <p class="state-note is-error">{problem.message}</p> })}
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
    let target_url = draft.url.get_untracked();
    let target_url = target_url.trim();
    data.update.run(WebhookConfigPatch {
        provider: Some(if draft.provider.get_untracked() == "bark" {
            WebhookProvider::Bark
        } else {
            WebhookProvider::Generic
        }),
        url: (!target_url.is_empty()).then(|| target_url.to_owned()),
        secret: (!draft.secret.get_untracked().trim().is_empty())
            .then(|| draft.secret.get_untracked()),
        event_kinds: Some(draft.event_kinds.get_untracked()),
        timeout_ms: draft.timeout.get_untracked().parse().ok(),
        max_attempts: draft.attempts.get_untracked().parse().ok(),
        base_backoff_ms: draft.backoff.get_untracked().parse().ok(),
        queue_capacity: draft.capacity.get_untracked().parse().ok(),
        ..WebhookConfigPatch::default()
    });
    draft.url.set(String::new());
    draft.secret.set(String::new());
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

fn status_view(state: &LoadState<WebhookRuntimeStatus>) -> AnyView {
    let Some(status) = state.value() else {
        return view! { <p class="state-note">"读取 Webhook 状态…"</p> }.into_any();
    };
    let recent = status
        .recent_deliveries
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    let delivery_summary = recent.first().map_or_else(
        || "尚无投递记录".to_owned(),
        |row| {
            format!(
                "最近投递 · {}",
                ack_label(row.application_ack, row.response_status)
            )
        },
    );
    view! {
        <div class="webhook-runtime">
            <div class="webhook-summary">
                <strong>{if status.config.enabled { "运行中" } else { "已停用" }}</strong>
                <span>{provider_label(status.config.provider)}</span>
                <span>{if status.config.url_configured { status.config.url.clone() } else { "需要填写公网 HTTPS URL".to_owned() }}</span>
                <span>{credential_label(status)}</span>
                <span>{format!("队列 {}/{}", status.queue_depth, status.config.queue_capacity)}</span>
                <span>{format!("成功 {} · 失败 {} · 丢弃 {}", status.delivered_total, status.failed_total, status.dropped_total)}</span>
            </div>
            <details class="webhook-delivery-history">
                <summary>
                    <strong>{delivery_summary}</strong>
                    <span>{format!("{} 条", recent.len())}</span>
                </summary>
                <div class="webhook-deliveries">
                    {recent.into_iter().map(|row| view! {
                        <div class="webhook-delivery-row"><strong>{format!("{:?}", row.kind)}</strong><span>{ack_label(row.application_ack, row.response_status)}</span><span>{format!("{} 次", row.attempts)}</span><span>{webhook_delivery_message(row.error.as_deref(), row.response_message.as_deref()).unwrap_or_else(|| "投递完成".to_owned())}</span></div>
                    }).collect_view()}
                </div>
            </details>
        </div>
    }.into_any()
}

fn provider_label(provider: WebhookProvider) -> &'static str {
    match provider {
        WebhookProvider::Generic => "通用签名 Webhook",
        WebhookProvider::Bark => "Bark 推送",
    }
}

fn credential_label(status: &WebhookRuntimeStatus) -> &'static str {
    match status.config.provider {
        WebhookProvider::Bark => "Bark device key 已隐藏",
        WebhookProvider::Generic if status.config.secret_configured => "签名密钥已保存（不回显）",
        WebhookProvider::Generic => "需要填写签名密钥",
    }
}

fn ack_label(ack: WebhookApplicationAck, status: Option<u16>) -> String {
    let http = status.map_or_else(|| "HTTP 未知".to_owned(), |status| format!("HTTP {status}"));
    let application = match ack {
        WebhookApplicationAck::Accepted => "应用已确认",
        WebhookApplicationAck::TransportOnly => "仅传输成功",
        WebhookApplicationAck::Rejected => "应用拒绝",
        WebhookApplicationAck::InvalidResponse => "确认格式无效",
        WebhookApplicationAck::Unknown => "应用状态未知",
    };
    format!("{http} · {application}")
}
