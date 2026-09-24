use crate::panels::shared::WebhookTestFeedback;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::use_conditional_polling_load_state;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ApiProblem, WebhookRuntimeStatus, WebhookTestRequest};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::opportunities) struct OpportunityWebhookData {
    pub state: RwSignal<LoadState<WebhookRuntimeStatus>>,
    pub action_problem: RwSignal<Option<ApiProblem>>,
    pub test: Callback<WebhookTestRequest>,
    pub feedback: WebhookTestFeedback,
}

pub(in crate::panels::modules::opportunities) fn use_opportunity_webhook() -> OpportunityWebhookData
{
    let client = use_global().client;
    let state = use_conditional_polling_load_state(Duration::from_secs(5), || true, {
        let client = client.clone();
        move || {
            let client = client.clone();
            async move { client.webhook_status().await.map_err(|error| error.problem) }
        }
    });
    let action_problem = RwSignal::new(None);
    let feedback = WebhookTestFeedback {
        pending: RwSignal::new(false),
        message: RwSignal::new(None),
    };
    let test = Callback::new(move |request| {
        if feedback.pending.get_untracked() {
            return;
        }
        feedback.pending.set(true);
        feedback.message.set(None);
        let client = client.clone();
        action_problem.set(None);
        spawn_local(async move {
            let result = client.test_webhook(&request).await;
            if feedback.pending.try_get_untracked().is_none() {
                return;
            }
            feedback.pending.set(false);
            match result {
                Err(error) => action_problem.set(Some(error.problem)),
                Ok(()) => feedback.message.set(Some(
                    "测试请求已受理；实际送达以最近投递回执为准".to_owned(),
                )),
            }
        });
    });
    OpportunityWebhookData {
        state,
        action_problem,
        test,
        feedback,
    }
}
