use crate::panels::shared::execution_environment_label;
use crate::state::context::use_global;
use leptos::{prelude::*, task::spawn_local};
use shared_types::{
    ExecutionArtifactStatus, ExecutionArtifactValidationRequest,
    ExecutionArtifactValidationResponse,
};

pub(in crate::panels::modules::execution) fn artifact_inbox(clock: RwSignal<i64>) -> impl IntoView {
    let client = use_global().client;
    let input = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let version = RwSignal::new(0_u64);
    let result = RwSignal::new(None::<Result<ExecutionArtifactValidationResponse, String>>);
    let verify = Callback::new(move |()| {
        if pending.get_untracked() {
            return;
        }
        let request =
            match ExecutionArtifactValidationRequest::from_handoff_code(&input.get_untracked()) {
                Ok(value) => value,
                Err(error) => {
                    result.set(Some(Err(error.into())));
                    return;
                }
            };
        let current = version.get_untracked();
        let client = client.clone();
        pending.set(true);
        result.set(None);
        spawn_local(async move {
            let response = client
                .validate_execution_artifact(&request)
                .await
                .map_err(|error| error.problem.message)
                .and_then(|response| {
                    if response
                        .artifact
                        .as_ref()
                        .is_some_and(|artifact| artifact.validation_request() != request)
                    {
                        Err("返回的票据与校验码不一致；未导入".into())
                    } else {
                        Ok(response)
                    }
                });
            if version.try_get_untracked() == Some(current) {
                pending.set(false);
                result.set(Some(response));
            }
        });
    });
    view! {
        <details class="execution-artifact-inbox">
            <summary>"提醒票据"<span>"只读核验"</span></summary>
            <div class="execution-artifact-inbox-body">
                <label for="execution-handoff-code">"Webhook 校验码"</label>
                <div class="execution-artifact-inbox-input">
                    <textarea id="execution-handoff-code" rows="2" maxlength="4096"
                        spellcheck="false" autocomplete="off" placeholder="CROSSLINE:…"
                        prop:value=move || input.get()
                        on:input=move |event| {
                            input.set(event_target_value(&event));
                            version.update(|value| *value = value.wrapping_add(1));
                            result.set(None);
                            pending.set(false);
                        } />
                    <button class="btn-secondary" type="button"
                        disabled=move || pending.get() || input.get().trim().is_empty()
                        on:click=move |_| verify.run(())>
                        {move || if pending.get() { "核验中" } else { "校验提醒票据" }}
                    </button>
                </div>
                {move || result.get().map(|value| match value {
                    Err(error) => view! { <p class="execution-artifact-validation" role="alert">{error}</p> }.into_any(),
                    Ok(response) => {
                        let trusted = response.artifact.as_ref().filter(|_| matches!(response.status,
                            ExecutionArtifactStatus::Ready | ExecutionArtifactStatus::Expired | ExecutionArtifactStatus::Blocked));
                        let expires = trusted.and_then(super::super::data::artifact_valid_until);
                        let valid = response.valid && response.status.is_ready()
                            && response.blockers.is_empty()
                            && expires.is_some_and(|until| clock.get() < until);
                        let label = if valid { "提醒票据校验通过 · 未下单" }
                            else if response.status == ExecutionArtifactStatus::Expired || expires.is_some_and(|until| clock.get() >= until) {
                                "提醒票据已过期 · 请查看当前机会"
                            } else { "提醒票据未通过 · 未下单" };
                        let current = trusted.map(|artifact| {
                            let params = web_sys::UrlSearchParams::new().expect("empty query parameters");
                            params.append("symbol", &artifact.symbol);
                            params.append("opp", &artifact.opportunity_id);
                            params.append("page", "0");
                            if let Some(strategy) = artifact.strategy { params.append("strategy", strategy.as_query_value()); }
                            let href = format!("#opportunities?{}", params.to_string());
                            let identity = format!("{} · {} · ${:.2} · {}", artifact.symbol,
                                execution_environment_label(artifact.environment), artifact.capital_usd, artifact.ticket_id);
                            view! { <div class="execution-artifact-inbox-result">
                                <span>{identity}</span><a href=href>"查看当前机会"</a>
                            </div> }
                        });
                        view! { <div role="status">
                            <strong>{label}</strong>
                            {current}
                            <p class="execution-artifact-validation">"提醒只读；当前交易须重新构建和预检。"</p>
                            {(!response.blockers.is_empty()).then(|| view! {
                                <p class="execution-artifact-validation">{response.blockers.join(" · ")}</p>
                            })}
                        </div> }.into_any()
                    }
                })}
            </div>
        </details>
    }
}
