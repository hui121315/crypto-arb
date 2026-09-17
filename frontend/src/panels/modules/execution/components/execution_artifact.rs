use crate::panels::shared::{copy_text, execution_environment_label};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    DeterministicExecutionArtifact, ExecutionArtifactStatus, ExecutionArtifactValidationResponse,
    ExecutionEnvironment,
};

use super::super::data::{artifact_validation_is_ready, ExecutionArtifactRuntime};

pub(in crate::panels::modules::execution) fn execution_artifact_panel(
    runtime: ExecutionArtifactRuntime,
    reviewed: RwSignal<bool>,
) -> impl IntoView {
    let copied = RwSignal::new(false);
    let artifact_id = Memo::new(move |_| {
        runtime
            .state
            .get()
            .value()
            .and_then(Option::as_ref)
            .map(|artifact| artifact.artifact_id.clone())
    });
    Effect::new(move |previous: Option<Option<String>>| {
        let current = artifact_id.get();
        if previous.as_ref().is_some_and(|value| value != &current) {
            reviewed.set(false);
            copied.set(false);
        }
        current
    });
    view! {
        <section class="execution-artifact" aria-live="polite">
            {move || artifact_state(runtime, reviewed, copied)}
        </section>
    }
}

fn artifact_state(
    runtime: ExecutionArtifactRuntime,
    reviewed: RwSignal<bool>,
    copied: RwSignal<bool>,
) -> AnyView {
    match runtime.state.get() {
        LoadState::Loading => state_message(
            "正在绑定执行工件",
            "校验快照、票据、双腿证据与成本…",
            "is-loading",
        ),
        LoadState::Error(problem) => {
            state_message("执行工件生成失败", &problem.message, "is-blocked")
        }
        LoadState::Ready(None) => state_message(
            "等待可执行预览",
            "预览通过后自动生成短时效、可复制、只读校验工件。",
            "is-idle",
        ),
        LoadState::Stale {
            value: None,
            problem,
        } => state_message("执行工件不可用", &problem.message, "is-blocked"),
        LoadState::Ready(Some(artifact)) => {
            artifact_view(&artifact, runtime, reviewed, copied, None)
        }
        LoadState::Stale {
            value: Some(artifact),
            problem,
        } => artifact_view(&artifact, runtime, reviewed, copied, Some(problem.message)),
    }
}

fn artifact_view(
    artifact: &DeterministicExecutionArtifact,
    runtime: ExecutionArtifactRuntime,
    reviewed: RwSignal<bool>,
    copied: RwSignal<bool>,
    stale_message: Option<String>,
) -> AnyView {
    let status = if current_time_ms() >= artifact.expires_at_ms {
        ExecutionArtifactStatus::Expired
    } else {
        artifact.status
    };
    let status_class = format!("execution-artifact-status {}", status_tone(status));
    let command = artifact.validation_command.clone();
    let evidence = artifact.evidence.clone();
    let (validation, validation_tone, validation_detail) =
        validation_summary(&runtime.validation.get());
    let environment = artifact.environment;
    let live = environment == ExecutionEnvironment::Live;
    let review_label = if live {
        format!(
            "重验通过，允许人工{}提交",
            execution_environment_label(environment)
        )
    } else {
        format!(
            "重验通过，允许{}提交",
            execution_environment_label(environment)
        )
    };
    let review_allowed = status.is_ready()
        && artifact.blockers.is_empty()
        && artifact_validation_is_ready(&runtime.validation.get());
    let expected_net_edge_usd = artifact.expected_net_edge_usd;
    let expected_total_cost_usd = artifact.expected_total_cost_usd;
    let expires_at_ms = artifact.expires_at_ms;
    let symbol = artifact.symbol.clone();
    let artifact_id = artifact.artifact_id.clone();
    let checksum = artifact.checksum.clone();
    let checksum_short = short_checksum(&checksum);
    let net_class = if expected_net_edge_usd > 0.0 {
        "is-positive"
    } else {
        "is-blocked"
    };
    let metrics = format!(
        "费后净边际 ${expected_net_edge_usd:+.4} · 完整成本 ${expected_total_cost_usd:.4} · 有效期 {} · Checksum {checksum_short}",
        expiry_label(expires_at_ms),
    );
    view! {
        <header class="execution-artifact-head">
            <div>
                <span>"快照绑定工件"</span>
                <strong>{symbol}" · "{artifact_id}</strong>
            </div>
            <div class=status_class>
                <strong>{status_label(status)}</strong>
                <span>{execution_environment_label(environment)}</span>
            </div>
        </header>
        <p class=format!("execution-artifact-metrics {net_class}") title=checksum>{metrics}</p>
        <div class="execution-artifact-evidence">
            {evidence.into_iter().map(|row| {
                let passed = row.passed;
                let class = if passed { "is-passed" } else { "is-failed" };
                view! {
                    <div class=class title=row.detail>
                        <span>{row.label}</span>
                        <strong>{if passed { "通过" } else { "阻断" }}</strong>
                    </div>
                }
            }).collect_view()}
        </div>
        <div class="execution-artifact-command">
            <code>{command.clone()}</code>
            <button
                class="btn-secondary"
                type="button"
                title="复制只读校验命令"
                on:click=move |_| {
                    copy_text(&command);
                    copied.set(true);
                }
            >{move || if copied.get() { "已复制" } else { "复制" }}</button>
        </div>
        <div class="execution-artifact-footer">
            <div class=format!("execution-artifact-validation {validation_tone}")>
                <strong>{validation}</strong>
                <span>{stale_message.unwrap_or(validation_detail)}</span>
            </div>
            <button class="btn-secondary" type="button" disabled=move || matches!(runtime.validation.get(), LoadState::Loading) on:click=move |_| runtime.validate.run(())>"服务端重验"</button>
            <label class=if review_allowed { "execution-artifact-review" } else { "execution-artifact-review is-disabled" }>
                <input
                    type="checkbox"
                    disabled=!review_allowed
                    prop:checked=move || reviewed.get()
                    on:change=move |event| reviewed.set(event_target_checked(&event))
                />
                <span>{review_label}</span>
            </label>
        </div>
    }
    .into_any()
}

fn expiry_label(expires_at_ms: i64) -> String {
    let remaining_ms = expires_at_ms.saturating_sub(current_time_ms());
    if remaining_ms <= 0 {
        "已过期".to_owned()
    } else {
        format!("{}s 后", (remaining_ms + 999) / 1_000)
    }
}

fn current_time_ms() -> i64 {
    i64::try_from(crate::state::polling::now_ms()).unwrap_or(i64::MAX)
}

fn state_message(title: &str, detail: &str, tone: &'static str) -> AnyView {
    view! {
        <div class=format!("execution-artifact-empty {tone}")>
            <strong>{title.to_owned()}</strong>
            <span>{detail.to_owned()}</span>
        </div>
    }
    .into_any()
}

fn validation_summary(
    state: &LoadState<Option<ExecutionArtifactValidationResponse>>,
) -> (String, &'static str, String) {
    match state {
        LoadState::Loading => (
            "正在服务端重验".to_owned(),
            "is-current",
            "重新读取快照、双腿 WS 行情与执行证据".to_owned(),
        ),
        LoadState::Ready(Some(result)) => {
            if result.valid {
                (
                    "服务端重验通过".to_owned(),
                    "is-positive",
                    "Checksum、快照、TTL 与证据均匹配".to_owned(),
                )
            } else {
                (
                    format!("服务端重验：{}", status_label(result.status)),
                    "is-blocked",
                    blockers_label(&result.blockers),
                )
            }
        }
        LoadState::Stale { problem, .. } => (
            "服务端重验已失效".to_owned(),
            "is-blocked",
            problem.message.clone(),
        ),
        LoadState::Error(problem) => (
            "服务端重验失败".to_owned(),
            "is-blocked",
            problem.message.clone(),
        ),
        LoadState::Ready(None) => (
            "尚未服务端重验".to_owned(),
            "is-idle",
            "复制命令只读；重验通过后才能复核并提交".to_owned(),
        ),
    }
}

fn blockers_label(blockers: &[String]) -> String {
    if blockers.is_empty() {
        "工件未通过服务端校验".to_owned()
    } else {
        blockers.join(" · ")
    }
}

fn short_checksum(value: &str) -> String {
    if value.len() <= 14 {
        value.to_owned()
    } else {
        format!("{}…{}", &value[..8], &value[value.len() - 6..])
    }
}

const fn status_label(status: ExecutionArtifactStatus) -> &'static str {
    match status {
        ExecutionArtifactStatus::Ready => "READY",
        ExecutionArtifactStatus::Blocked => "BLOCKED",
        ExecutionArtifactStatus::Expired => "EXPIRED",
        ExecutionArtifactStatus::Missing => "MISSING",
        ExecutionArtifactStatus::Tampered => "TAMPERED",
        ExecutionArtifactStatus::Unknown => "UNKNOWN",
    }
}

const fn status_tone(status: ExecutionArtifactStatus) -> &'static str {
    match status {
        ExecutionArtifactStatus::Ready => "is-ready",
        ExecutionArtifactStatus::Expired | ExecutionArtifactStatus::Unknown => "is-warning",
        ExecutionArtifactStatus::Blocked
        | ExecutionArtifactStatus::Missing
        | ExecutionArtifactStatus::Tampered => "is-blocked",
    }
}
