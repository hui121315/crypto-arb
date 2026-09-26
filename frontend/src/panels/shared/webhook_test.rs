use super::operation_journal::{validate_setting_response, OperationJournal};
use crate::api::rest::with_mutation_timeout;
use crate::state::load_state::LoadState;
use leptos::{prelude::*, task::spawn_local};
use shared_types::{
    ActionRun, ActionRunKind, ActionRunStatus, ApiProblem, WebhookApplicationAck,
    WebhookDeliveryStatus, WebhookEventKind, WebhookRuntimeStatus, WebhookTestRequest,
    WebhookTestResponse,
};

#[derive(Clone, Copy)]
pub(crate) struct WebhookTestRuntime {
    pub config: OperationJournal,
    pub journal: OperationJournal,
    pub pending: Memo<bool>,
    pub message: RwSignal<Option<String>>,
    pub problem: RwSignal<Option<ApiProblem>>,
    pub receipt: RwSignal<Option<WebhookTestResponse>>,
    pub send: Callback<WebhookTestRequest>,
    pub recheck: Callback<()>,
}

pub(crate) fn provide_webhook_test() {
    let config = OperationJournal::new("webhook");
    let journal = OperationJournal::new("webhook-test");
    let message = RwSignal::new(None);
    let problem = RwSignal::new(None);
    let receipt = RwSignal::new(None::<WebhookTestResponse>);
    let pending = Memo::new(move |_| journal.locked() || config.locked());
    let observe = Callback::new(move |status: WebhookRuntimeStatus| {
        accept_delivery(journal, receipt, message, problem, &status);
    });
    let refresh = Callback::new(move |()| {
        let client = journal.client();
        let epoch = journal.epoch.get_untracked();
        spawn_local(async move {
            let result = with_mutation_timeout("读取测试投递处理结果", client.webhook_status()).await;
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(status) => observe.run(status),
                Err(error) => problem.set(Some(error.problem)),
            }
        });
    });
    let recheck = journal.recheck(Callback::new(move |run: ActionRun| {
        problem.set(run.problem);
        if run.status == ActionRunStatus::Succeeded {
            if let Some(Ok(result)) = run
                .result
                .map(serde_json::from_value::<WebhookTestResponse>)
            {
                receipt.set(Some(result));
                message.set(Some("测试消息已排队，等待原消息的投递处理结果。".into()));
                refresh.run(());
            }
        } else {
            receipt.set(None);
            message.set(Some("原测试请求未成功入队，可以重新发起。".into()));
        }
    }));
    let send = Callback::new(move |request: WebhookTestRequest| {
        if pending.get_untracked() {
            return;
        }
        let Some(attempt) = journal.begin(ActionRunKind::WebhookTest, "webhook-test".into()) else {
            return;
        };
        message.set(None);
        problem.set(None);
        receipt.set(None);
        let client = journal.client();
        let epoch = journal.epoch.get_untracked();
        spawn_local(async move {
            let result = with_mutation_timeout(
                "提交测试消息",
                client.test_webhook(&request, &attempt.context),
            )
            .await
            .and_then(|response| {
                validate_setting_response(&attempt, &response)?;
                Ok(response)
            });
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(response) => {
                    journal.remember_run(&attempt, response.action_run_id.clone());
                    receipt.set(Some(response));
                    message.set(Some("测试消息已排队，等待原消息的投递处理结果。".into()));
                    journal.busy.set(false);
                    refresh.run(());
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    problem.set(Some(error.problem));
                    message.set(Some(
                        if journal.locked() {
                            "测试请求结果未知，请核对原请求；不会自动重发。"
                        } else {
                            "测试请求未成功入队。"
                        }
                        .into(),
                    ));
                }
            }
        });
    });
    Effect::new(move |_| {
        journal.connection.track();
        receipt.set(None);
        message.set(None);
        problem.set(None);
    });
    provide_context(WebhookTestRuntime {
        config,
        journal,
        pending,
        message,
        problem,
        receipt,
        send,
        recheck,
    });
}

fn accept_delivery(
    journal: OperationJournal,
    receipt: RwSignal<Option<WebhookTestResponse>>,
    message: RwSignal<Option<String>>,
    problem: RwSignal<Option<ApiProblem>>,
    status: &WebhookRuntimeStatus,
) {
    let Some(response) = receipt.get_untracked() else {
        return;
    };
    let Some(attempt) = journal.pending.get_untracked() else {
        return;
    };
    if response.request_id.as_deref() != Some(attempt.context.request_id())
        || response.idempotency_key.as_deref() != attempt.context.idempotency_key()
    {
        return;
    }
    let Some(row) = status
        .recent_deliveries
        .iter()
        .find(|row| row.event_id == response.event_id && row.kind == WebhookEventKind::Test)
    else {
        return;
    };
    let detail = match row.status {
        WebhookDeliveryStatus::Queued => "测试消息已排队，尚未取得最终投递处理结果。",
        WebhookDeliveryStatus::Delivered => match row.application_ack {
            WebhookApplicationAck::Accepted => "测试消息：应用已确认接收，不代表手机已展示。",
            WebhookApplicationAck::TransportOnly => "测试消息：仅传输成功，未取得应用接收确认。",
            _ => "测试消息：传输已结束，应用接收状态待确认。",
        },
        WebhookDeliveryStatus::Failed => "测试消息投递失败，可查看原消息的失败详情。",
        WebhookDeliveryStatus::Dropped => "测试消息被队列丢弃，未送达。",
        WebhookDeliveryStatus::Disabled => "测试消息投递已停用，未送达。",
    };
    problem.set(None);
    message.set(Some(detail.into()));
    if row.status != WebhookDeliveryStatus::Queued {
        journal.resolve(&attempt);
    }
}

impl WebhookTestRuntime {
    pub(crate) fn watch(self, state: RwSignal<LoadState<WebhookRuntimeStatus>>) {
        Effect::new(move |_| {
            let snapshot = state.get();
            self.receipt.track();
            if let LoadState::Ready(status) = snapshot {
                accept_delivery(
                    self.journal,
                    self.receipt,
                    self.message,
                    self.problem,
                    &status,
                );
            }
        });
    }
}

pub(crate) fn webhook_test_feedback(runtime: WebhookTestRuntime) -> impl IntoView {
    view! {
        <div class="webhook-test-feedback" aria-live="polite">
            <Show when=move || runtime.journal.locked()>
                <div class="provider-credentials-feedback provider-credentials-recovery has-action" role="alert" aria-label="测试投递待核对">
                    <span class="provider-credentials-recovery-copy">
                        <strong>{move || if runtime.journal.busy.get() { "测试请求处理中" } else if runtime.receipt.get().is_some() { "测试消息等待投递结果" } else { "测试请求结果待核对" }}</strong>
                        <span>"核对原消息，不重复发送。"</span>
                        {move || runtime.journal.problem.get().map(|text| view! { <span>{text}</span> })}
                    </span>
                    <button type="button" class="row-action" disabled=move || runtime.journal.busy.get()
                        on:click=move |_| runtime.recheck.run(())>"核对测试投递"</button>
                </div>
            </Show>
            {move || runtime.message.get().map(|text| view! { <p role="status">{text}</p> })}
            {move || runtime.problem.get().map(|error| view! { <p class="state-note is-error" role="alert">{format!("测试投递未确认 · {} · {}", error.code, error.message)}</p> })}
            {move || runtime.receipt.get().map(|receipt| view! {
                <details class="webhook-monitor-history"><summary>"本次测试消息数据依据"</summary>
                    <span style="overflow-wrap:anywhere;">{format!("event {} · action {}", receipt.event_id, receipt.action_run_id)}</span>
                </details>
            })}
        </div>
    }
}
