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
    let test = Callback::new(move |request| {
        let client = client.clone();
        action_problem.set(None);
        spawn_local(async move {
            if let Err(error) = client.test_webhook(&request).await {
                action_problem.set(Some(error.problem));
            }
        });
    });
    OpportunityWebhookData {
        state,
        action_problem,
        test,
    }
}
