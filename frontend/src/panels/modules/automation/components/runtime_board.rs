use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    AutoProfitCloseConfig, AutomationRuntimeStatus, DeterministicExecutionArtifact,
    ExecutionArtifactStatus, ExecutionEnvironment, WebhookRuntimeStatus,
};

use super::super::format::{
    cooldown_label, decision_label, decision_reason_label, decision_tone, environment_label,
    runtime_label, runtime_tone, time_label,
};
use super::super::protection_calibration::protection_capital_ready;

#[path = "runtime_board/flow.rs"]
mod flow;
use flow::{
    artifact_status_label, automation_flow, current_artifact, current_decision, current_time_ms,
    effective_artifact_status,
};

pub(in crate::panels::modules::automation) fn runtime_board(
    state: RwSignal<LoadState<AutomationRuntimeStatus>>,
    protection: RwSignal<LoadState<AutoProfitCloseConfig>>,
    inspect_run: Callback<String>,
) -> impl IntoView {
    view! {
        <section class="automation-runtime-board">
            {move || runtime_state(state.get(), inspect_run)}
            {guard_summary(state, protection)}
        </section>
    }
}

pub(in crate::panels::modules::automation) fn execution_evidence(
    state: RwSignal<LoadState<AutomationRuntimeStatus>>,
    protection: RwSignal<LoadState<AutoProfitCloseConfig>>,
    webhook: RwSignal<LoadState<WebhookRuntimeStatus>>,
    receipt: RwSignal<LoadState<shared_types::AutomationExecutionReceipt>>,
) -> impl IntoView {
    view! {
        {move || automation_flow(&state.get(), &webhook.get(), &protection.get(), &receipt.get())}
    }
}

fn runtime_state(
    state: LoadState<AutomationRuntimeStatus>,
    inspect_run: Callback<String>,
) -> AnyView {
    match state {
        LoadState::Loading => empty_state("加载中", "正在连接自动化运行状态…", false),
        LoadState::Error(problem) if problem.code == "AUTOMATION_RESULT_UNKNOWN" =>
            empty_state("操作结果待核对", &problem.message, false),
        LoadState::Error(problem) => empty_state("读取失败", &problem.message, true),
        LoadState::Ready(status) => status_view(&status, true, inspect_run).into_any(),
        LoadState::Stale { value: status, .. } => {
            status_view(&status, false, inspect_run).into_any()
        }
    }
}

fn status_view(
    status: &AutomationRuntimeStatus,
    confirmed: bool,
    inspect_run: Callback<String>,
) -> impl IntoView {
    let runtime_class = format!(
        "automation-runtime-state {}",
        if confirmed {
            runtime_tone(status.state)
        } else {
            "is-warning"
        }
    );
    let enabled = status.config.enabled;
    let latest = current_decision(status).cloned();
    let artifact = current_artifact(status).cloned();
    let waiting_to_start = !enabled && latest.is_none() && artifact.is_none();
    let (empty_artifact_title, empty_artifact_detail) = if enabled {
        (
            "等待通过全部执行门槛的机会",
            "身份、行情、成本、深度、规格与风险全部通过后生成。",
        )
    } else {
        (
            "自动化关闭期间不生成执行信息",
            "启动后才会监控候选，并在全部执行门槛通过后生成。",
        )
    };
    let (empty_decision_title, empty_decision_detail) = if enabled {
        ("尚无决策", "监控中会显示候选选择、交易检查阻断或提交结果。")
    } else {
        ("当前未运行", "启动后才会产生候选判断、交易检查或提交结果。")
    };
    view! {
        <header class="automation-board-header">
            <div>
                <span>"自动化运行状态"</span>
                <h2>{if confirmed { runtime_label(status.state) } else { "运行状态待确认" }}</h2>
            </div>
            <div class=runtime_class>
                <span>{if !confirmed { "上次已知状态" } else if status.config.enabled { "策略已启用" } else { "策略已关闭" }}</span>
                <strong>{environment_label(status.config.environment)}</strong>
                <small>{format!("更新 {}", time_label(status.updated_at_ms))}</small>
            </div>
        </header>

        <div class="automation-metric-strip">
            <div><span>"活跃执行"</span><strong>{format!("{} / {}", status.active_run_count, status.config.max_concurrent_runs)}</strong><small>"当前 / 并发上限"</small></div>
            <div><span>"费后净差门槛"</span><strong>{format!("{}%", status.config.min_one_cycle_net_bps / 100.0)}</strong><small>"单周期"</small></div>
            <div><span>"双腿深度"</span><strong>{format!("${:.0}", status.config.min_depth_usd)}</strong><small>"最低要求"</small></div>
            <div><span>"冷却"</span><strong>{cooldown_label(status.state, status.cooldown_until_ms)}</strong><small>{format!("配置 {}s", status.config.cooldown_secs)}</small></div>
        </div>

        {(!waiting_to_start).then(|| artifact.map_or_else(
            || view! {
                <section class="automation-artifact-summary is-empty">
                    <div><span>"执行计划"</span><strong>{empty_artifact_title}</strong></div>
                    <small>{empty_artifact_detail}</small>
                </section>
            }.into_any(),
            |artifact| artifact_summary(&artifact, confirmed),
        ))}

        <div class=if waiting_to_start { "automation-focus-grid is-idle" } else { "automation-focus-grid" }>
            {if waiting_to_start {
                view! {
                    <section class="automation-idle-summary">
                        <div><span>"当前任务"</span><strong>"等待启动"</strong></div>
                        <p>"当前不监控候选，也不会生成执行计划或提交双腿。"</p>
                    </section>
                }.into_any()
            } else {
                view! {
                    <section class="automation-current-decision">
                        <header><span>"最近决策"</span><small>"后端最近记录"</small></header>
                        {latest.map_or_else(
                            || view! {
                                <div class="automation-empty-decision"><strong>{empty_decision_title}</strong><span>{empty_decision_detail}</span></div>
                            }.into_any(),
                            |decision| {
                                let class = format!("automation-decision-kind {}", decision_tone(decision.kind));
                                view! {
                                    <div class="automation-decision-focus">
                                        <div class=class><strong>{decision_label(decision.kind)}</strong><span>{time_label(decision.occurred_at_ms)}</span></div>
                                        <h3>{decision.symbol.unwrap_or_else(|| "系统事件".to_owned())}</h3>
                                        <p title=decision.reason.clone()>{decision_reason_label(&decision.reason)}</p>
                                        {decision.execution_run_id.map(|id| view! {
                                            <button type="button" class="row-action" on:click=move |_| inspect_run.run(id.clone())>"查看处理结果"</button>
                                        })}
                                    </div>
                                }.into_any()
                            },
                        )}
                    </section>
                }.into_any()
            }}
        </div>
    }
}

fn guard_summary(
    state: RwSignal<LoadState<AutomationRuntimeStatus>>,
    protection: RwSignal<LoadState<AutoProfitCloseConfig>>,
) -> impl IntoView {
    view! {
        <details class="automation-guard-summary">
            <summary><span>"已保存入场规则"</span><strong>{move || state.with(|state| state.value().map_or_else(|| "等待配置".into(), |status|
                format!("净利 ≥ {:.4}% · 资金 ${}", status.config.min_one_cycle_net_bps / 100.0, status.config.capital_usd)))}</strong></summary>
            <div class="automation-guard-list">{move || state.with(|state| state.value().map(|status| {
                let config = &status.config;
                let labels = [("资金", format!("${}", config.capital_usd)), ("杠杆", format!("{}x", config.leverage)),
                    ("并发", config.max_concurrent_runs.to_string()), ("自动入场", if matches!(state, LoadState::Ready(_)) {
                        effective_config_state_label(config.enabled, config.paused).to_owned() } else { "状态待确认".into() }),
                    ("提交方式", submission_mode_label(config.environment).to_owned()), ("退出保护", protection_summary(&protection.get(), config.capital_usd))];
                labels.into_iter().map(|(label, value)| view! { <div><span>{label}</span><strong>{value}</strong></div> }).collect_view()
            }))}</div>
        </details>
    }
}

fn artifact_summary(artifact: &DeterministicExecutionArtifact, confirmed: bool) -> AnyView {
    let status = if confirmed {
        effective_artifact_status(artifact)
    } else {
        ExecutionArtifactStatus::Unknown
    };
    let status_class = format!("automation-artifact-state {}", artifact_tone(status));
    let evidence_passed = artifact.evidence.iter().filter(|row| row.passed).count();
    let evidence_total = artifact.evidence.len();
    let symbol = artifact.symbol.clone();
    let artifact_id = artifact.artifact_id.clone();
    let summary = format!(
        "净收益 ${:+.4} · 成本 ${:.4} · 数据依据 {}/{} · {}",
        artifact.expected_net_edge_usd,
        artifact.expected_total_cost_usd,
        evidence_passed,
        evidence_total,
        expiry_label(artifact.expires_at_ms),
    );
    view! {
        <section class="automation-artifact-summary">
            <header>
                <div>
                    <span>"执行计划"</span>
                    <strong>{symbol}" · "{artifact_id}</strong>
                </div>
                <div class=status_class><strong>{artifact_status_label(status)}</strong><span>{summary}</span></div>
            </header>
        </section>
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

const fn artifact_tone(status: ExecutionArtifactStatus) -> &'static str {
    match status {
        ExecutionArtifactStatus::Ready => "is-positive",
        ExecutionArtifactStatus::Expired | ExecutionArtifactStatus::Unknown => "is-warning",
        ExecutionArtifactStatus::Blocked
        | ExecutionArtifactStatus::Missing
        | ExecutionArtifactStatus::Tampered => "is-danger",
    }
}

fn protection_summary(state: &LoadState<AutoProfitCloseConfig>, capital_usd: f64) -> String {
    if !matches!(state, LoadState::Ready(_)) {
        return "状态待确认".into();
    }
    let Some(config) = state.value() else {
        return "未知".to_owned();
    };
    let labels = [
        config.enabled.then_some("止盈"),
        config.stop_loss_enabled.then_some("止损"),
        config.liquidation_guard_enabled.then_some("强平"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if labels.is_empty() {
        "未配置".to_owned()
    } else if !protection_capital_ready(config, capital_usd) {
        "需按资金校准".to_owned()
    } else {
        labels.join(" + ")
    }
}

const fn submission_mode_label(environment: ExecutionEnvironment) -> &'static str {
    match environment {
        ExecutionEnvironment::Live => "实盘双腿自动提交",
        ExecutionEnvironment::Paper => "模拟双腿自动执行",
    }
}

const fn effective_config_state_label(enabled: bool, paused: bool) -> &'static str {
    if !enabled {
        "已关闭"
    } else if paused {
        "已暂停"
    } else {
        "监控中"
    }
}

fn empty_state(title: &'static str, detail: &str, error: bool) -> AnyView {
    let class = if error {
        "workbench-empty-state is-error"
    } else {
        "workbench-empty-state"
    };
    view! { <div class=class><strong>{title}</strong><span>{detail.to_owned()}</span></div> }
        .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_mode_matches_automatic_runtime_behavior() {
        assert_eq!(
            submission_mode_label(ExecutionEnvironment::Live),
            "实盘双腿自动提交"
        );
        assert_eq!(
            submission_mode_label(ExecutionEnvironment::Paper),
            "模拟双腿自动执行"
        );
    }

    #[test]
    fn disabled_automation_does_not_render_as_paused() {
        assert_eq!(effective_config_state_label(false, true), "已关闭");
        assert_eq!(effective_config_state_label(true, true), "已暂停");
        assert_eq!(effective_config_state_label(true, false), "监控中");
    }
}
