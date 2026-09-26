use super::webhook_delivery_message;
use super::webhook_test::webhook_test_feedback;
pub(crate) use super::webhook_test::WebhookTestRuntime as WebhookTestFeedback;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    WebhookApplicationAck, WebhookDeliveryStatus, WebhookEventKind, WebhookProvider,
    WebhookRuntimeStatus, WebhookTestRequest,
};

pub(crate) fn webhook_monitor(
    title: &'static str,
    required_event: WebhookEventKind,
    state: RwSignal<LoadState<WebhookRuntimeStatus>>,
    action_problem: RwSignal<Option<shared_types::ApiProblem>>,
    test: Callback<WebhookTestRequest>,
    feedback: Option<WebhookTestFeedback>,
) -> impl IntoView {
    if let Some(runtime) = feedback { runtime.watch(state); }
    view! {
        <section class="webhook-monitor" aria-live="polite">
            <div class="webhook-monitor-title">
                <span class=move || monitor_dot_class(&state.get(), required_event)></span>
                <div><strong>{title}</strong><small>{move || monitor_state_label(&state.get(), required_event)}</small></div>
            </div>
            {move || monitor_summary(&state.get())}
            <button
                class="btn-secondary webhook-monitor-test"
                type="button"
                disabled=move || feedback.is_some_and(|feedback| feedback.pending.get())
                    || !matches!(state.get(), LoadState::Ready(status) if status.config.url_configured && status.configuration_problem.is_none())
                title=move || if feedback.is_some_and(|runtime| runtime.config.locked()) {
                    "Webhook 配置待核对，请前往设置查看原操作".to_owned()
                } else { action_problem.get().map_or_else(|| "设备 Key 与 URL 始终隐藏".to_owned(), |problem| problem.message) }
                on:click=move |_| test.run(WebhookTestRequest { message: Some("CROSSLINE deterministic workflow test".to_owned()) })
            >{move || if feedback.is_some_and(|feedback| feedback.journal.busy.get()) { "提交中" } else { "测试投递" }}</button>
            {feedback.map(webhook_test_feedback)}
            <details class="webhook-monitor-history">
                <summary>{move || format!("最近投递 · {} 条", state.get().value().map_or(0, |status| status.recent_deliveries.len().min(3)))}</summary>
                {move || delivery_trace(&state.get())}
            </details>
            {move || action_problem.get().filter(|_| feedback.is_none()).map(|problem| {
                let detail = format!("测试投递失败 · {} · {}", problem.code, problem.message);
                view! { <p class="webhook-monitor-problem" title=problem.message>{detail}</p> }
            })}
        </section>
    }
}

pub(crate) fn webhook_monitor_disclosure(
    title: &'static str,
    required_event: WebhookEventKind,
    state: RwSignal<LoadState<WebhookRuntimeStatus>>,
    action_problem: RwSignal<Option<shared_types::ApiProblem>>,
    test: Callback<WebhookTestRequest>,
    feedback: Option<WebhookTestFeedback>,
) -> impl IntoView {
    let monitor = webhook_monitor(title, required_event, state, action_problem, test, feedback);
    view! {
        <details class="webhook-monitor-disclosure">
            <summary>
                <span class=move || monitor_dot_class(&state.get(), required_event)></span>
                <strong>{title}</strong>
                <em>{move || monitor_state_label(&state.get(), required_event)}</em>
                <span class="webhook-monitor-disclosure-action">"投递详情"</span>
            </summary>
            <div class="webhook-monitor-disclosure-body">{monitor}</div>
        </details>
    }
}

fn monitor_summary(state: &LoadState<WebhookRuntimeStatus>) -> AnyView {
    let Some(status) = state.value() else {
        let message = if matches!(state, LoadState::Loading) {
            "正在读取后端 Webhook 状态…"
        } else {
            "读取失败，暂无可用的投递快照"
        };
        return view! { <div class="webhook-monitor-loading">{message}</div> }
            .into_any();
    };
    if let Some(problem) = &status.configuration_problem {
        return view! { <p class="webhook-monitor-problem" role="alert">{problem.message.clone()}</p> }.into_any();
    }
    let last = status.recent_deliveries.first().cloned();
    let last_label = last.as_ref().map_or_else(
        || "尚无投递".to_owned(),
        |row| delivery_label(row.status, row.application_ack, row.response_status),
    );
    let last_tone = last.as_ref().map_or("is-neutral", |row| match row.status {
        WebhookDeliveryStatus::Delivered
            if row.application_ack == WebhookApplicationAck::Accepted =>
        {
            "is-positive"
        }
        WebhookDeliveryStatus::Failed | WebhookDeliveryStatus::Dropped => "is-negative",
        WebhookDeliveryStatus::Queued
        | WebhookDeliveryStatus::Disabled
        | WebhookDeliveryStatus::Delivered => "is-warning",
    });
    let endpoint = if status.config.url_configured {
        status.config.url.clone()
    } else {
        "未配置".to_owned()
    };
    let summary = format!(
        "{} · Queue {}/{} · {} · {} 成功 / {} 失败 / {} 丢弃",
        endpoint,
        status.queue_depth,
        status.config.queue_capacity,
        last_label,
        status.delivered_total,
        status.failed_total,
        status.dropped_total,
    );
    view! {
        <div class="webhook-monitor-summary">
            <strong>{provider_label(status.config.provider)}</strong>
            <span class=last_tone>{summary}</span>
        </div>
    }
    .into_any()
}

fn monitor_state_label(
    state: &LoadState<WebhookRuntimeStatus>,
    required_event: WebhookEventKind,
) -> String {
    match state {
        LoadState::Loading => "连接中".to_owned(),
        LoadState::Error(problem) => format!("读取失败 · {}", problem.code),
        LoadState::Stale { problem, .. } => format!("状态待确认 · {} · 保留上次处理结果", problem.code),
        LoadState::Ready(status) => {
            if status.configuration_problem.is_some() {
                "配置恢复失败 · 投递暂停".to_owned()
            } else if !status.config.url_configured {
                "尚未配置投递地址".to_owned()
            } else if !status.config.enabled {
                "配置已保存，当前停用".to_owned()
            } else if !status.config.event_kinds.contains(&required_event) {
                format!("已启用，未订阅{}事件", event_label(required_event))
            } else if status.queue_depth > 0 {
                format!("实时队列 · {} 条等待投递", status.queue_depth)
            } else if status.recent_deliveries.first().is_some_and(|delivery| {
                matches!(
                    delivery.status,
                    WebhookDeliveryStatus::Failed | WebhookDeliveryStatus::Dropped
                )
            }) {
                "投递降级 · 保留失败数据依据".to_owned()
            } else {
                format!("{}投递通道已就绪", event_label(required_event))
            }
        }
    }
}

fn monitor_dot_class(
    state: &LoadState<WebhookRuntimeStatus>,
    required_event: WebhookEventKind,
) -> &'static str {
    if matches!(state, LoadState::Stale { .. } | LoadState::Error(_)) {
        return "webhook-monitor-dot is-warning";
    }
    match state.value() {
        Some(status) if status.configuration_problem.is_some() => "webhook-monitor-dot is-blocked",
        Some(status)
            if status.recent_deliveries.first().is_some_and(|delivery| {
                matches!(
                    delivery.status,
                    WebhookDeliveryStatus::Failed | WebhookDeliveryStatus::Dropped
                )
            }) =>
        {
            "webhook-monitor-dot is-blocked"
        }
        Some(status) if status.queue_depth > 0 => "webhook-monitor-dot is-warning",
        Some(status)
            if status.config.enabled
                && status.config.url_configured
                && status.config.event_kinds.contains(&required_event) =>
        {
            "webhook-monitor-dot is-ready"
        }
        Some(_) => "webhook-monitor-dot is-warning",
        None => "webhook-monitor-dot is-blocked",
    }
}

fn delivery_trace(state: &LoadState<WebhookRuntimeStatus>) -> AnyView {
    let Some(status) = state.value() else {
        return ().into_any();
    };
    if status.recent_deliveries.is_empty() {
        return view! {
            <div class="webhook-monitor-deliveries is-empty">"尚无自动投递记录"</div>
        }
        .into_any();
    }
    view! {
            <div class="webhook-monitor-deliveries" aria-label="最近 Webhook 投递">
                {status.recent_deliveries.iter().take(3).cloned().map(|delivery| {
                    let tone = delivery_tone(delivery.status, delivery.application_ack);
                    let detail = delivery_label(
                        delivery.status,
                        delivery.application_ack,
                        delivery.response_status,
                    );
                    view! {
                        <span data-tone=tone title=webhook_delivery_message(delivery.error.as_deref(), delivery.response_message.as_deref()).unwrap_or_else(|| detail.clone())>
                            <b>{event_label(delivery.kind)}</b>
                            <em>{format!("{} · 尝试 {}", detail, delivery.attempts.max(1))}</em>
                        </span>
                    }
                }).collect_view()}
            </div>
    }
    .into_any()
}

const fn delivery_tone(status: WebhookDeliveryStatus, ack: WebhookApplicationAck) -> &'static str {
    match (status, ack) {
        (WebhookDeliveryStatus::Delivered, WebhookApplicationAck::Accepted) => "positive",
        (WebhookDeliveryStatus::Failed | WebhookDeliveryStatus::Dropped, _) => "negative",
        _ => "warning",
    }
}

const fn event_label(kind: WebhookEventKind) -> &'static str {
    match kind {
        WebhookEventKind::Opportunity => "确定性机会",
        WebhookEventKind::OpportunityMonitor => "价差/充提监控",
        WebhookEventKind::AutomationDecision => "自动化决策",
        WebhookEventKind::ExecutionResult => "执行最终结果",
        WebhookEventKind::Compensation => "补偿最终结果",
        WebhookEventKind::RiskAlert => "风险告警",
        WebhookEventKind::SystemDegradation => "系统降级",
        WebhookEventKind::OnchainSpread => "链上价差",
        WebhookEventKind::StockSpread => "股票价差观察",
        WebhookEventKind::Test => "测试",
    }
}

fn provider_label(provider: WebhookProvider) -> &'static str {
    match provider {
        WebhookProvider::Generic => "Generic signed",
        WebhookProvider::Bark => "Bark",
    }
}

pub(crate) fn delivery_label(
    status: WebhookDeliveryStatus,
    ack: WebhookApplicationAck,
    response_status: Option<u16>,
) -> String {
    let http = response_status.map_or_else(|| "HTTP —".to_owned(), |value| format!("HTTP {value}"));
    let result = match (status, ack) {
        (WebhookDeliveryStatus::Delivered, WebhookApplicationAck::Accepted) => "应用已确认",
        (WebhookDeliveryStatus::Delivered, WebhookApplicationAck::TransportOnly) => "仅传输成功",
        (WebhookDeliveryStatus::Failed, _) => "投递失败",
        (WebhookDeliveryStatus::Dropped, _) => "队列丢弃",
        (WebhookDeliveryStatus::Queued, _) => "等待投递",
        (WebhookDeliveryStatus::Disabled, _) => "已停用",
        (_, WebhookApplicationAck::Rejected) => "应用拒绝",
        (_, WebhookApplicationAck::InvalidResponse) => "确认格式无效",
        (_, WebhookApplicationAck::Unknown) => "应用状态未知",
    };
    format!("{http} · {result}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_webhook_status_never_claims_current_readiness() {
        let mut status = WebhookRuntimeStatus::default();
        status.config.enabled = true;
        status.config.url_configured = true;
        let state = LoadState::Stale {
            value: status,
            problem: shared_types::ApiProblem::new("TIMEOUT", "timed out"),
        };
        assert_eq!(
            monitor_state_label(&state, WebhookEventKind::Opportunity),
            "状态待确认 · TIMEOUT · 保留上次处理结果"
        );
        assert_eq!(
            monitor_dot_class(&state, WebhookEventKind::Opportunity),
            "webhook-monitor-dot is-warning"
        );
    }

    #[test]
    fn configured_disabled_webhook_is_not_reported_as_unconfigured() {
        let mut status = WebhookRuntimeStatus::default();
        status.config.url_configured = true;

        assert_eq!(
            monitor_state_label(
                &LoadState::Ready(status),
                WebhookEventKind::AutomationDecision
            ),
            "配置已保存，当前停用"
        );
    }

    #[test]
    fn enabled_webhook_reports_a_missing_required_event_scope() {
        let mut status = WebhookRuntimeStatus::default();
        status.config.url_configured = true;
        status.config.enabled = true;
        status.config.event_kinds = vec![WebhookEventKind::Opportunity];

        assert_eq!(
            monitor_state_label(
                &LoadState::Ready(status),
                WebhookEventKind::AutomationDecision
            ),
            "已启用，未订阅自动化决策事件"
        );
    }

    #[test]
    fn enabled_webhook_reports_channel_readiness_without_claiming_runtime_activity() {
        let mut status = WebhookRuntimeStatus::default();
        status.config.url_configured = true;
        status.config.enabled = true;
        status.config.event_kinds = vec![WebhookEventKind::AutomationDecision];

        assert_eq!(
            monitor_state_label(
                &LoadState::Ready(status),
                WebhookEventKind::AutomationDecision
            ),
            "自动化决策投递通道已就绪"
        );
    }
}
