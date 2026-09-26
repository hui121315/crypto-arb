use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    WebhookApplicationAck, WebhookDeliveryStatus, WebhookEventKind, WebhookRuntimeStatus,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::panels) enum DeterministicFlowState {
    Idle,
    Current,
    Complete,
    Warning,
    Blocked,
}

impl DeterministicFlowState {
    const fn token(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Current => "current",
            Self::Complete => "complete",
            Self::Warning => "warning",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::panels) struct DeterministicFlowStage {
    pub label: &'static str,
    pub detail: String,
    pub state: DeterministicFlowState,
}

impl DeterministicFlowStage {
    pub(in crate::panels) fn new(
        label: &'static str,
        detail: impl Into<String>,
        state: DeterministicFlowState,
    ) -> Self {
        Self {
            label,
            detail: detail.into(),
            state,
        }
    }
}

pub(in crate::panels) fn deterministic_flow_rail(
    label: &'static str,
    stages: Vec<DeterministicFlowStage>,
) -> impl IntoView {
    view! {
        <section class="deterministic-flow" aria-label=label>
            <ol>
                {stages.into_iter().enumerate().map(|(index, stage)| {
                    let detail = stage.detail;
                    let title = detail.clone();
                    view! {
                        <li data-state=stage.state.token() title=title>
                            <span>{format!("{:02} · {}", index + 1, stage.label)}</span>
                            <strong>{detail}</strong>
                        </li>
                    }
                }).collect_view()}
            </ol>
        </section>
    }
}

pub(in crate::panels) fn webhook_flow_stage(
    state: &LoadState<WebhookRuntimeStatus>,
    kind: WebhookEventKind,
) -> DeterministicFlowStage {
    if matches!(state, LoadState::Stale { .. } | LoadState::Error(_)) {
        return DeterministicFlowStage::new(
            "Webhook",
            "状态待确认",
            DeterministicFlowState::Warning,
        );
    }
    let Some(status) = state.value() else {
        return DeterministicFlowStage::new(
            "Webhook",
            "读取投递状态",
            DeterministicFlowState::Current,
        );
    };
    if status.configuration_problem.is_some() {
        return DeterministicFlowStage::new("Webhook", "配置恢复失败", DeterministicFlowState::Warning);
    }
    if !status.config.enabled || !status.config.url_configured {
        return DeterministicFlowStage::new("Webhook", "未启用", DeterministicFlowState::Warning);
    }
    if !status.config.event_kinds.contains(&kind) {
        return DeterministicFlowStage::new(
            "Webhook",
            "未订阅当前事件",
            DeterministicFlowState::Warning,
        );
    }
    if status.queue_depth > 0 {
        return DeterministicFlowStage::new(
            "Webhook",
            format!("排队 {} 条", status.queue_depth),
            DeterministicFlowState::Current,
        );
    }
    let Some(delivery) = status
        .recent_deliveries
        .iter()
        .find(|delivery| delivery.kind == kind)
    else {
        return DeterministicFlowStage::new(
            "Webhook",
            "等待合格事件",
            DeterministicFlowState::Idle,
        );
    };
    let attempts = delivery.attempts.max(1);
    match (delivery.status, delivery.application_ack) {
        (WebhookDeliveryStatus::Delivered, WebhookApplicationAck::Accepted) => {
            DeterministicFlowStage::new(
                "Webhook",
                if attempts > 1 {
                    format!("重试 {attempts} 次后应用确认")
                } else {
                    "应用已确认".to_owned()
                },
                DeterministicFlowState::Complete,
            )
        }
        (WebhookDeliveryStatus::Delivered, WebhookApplicationAck::TransportOnly) => {
            DeterministicFlowStage::new("Webhook", "仅 HTTP 成功", DeterministicFlowState::Warning)
        }
        (WebhookDeliveryStatus::Queued, _) => {
            DeterministicFlowStage::new("Webhook", "等待投递", DeterministicFlowState::Current)
        }
        (WebhookDeliveryStatus::Failed, _) => DeterministicFlowStage::new(
            "Webhook",
            format!("{attempts} 次尝试后失败"),
            DeterministicFlowState::Blocked,
        ),
        (WebhookDeliveryStatus::Dropped, _) => DeterministicFlowStage::new(
            "Webhook",
            "队列已满，等待重试",
            DeterministicFlowState::Blocked,
        ),
        (WebhookDeliveryStatus::Disabled, _) => {
            DeterministicFlowStage::new("Webhook", "事件未订阅", DeterministicFlowState::Warning)
        }
        (_, WebhookApplicationAck::Rejected | WebhookApplicationAck::InvalidResponse) => {
            DeterministicFlowStage::new("Webhook", "应用确认失败", DeterministicFlowState::Blocked)
        }
        (_, WebhookApplicationAck::Unknown) => {
            DeterministicFlowStage::new("Webhook", "应用状态未知", DeterministicFlowState::Warning)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{WebhookConfig, WebhookDeliveryRecord, WebhookProvider};

    #[test]
    fn bark_application_ack_is_complete_but_transport_only_is_warning() {
        let accepted = state(WebhookApplicationAck::Accepted);
        let transport = state(WebhookApplicationAck::TransportOnly);

        assert_eq!(
            webhook_flow_stage(&accepted, WebhookEventKind::Opportunity).state,
            DeterministicFlowState::Complete
        );
        assert_eq!(
            webhook_flow_stage(&transport, WebhookEventKind::Opportunity).state,
            DeterministicFlowState::Warning
        );
    }

    fn state(ack: WebhookApplicationAck) -> LoadState<WebhookRuntimeStatus> {
        LoadState::Ready(WebhookRuntimeStatus {
            config: WebhookConfig {
                enabled: true,
                provider: WebhookProvider::Bark,
                url: "https://api.day.app/***".to_owned(),
                url_configured: true,
                ..WebhookConfig::default()
            },
            recent_deliveries: vec![WebhookDeliveryRecord {
                event_id: "opportunity-1".to_owned(),
                kind: WebhookEventKind::Opportunity,
                provider: WebhookProvider::Bark,
                status: WebhookDeliveryStatus::Delivered,
                attempts: 1,
                response_status: Some(200),
                application_ack: ack,
                response_message: None,
                error: None,
                updated_at_ms: 1,
            }],
            updated_at_ms: 1,
            ..WebhookRuntimeStatus::default()
        })
    }
}
