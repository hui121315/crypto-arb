use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::shared::webhook_delivery_diagnostic;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    onchain_cex_quote_token, OnchainSpreadAlertMode, WebhookApplicationAck, WebhookDeliveryRecord,
    WebhookDeliveryStatus, WebhookEventKind, WebhookProvider, WebhookRuntimeStatus,
};

use super::super::data::OnchainData;
use super::super::draft::OnchainConfigDraft;
use super::super::format::time_label;
use super::cex_pair_select::selected_quotes_comparable;

pub(super) fn spread_alert_control(draft: OnchainConfigDraft, data: OnchainData) -> impl IntoView {
    view! {
        <section class="onchain-spread-alert" aria-label="链上价差 Webhook">
            <div class="onchain-alert-heading">
                <div><strong>{move || alert_title(draft)}</strong><span>{move || alert_state_label(draft, data, &data.webhook_status.get())}</span></div>
                <label class="onchain-alert-toggle">
                    <input
                        type="checkbox"
                        aria-label="启用链上价差 Webhook"
                        prop:checked=move || draft.alert_enabled.get()
                        on:change=move |event| draft.alert_enabled.set(event_target_checked(&event))
                    />
                    <span class="onchain-switch" aria-hidden="true"></span>
                </label>
            </div>
            <div class="onchain-alert-mode" role="group" aria-label="提醒判断方式">
                <button
                    type="button"
                    class=move || alert_mode_class(draft, OnchainSpreadAlertMode::VerifiedNet)
                    aria-pressed=move || (draft.alert_mode.get() == OnchainSpreadAlertMode::VerifiedNet).to_string()
                    on:click=move |_| draft.alert_mode.set(OnchainSpreadAlertMode::VerifiedNet)
                >"费后机会"</button>
                <button
                    type="button"
                    class=move || alert_mode_class(draft, OnchainSpreadAlertMode::RawObservation)
                    aria-pressed=move || (draft.alert_mode.get() == OnchainSpreadAlertMode::RawObservation).to_string()
                    on:click=move |_| draft.alert_mode.set(OnchainSpreadAlertMode::RawObservation)
                >"原始观察"</button>
            </div>
            <p class=move || alert_mode_note_class(draft, data)>{move || alert_mode_note(draft, data)}</p>
            <div class="onchain-alert-fields">
                {move || match draft.alert_mode.get() {
                    OnchainSpreadAlertMode::VerifiedNet => view! {
                        <label class="workbench-field"><span>"费后净价差阈值 (%)"</span><input type="number" min="0" max="100" step="0.001" bind:value=draft.alert_threshold /></label>
                    }.into_any(),
                    OnchainSpreadAlertMode::RawObservation => view! {
                        <label class="workbench-field"><span>"原始价差阈值 (%)"</span><input type="number" min="0" max="100" step="0.001" bind:value=draft.alert_raw_threshold /></label>
                    }.into_any(),
                }}
                <label class="workbench-field"><span>"提醒冷却 (秒)"</span><input type="number" min="30" max="86400" step="30" bind:value=draft.alert_cooldown /></label>
            </div>
            <p
                class="onchain-alert-policy"
                title="路径由链、Base/Quote 合约、交易所、交易对、提醒模式和方向共同确定"
            >
                {move || alert_policy_label(draft)}
            </p>
            <details
                class="onchain-alert-runtime-disclosure"
                open=move || data.webhook_status.with(|state| state.value().is_some_and(|status| status.configuration_problem.is_some()))
                    || (draft.alert_enabled.get() && !webhook_ready(&data.webhook_status.get()))
            >
                <summary>
                    <span>"投递运行数据依据"</span>
                    <strong class=move || alert_runtime_summary_class(draft, data)>
                        {move || alert_state_label(draft, data, &data.webhook_status.get())}
                    </strong>
                </summary>
                {move || webhook_readiness(&data.webhook_status.get(), &data.webhook_transport.get())}
            </details>
        </section>
    }
}

pub(super) fn spread_alert_quick_control(
    draft: OnchainConfigDraft,
    data: OnchainData,
    open_rules: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="onchain-alert-quick" aria-label="价差提醒快捷设置">
            <button
                type="button"
                class="onchain-alert-quick-open"
                on:click=move |_| open_rules.run(())
            >
                <span class=move || alert_quick_state_class(draft, data) aria-hidden="true"></span>
                <span>
                    <strong>{move || alert_title(draft)}</strong>
                    <small>{move || alert_state_label(draft, data, &data.webhook_status.get())}</small>
                </span>
                <em>"规则"</em>
            </button>
            <label class="onchain-alert-toggle">
                <input
                    type="checkbox"
                    aria-label="启用链上价差 Webhook"
                    prop:checked=move || draft.alert_enabled.get()
                    on:change=move |event| draft.alert_enabled.set(event_target_checked(&event))
                />
                <span class="onchain-switch" aria-hidden="true"></span>
            </label>
        </div>
    }
}

fn alert_quick_state_class(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    match alert_state_label(draft, data, &data.webhook_status.get()) {
        "已就绪" => "onchain-alert-quick-state is-positive",
        "已关闭" => "onchain-alert-quick-state is-neutral",
        _ => "onchain-alert-quick-state is-warning",
    }
}

fn alert_runtime_summary_class(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    match alert_state_label(draft, data, &data.webhook_status.get()) {
        "已就绪" => "is-positive",
        "已关闭" => "is-neutral",
        "当前组合不适用" => "is-danger",
        _ => "is-warning",
    }
}

fn alert_policy_label(draft: OnchainConfigDraft) -> String {
    let cooldown = draft.alert_cooldown.get();
    let cooldown = cooldown.trim();
    let cooldown = if cooldown.is_empty() { "30" } else { cooldown };
    format!("进入门槛时触发；同一路径同方向 {cooldown} 秒内重复事件会去重。")
}

fn alert_state_label(
    draft: OnchainConfigDraft,
    data: OnchainData,
    status: &LoadState<WebhookRuntimeStatus>,
) -> &'static str {
    if status.value().is_some_and(|status| status.configuration_problem.is_some()) {
        return "配置恢复失败";
    }
    let applied = data.state.with(|state| {
        state
            .value()
            .is_some_and(|snapshot| draft.alert_matches_applied_config(&snapshot.config))
    });
    alert_state_label_for(
        applied,
        draft.alert_enabled.get(),
        selected_quotes_comparable(draft, &data.state.get()),
        webhook_ready(status),
    )
}

const fn alert_state_label_for(
    applied: bool,
    enabled: bool,
    quotes_match: bool,
    ready: bool,
) -> &'static str {
    if !applied {
        "待应用"
    } else if !enabled {
        "已关闭"
    } else if !quotes_match {
        "当前组合不适用"
    } else if ready {
        "已就绪"
    } else {
        "等待配置"
    }
}

fn alert_title(draft: OnchainConfigDraft) -> &'static str {
    match draft.alert_mode.get() {
        OnchainSpreadAlertMode::VerifiedNet => "费后机会提醒",
        OnchainSpreadAlertMode::RawObservation => "原始价差观察",
    }
}

fn alert_mode_class(draft: OnchainConfigDraft, mode: OnchainSpreadAlertMode) -> &'static str {
    if draft.alert_mode.get() == mode {
        "is-active"
    } else {
        ""
    }
}

fn alert_mode_note_class(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    if draft.alert_mode.get() == OnchainSpreadAlertMode::VerifiedNet
        && !selected_quotes_comparable(draft, &data.state.get())
    {
        "onchain-alert-mode-note is-warning"
    } else {
        "onchain-alert-mode-note"
    }
}

fn alert_mode_note(draft: OnchainConfigDraft, data: OnchainData) -> String {
    match draft.alert_mode.get() {
        OnchainSpreadAlertMode::VerifiedNet
            if !selected_quotes_comparable(draft, &data.state.get()) =>
        {
            "当前 交易所 与链上 Quote 不同，且尚无匹配的新鲜 WS 汇率；只能使用原始观察。".to_owned()
        }
        OnchainSpreadAlertMode::VerifiedNet if selected_quotes_match(draft) => {
            "同一 Quote；只在双源新鲜且达到最低可执行净差时推送。".to_owned()
        }
        OnchainSpreadAlertMode::VerifiedNet => {
            "已接入当前交易所的实时 WS 汇率；换算 Quote 后按费后净差推送。".to_owned()
        }
        OnchainSpreadAlertMode::RawObservation => {
            "只提醒名义价格差；不扣费用、不代表利润，也不会进入执行。".to_owned()
        }
    }
}

fn selected_quotes_match(draft: OnchainConfigDraft) -> bool {
    let symbol = draft.symbol.get();
    onchain_cex_quote_token(&symbol)
        .is_some_and(|quote| quote.eq_ignore_ascii_case(draft.quote_token.get().trim()))
}

fn webhook_readiness(
    state: &LoadState<WebhookRuntimeStatus>,
    transport: &WsChannelState,
) -> AnyView {
    match state {
        LoadState::Loading => {
            runtime_message("读取 Webhook 状态", "等待后端凭证和投递队列数据依据", "")
        }
        LoadState::Error(problem) => runtime_message(
            "Webhook 状态不可用",
            &format!("{} · {}", problem.code, problem.message),
            "is-danger",
        ),
        LoadState::Ready(status) => webhook_runtime(status, transport, None),
        LoadState::Stale {
            value: status,
            problem,
        } => webhook_runtime(
            status,
            transport,
            Some(format!("{} · {}", problem.code, problem.message)),
        ),
    }
}

fn webhook_runtime(
    status: &WebhookRuntimeStatus,
    transport: &WsChannelState,
    stale_problem: Option<String>,
) -> AnyView {
    if let Some(problem) = &status.configuration_problem {
        return runtime_message("Webhook 配置恢复失败", &problem.message, "is-danger");
    }
    let missing = missing_webhook_fields(status);
    if !missing.is_empty() {
        return view! {
            <div class="onchain-alert-runtime is-warning">
                <div class="onchain-alert-runtime-head">
                    <div><strong>"Webhook 还需配置"</strong><span>{missing.join("、")}</span></div>
                    <a href="#settings">"前往设置"</a>
                </div>
                {stale_problem.map(|problem| view! { <p class="onchain-alert-runtime-problem">{format!("状态读取降级 · {problem}")}</p> })}
            </div>
        }
        .into_any();
    }

    let latest = latest_alert_delivery(status);
    let (delivery_label, delivery_tone) = latest
        .map(delivery_state)
        .unwrap_or(("尚无链上价差投递", "is-neutral"));
    let tone = if stale_problem.is_some() {
        "is-warning"
    } else if matches!(delivery_tone, "is-danger") {
        "is-danger"
    } else if status.queue_depth > 0 || matches!(delivery_tone, "is-warning") {
        "is-warning"
    } else {
        "is-positive"
    };
    let delivery_detail = latest.and_then(delivery_detail);
    let delivery_problem = latest.and_then(delivery_attention_reason);
    let delivery_meta = latest.map_or_else(
        || provider_label(status.config.provider).to_owned(),
        |delivery| {
            format!(
                "{} · {} · 更新 {}",
                provider_label(delivery.provider),
                delivery_kind_label(delivery.kind),
                time_label(delivery.updated_at_ms)
            )
        },
    );
    let transport_label = webhook_transport_label(transport);
    let transport_title = webhook_transport_title(transport);
    view! {
        <div class=format!("onchain-alert-runtime {tone}")>
            <div class="onchain-alert-runtime-head">
                <div><small>"最近验证"</small><strong>{delivery_label}</strong><span>{delivery_meta}</span></div>
                <span class="onchain-alert-transport" title=transport_title>{transport_label}</span>
            </div>
            {delivery_problem.map(|problem| view! {
                <p class="onchain-alert-current-problem">{problem}</p>
            })}
            <div class="onchain-alert-runtime-foot">
                <span class="num">{format!("队列 {}/{}", status.queue_depth, status.config.queue_capacity)}</span>
                <span class="num">{format!("历史 {} 成功 · {} 失败 · {} 丢弃", status.delivered_total, status.failed_total, status.dropped_total)}</span>
            </div>
            {stale_problem.map(|problem| view! { <p class="onchain-alert-runtime-problem">{format!("显示上次状态 · {problem}")}</p> })}
            {delivery_detail.map(|detail| view! {
                <details class="onchain-alert-delivery-detail">
                    <summary>"完整投递数据依据"</summary>
                    <p>{detail}</p>
                </details>
            })}
        </div>
    }
    .into_any()
}

fn webhook_transport_label(transport: &WsChannelState) -> &'static str {
    match (transport.status, transport.subscribed) {
        (WsStatus::Connected, true) => "WS 实时",
        (WsStatus::Connected, false) | (WsStatus::Connecting, _) => "WS 连接中",
        (WsStatus::Disconnected, _) => "WS 重连中",
    }
}

fn webhook_transport_title(transport: &WsChannelState) -> String {
    transport.last_error.as_ref().map_or_else(
        || "Webhook 状态通过共享 AppWS 即时更新".to_owned(),
        |problem| format!("{} · {}", problem.code, problem.message),
    )
}

fn runtime_message(title: &str, detail: &str, tone: &str) -> AnyView {
    view! {
        <div class=format!("onchain-alert-runtime {tone}")>
            <div class="onchain-alert-runtime-head"><div><strong>{title.to_owned()}</strong><span>{detail.to_owned()}</span></div></div>
        </div>
    }
    .into_any()
}

fn delivery_state(delivery: &WebhookDeliveryRecord) -> (&'static str, &'static str) {
    match (delivery.status, delivery.application_ack) {
        (WebhookDeliveryStatus::Delivered, WebhookApplicationAck::Accepted) => {
            ("应用已确认", "is-positive")
        }
        (WebhookDeliveryStatus::Delivered, WebhookApplicationAck::TransportOnly) => {
            ("仅传输成功", "is-warning")
        }
        (WebhookDeliveryStatus::Failed, _) => ("投递失败", "is-danger"),
        (WebhookDeliveryStatus::Dropped, _) => ("队列丢弃", "is-danger"),
        (WebhookDeliveryStatus::Queued, _) => ("等待投递", "is-warning"),
        (WebhookDeliveryStatus::Disabled, _) => ("投递已停用", "is-neutral"),
        (_, WebhookApplicationAck::Rejected) => ("应用拒绝", "is-danger"),
        (_, WebhookApplicationAck::InvalidResponse) => ("确认格式无效", "is-danger"),
        (_, WebhookApplicationAck::Unknown) => ("应用状态未知", "is-warning"),
    }
}

fn latest_alert_delivery(status: &WebhookRuntimeStatus) -> Option<&WebhookDeliveryRecord> {
    status.recent_deliveries.iter().find(|delivery| {
        matches!(
            delivery.kind,
            WebhookEventKind::OnchainSpread | WebhookEventKind::Test
        )
    })
}

const fn delivery_kind_label(kind: WebhookEventKind) -> &'static str {
    match kind {
        WebhookEventKind::OnchainSpread => "链上提醒",
        WebhookEventKind::Test => "连通测试",
        _ => "通道事件",
    }
}

fn delivery_detail(delivery: &WebhookDeliveryRecord) -> Option<String> {
    let mut parts = Vec::new();
    push_unique_detail(&mut parts, delivery.error.as_deref());
    push_unique_detail(&mut parts, delivery.response_message.as_deref());
    if let Some(status) = delivery.response_status {
        parts.push(format!("HTTP {status}"));
    }
    if delivery.attempts > 0 {
        parts.push(format!("尝试 {} 次", delivery.attempts));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn push_unique_detail(parts: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let value = webhook_delivery_diagnostic(value);
    if !parts.iter().any(|part| part == &value) {
        parts.push(value);
    }
}

fn delivery_attention_reason(delivery: &WebhookDeliveryRecord) -> Option<String> {
    let needs_attention = matches!(
        delivery.status,
        WebhookDeliveryStatus::Failed | WebhookDeliveryStatus::Dropped
    ) || matches!(
        delivery.application_ack,
        WebhookApplicationAck::TransportOnly
            | WebhookApplicationAck::Rejected
            | WebhookApplicationAck::InvalidResponse
            | WebhookApplicationAck::Unknown
    );
    needs_attention.then(|| {
        let fallback = match (delivery.status, delivery.application_ack) {
            (WebhookDeliveryStatus::Dropped, _) => "投递队列已满或事件被丢弃。",
            (_, WebhookApplicationAck::TransportOnly) => {
                "HTTP 已送达，但目标应用没有返回可确认的成功结果。"
            }
            (_, WebhookApplicationAck::Rejected) => "目标应用明确拒绝了这次投递。",
            (_, WebhookApplicationAck::InvalidResponse) => "目标应用返回内容无法确认成功。",
            _ => "后端尚未证明这次投递已被目标应用接收。",
        };
        compact_reason(
            delivery_detail(delivery).as_deref().unwrap_or(fallback),
            132,
        )
    })
}

fn compact_reason(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let compact = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}…")
    } else {
        compact
    }
}

const fn provider_label(provider: WebhookProvider) -> &'static str {
    match provider {
        WebhookProvider::Generic => "通用签名 Webhook",
        WebhookProvider::Bark => "Bark 推送",
    }
}

pub(super) fn webhook_ready(state: &LoadState<WebhookRuntimeStatus>) -> bool {
    state
        .value()
        .is_some_and(|status| status.configuration_problem.is_none() && missing_webhook_fields(status).is_empty())
}

fn missing_webhook_fields(status: &WebhookRuntimeStatus) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !status.config.enabled {
        missing.push("启用开关");
    }
    if !status.config.url_configured {
        missing.push("公网 HTTPS URL");
    }
    if status.config.provider == shared_types::WebhookProvider::Generic
        && !status.config.secret_configured
    {
        missing.push("签名密钥");
    }
    if !status
        .config
        .event_kinds
        .contains(&WebhookEventKind::OnchainSpread)
    {
        missing.push("链上价差事件");
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unapplied_alert_rule_never_claims_to_be_ready() {
        assert_eq!(alert_state_label_for(false, true, true, true), "待应用");
        assert_eq!(alert_state_label_for(true, true, true, true), "已就绪");
        assert_eq!(alert_state_label_for(true, false, true, true), "已关闭");
    }

    #[test]
    fn subscribed_webhook_channel_is_reported_as_realtime() {
        let mut transport = WsChannelState::new("webhook");
        transport.status = WsStatus::Connected;
        transport.subscribed = true;

        assert_eq!(webhook_transport_label(&transport), "WS 实时");
    }

    #[test]
    fn failed_delivery_exposes_the_actual_reason_without_dominating_the_card() {
        let delivery = WebhookDeliveryRecord {
            event_id: "event-1".to_owned(),
            kind: WebhookEventKind::OnchainSpread,
            provider: WebhookProvider::Bark,
            status: WebhookDeliveryStatus::Failed,
            attempts: 3,
            response_status: Some(502),
            application_ack: WebhookApplicationAck::Unknown,
            response_message: None,
            error: Some("upstream connection failed".to_owned()),
            updated_at_ms: 1,
        };

        assert_eq!(
            delivery_attention_reason(&delivery).as_deref(),
            Some("upstream connection failed · HTTP 502 · 尝试 3 次")
        );
    }

    #[test]
    fn newer_connectivity_test_supersedes_an_old_onchain_failure() {
        let mut status = WebhookRuntimeStatus::default();
        status.recent_deliveries = vec![
            delivery_record(WebhookEventKind::Test, WebhookDeliveryStatus::Delivered, 2),
            delivery_record(
                WebhookEventKind::OnchainSpread,
                WebhookDeliveryStatus::Failed,
                1,
            ),
        ];

        let latest = latest_alert_delivery(&status);
        assert_eq!(latest.map(|row| row.kind), Some(WebhookEventKind::Test));
        assert_eq!(latest.map(|row| row.updated_at_ms), Some(2));
    }

    fn delivery_record(
        kind: WebhookEventKind,
        status: WebhookDeliveryStatus,
        updated_at_ms: i64,
    ) -> WebhookDeliveryRecord {
        WebhookDeliveryRecord {
            event_id: format!("event-{updated_at_ms}"),
            kind,
            provider: WebhookProvider::Bark,
            status,
            attempts: 1,
            response_status: Some(200),
            application_ack: WebhookApplicationAck::Accepted,
            response_message: Some("success".to_owned()),
            error: None,
            updated_at_ms,
        }
    }
}
