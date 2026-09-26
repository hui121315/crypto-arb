use crate::panels::shared::{copy_text, execution_environment_label};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::ExecutionArtifactStatus;

use super::super::data::{artifact_valid_until, ExecutionArtifactRuntime};

pub(in crate::panels::modules::execution) fn execution_artifact_panel(
    runtime: ExecutionArtifactRuntime,
    reviewed: RwSignal<bool>,
) -> impl IntoView {
    let artifact = Memo::new(move |_| {
        runtime
            .state
            .get()
            .value()
            .and_then(Option::as_ref)
            .cloned()
    });
    let copied = RwSignal::new(false);
    let binding = Memo::new(move |_| {
        artifact.get().map(|value| {
            (
                value.artifact_id,
                value.ticket_id,
                value.opportunity_snapshot_id,
                value.checksum,
            )
        })
    });
    Effect::new(move |_| {
        binding.get();
        copied.set(false);
        reviewed.set(false);
    });
    Effect::new(move |_| {
        if !runtime.validated.get() {
            reviewed.set(false);
        }
    });
    let status = Memo::new(move |_| validation_summary(runtime));
    let pending = Memo::new(move |_| matches!(runtime.validation.get(), LoadState::Loading));
    view! {
        <section class="execution-artifact" aria-label="提交前确认">
            <Show when=move || artifact.get().is_some() fallback=move || {
                let (title, detail) = match runtime.state.get() {
                    LoadState::Loading => ("正在生成校验凭据", "绑定当前参数、快照和双腿票据".to_owned()),
                    LoadState::Error(problem) | LoadState::Stale { problem, .. } => ("校验凭据不可用", problem.message),
                    _ => ("等待交易检查", "当前参数尚未取得可执行票据".to_owned()),
                };
                view! { <div class="execution-artifact-empty"><strong>{title}</strong><span>{detail}</span></div> }
            }>
                <header class="execution-artifact-head">
                    <strong>"提交前确认"</strong>
                    <div class="execution-artifact-status" class:is-ready=move || runtime.validated.get()>
                        <strong>{move || status.get().0}</strong>
                        <span>{move || artifact.get().map(|value| execution_environment_label(value.environment))}</span>
                    </div>
                </header>
                <dl class="execution-artifact-metrics">
                    <div><dt>{move || if runtime.ready.get() { "预计净收益" } else { "上次测算净收益" }}</dt><dd>{move || artifact.get().map(|a| format!("${:+.4}", a.expected_net_edge_usd))}</dd></div>
                    <div><dt>{move || if runtime.ready.get() { "预计总成本" } else { "上次测算总成本" }}</dt><dd>{move || artifact.get().map(|a| format!("${:.4}", a.expected_total_cost_usd))}</dd></div>
                    <div><dt>"凭据有效期"</dt><dd>{move || artifact.get().map(|a| artifact_valid_until(&a).map_or_else(|| "时效数据待确认".to_owned(), |expires| expiry_label(expires, runtime.clock.get())))}</dd></div>
                </dl>
                <div class="execution-artifact-footer">
                    <p class="execution-artifact-validation" role="status">{move || status.get().1}</p>
                    <button class="btn-secondary" type="button"
                        disabled=move || pending.get() || !runtime.ready.get()
                        on:click=move |_| runtime.validate.run(())>
                        {move || if pending.get() { "校验中" } else { "校验票据" }}
                    </button>
                </div>
                <label class="execution-artifact-review" class:is-disabled=move || !runtime.validated.get()>
                    <input type="checkbox" disabled=move || !runtime.validated.get()
                        prop:checked=move || reviewed.get()
                        on:change=move |event| reviewed.set(runtime.validated.get_untracked() && event_target_checked(&event)) />
                    <span>{move || artifact.get().map(|a| format!("已核对双腿、金额与成本，确认{}提交", execution_environment_label(a.environment)))}</span>
                </label>
                <details class="execution-artifact-details">
                    <summary>"票据与校验依据"</summary>
                    <dl class="execution-artifact-identifiers">
                        <dt>"票据"</dt><dd>{move || artifact.get().map(|a| a.ticket_id)}</dd>
                        <dt>"快照"</dt><dd>{move || artifact.get().map(|a| a.opportunity_snapshot_id)}</dd>
                        <dt>"校验码"</dt><dd>{move || artifact.get().map(|a| a.checksum)}</dd>
                    </dl>
                    <div class="execution-artifact-evidence">
                        <For each=move || artifact.get().map(|a| a.evidence).unwrap_or_default()
                            key=|row| (row.key.clone(), row.detail.clone(), row.passed)
                            children=move |row| view! {
                                <div class:failed=!row.passed><span>{row.label}</span><strong>{if row.passed { "通过" } else { "阻断" }}</strong><p>{row.detail}</p></div>
                            } />
                    </div>
                    <div class="execution-artifact-command">
                        <code>{move || artifact.get().map(|a| a.validation_command)}</code>
                        <button class="btn-secondary" type="button" title="复制只读校验命令"
                            on:click=move |_| {
                                if let Some(artifact) = artifact.get_untracked() {
                                    copy_text(&artifact.validation_command);
                                    copied.set(true);
                                }
                            }>{move || if copied.get() { "已复制" } else { "复制命令" }}</button>
                    </div>
                </details>
            </Show>
        </section>
    }
}

fn expiry_label(expires_at_ms: i64, now_ms: i64) -> String {
    let remaining = expires_at_ms.saturating_sub(now_ms);
    if remaining <= 0 {
        "已过期".to_owned()
    } else {
        format!("{} 秒", (remaining + 999) / 1_000)
    }
}

fn validation_summary(runtime: ExecutionArtifactRuntime) -> (&'static str, String) {
    if let Some(problem) = runtime.state.get().problem() {
        return ("凭据不可用", problem.message.clone());
    }
    if let Some(artifact) = runtime.state.get().value().and_then(Option::as_ref) {
        if artifact_valid_until(artifact).is_some_and(|expires| runtime.clock.get() >= expires) {
            return ("已过期", "请刷新预览，取得当前参数的新票据".to_owned());
        }
        if !artifact.status.is_ready() || !artifact.blockers.is_empty() {
            return ("交易检查阻断", artifact.blockers.join(" · "));
        }
    }
    if !runtime.ready.get() {
        return ("待交易检查", "当前参数与校验凭据尚未匹配".to_owned());
    }
    match runtime.validation.get() {
        LoadState::Loading => (
            "校验中",
            "核对后端已保存票据的快照、有效期和数据依据".to_owned(),
        ),
        LoadState::Ready(Some(_)) if runtime.validated.get() => (
            "校验通过",
            "当前票据有效；提交时仍由后端核对执行条件".to_owned(),
        ),
        LoadState::Ready(Some(result)) => (
            "校验未通过",
            if !result.blockers.is_empty() {
                result.blockers.join(" · ")
            } else if result.status == ExecutionArtifactStatus::Expired
                || result
                    .expires_at_ms
                    .is_some_and(|until| runtime.clock.get() >= until)
            {
                "校验已过期，请刷新预览".to_owned()
            } else {
                "校验结果与当前票据不一致，请重新校验".to_owned()
            },
        ),
        LoadState::Stale { problem, .. } | LoadState::Error(problem) => {
            ("校验失败", problem.message)
        }
        LoadState::Ready(None) => ("待校验", "尚未校验当前票据".to_owned()),
    }
}
