use crate::panels::modules::execution::data::ExecutionPreview;
use crate::panels::modules::execution::data::{artifact_valid_until, ExecutionArtifactRuntime};
use crate::panels::modules::execution::ExecutionSelection;
use crate::panels::shared::{
    deterministic_flow_rail, DeterministicFlowStage, DeterministicFlowState,
};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ExecutionArtifactStatus, ExecutionRun, ExecutionRunState};

#[path = "deterministic_flow/history.rs"]
mod history;

pub(in crate::panels::modules::execution) fn execution_deterministic_flow(
    selection: Memo<ExecutionSelection>,
    preview: Memo<ExecutionPreview>,
    artifact: ExecutionArtifactRuntime,
    run: RwSignal<Option<ExecutionRun>>,
    submission_pending: Memo<bool>,
    requested_history: Memo<bool>,
    read_problem: Memo<Option<shared_types::ApiProblem>>,
) -> impl IntoView {
    let historical = Memo::new(move |_| selection.get().opportunity_id.trim().is_empty()
        && (requested_history.get() || run.get().is_some()));
    let stages = Memo::new(move |_| {
        if historical.get() {
            return history::stages(run.get().as_ref(), read_problem.get().as_ref());
        }
        let selection = selection.get();
        let preview = preview.get();
        let artifact_state = artifact.state.get();
        let validation = artifact.validation.get();
        let run = run.get();
        let run = current_artifact_run(
            &selection,
            preview.ticket_id.as_deref(),
            &artifact_state,
            run.as_ref(),
        );
        vec![
            qualification_stage(&selection),
            DeterministicFlowStage::new("Webhook", "来源页核对", DeterministicFlowState::Idle),
            artifact_stage(
                &artifact_state,
                &validation,
                run,
                artifact.ready.get(),
                artifact.validated.get(),
                artifact.clock.get(),
            ),
            if submission_pending.get() {
                DeterministicFlowStage::new("双腿提交", "原提交待核对 · 不重复下单", DeterministicFlowState::Warning)
            } else { submission_stage(run) },
            finality_stage(run),
            exit_stage(run),
            review_stage(run),
        ]
    });
    let summary = Memo::new(move |_| {
        if submission_pending.get() {
            ExecutionFlowSummary {
                label: "原提交待核对".into(),
                detail: "只查询原请求，不重新下单".into(),
                state: DeterministicFlowState::Warning,
            }
        } else if historical.get() {
            history::summary(run.get().as_ref(), read_problem.get().as_ref())
        } else { summarize_flow(&selection.get(), &stages.get()) }
    });
    let awaiting_selection = Memo::new(move |_| !submission_pending.get() && !historical.get()
        && selection.get().opportunity_id.trim().is_empty());
    view! {
                <section
                    class="execution-flow-overview"
                    class:awaiting-selection=move || awaiting_selection.get()
                    class:historical=move || historical.get()
                    data-state=move || flow_state_token(summary.get().state)
                >
                    <div class="execution-flow-current">
                        <span>{move || if historical.get() { "历史执行" } else { "执行路径" }}</span>
                        <strong>{move || summary.get().label}</strong>
                        <em>{move || summary.get().detail}</em>
                    </div>
                    <Show when=move || awaiting_selection.get()>
                        <nav class="execution-flow-sources" aria-label="选择执行机会">
                            <a href="#futures" aria-label="前往期货套利选择机会">"期货套利"</a>
                            <a href="#opportunities" aria-label="前往机会扫描选择机会">"机会扫描"</a>
                        </nav>
                    </Show>
                    <details class="execution-flow-details">
                        <summary>{move || if historical.get() { "查看原运行阶段" } else { "查看 7 个阶段" }}</summary>
                        {move || deterministic_flow_rail("对冲执行路径", stages.get())}
                    </details>
                </section>
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecutionFlowSummary {
    label: String,
    detail: String,
    state: DeterministicFlowState,
}

fn summarize_flow(
    selection: &ExecutionSelection,
    stages: &[DeterministicFlowStage],
) -> ExecutionFlowSummary {
    if selection.opportunity_id.trim().is_empty() {
        return ExecutionFlowSummary {
            label: "等待选择机会".into(),
            detail: "选择可检查交易候选后创建新的双腿票据".to_owned(),
            state: DeterministicFlowState::Idle,
        };
    }
    let stage = stages
        .iter()
        .find(|stage| stage.state == DeterministicFlowState::Blocked)
        .or_else(|| {
            stages
                .iter()
                .find(|stage| stage.state == DeterministicFlowState::Warning)
        })
        .or_else(|| {
            stages
                .iter()
                .find(|stage| stage.state == DeterministicFlowState::Current)
        })
        .or_else(|| {
            stages
                .iter()
                .rev()
                .find(|stage| stage.state == DeterministicFlowState::Complete)
        })
        .unwrap_or(&stages[0]);
    ExecutionFlowSummary {
        label: stage.label.into(),
        detail: stage.detail.clone(),
        state: stage.state,
    }
}

const fn flow_state_token(state: DeterministicFlowState) -> &'static str {
    match state {
        DeterministicFlowState::Idle => "idle",
        DeterministicFlowState::Current => "current",
        DeterministicFlowState::Complete => "complete",
        DeterministicFlowState::Warning => "warning",
        DeterministicFlowState::Blocked => "blocked",
    }
}

fn current_artifact_run<'a>(
    selection: &ExecutionSelection,
    preview_ticket_id: Option<&str>,
    artifact: &LoadState<Option<shared_types::DeterministicExecutionArtifact>>,
    run: Option<&'a ExecutionRun>,
) -> Option<&'a ExecutionRun> {
    let run = run?;
    let current_ticket_id = preview_ticket_id.or_else(|| {
        artifact
            .value()
            .and_then(Option::as_ref)
            .map(|artifact| artifact.ticket_id.as_str())
    });
    selection.matches_run(current_ticket_id, run).then_some(run)
}

fn qualification_stage(selection: &ExecutionSelection) -> DeterministicFlowStage {
    if selection.opportunity_id.trim().is_empty() {
        return DeterministicFlowStage::new("资格判定", "等待机会", DeterministicFlowState::Idle);
    }
    if let Some(blocker) = selection.execution_blockers.first() {
        return DeterministicFlowStage::new(
            "资格判定",
            blocker.clone(),
            DeterministicFlowState::Blocked,
        );
    }
    DeterministicFlowStage::new(
        "资格判定",
        format!("{} · 已绑定", selection.pair),
        DeterministicFlowState::Complete,
    )
}

fn artifact_stage(
    artifact: &LoadState<Option<shared_types::DeterministicExecutionArtifact>>,
    validation: &LoadState<Option<shared_types::ExecutionArtifactValidationResponse>>,
    run: Option<&ExecutionRun>,
    ready: bool,
    validated: bool,
    now_ms: i64,
) -> DeterministicFlowStage {
    if run.is_some() {
        return DeterministicFlowStage::new(
            "复查计划",
            "已绑定 ExecutionRun",
            DeterministicFlowState::Complete,
        );
    }
    if let Some(problem) = artifact.problem().or_else(|| validation.problem()) {
        return DeterministicFlowStage::new(
            "票据校验",
            problem.message.clone(),
            DeterministicFlowState::Blocked,
        );
    }
    if validated {
        return DeterministicFlowStage::new(
            "票据校验",
            "当前票据已校验",
            DeterministicFlowState::Complete,
        );
    }
    if matches!(validation, LoadState::Loading) && ready {
        return DeterministicFlowStage::new(
            "复查计划",
            "服务端重验中",
            DeterministicFlowState::Current,
        );
    }
    if validation
        .value()
        .and_then(Option::as_ref)
        .is_some_and(|result| {
            result
                .expires_at_ms
                .is_some_and(|expires| now_ms >= expires)
        })
    {
        return DeterministicFlowStage::new(
            "票据校验",
            "EXPIRED · 校验已过期",
            DeterministicFlowState::Warning,
        );
    }
    if let Some(result) = validation
        .value()
        .and_then(Option::as_ref)
        .filter(|result| !result.valid)
    {
        return DeterministicFlowStage::new(
            "复查计划",
            if result.valid {
                "Checksum 与快照匹配"
            } else {
                artifact_status_label(result.status)
            },
            artifact_flow_state(result.status, result.valid),
        );
    }
    match artifact {
        LoadState::Loading => {
            DeterministicFlowStage::new("复查计划", "绑定快照中", DeterministicFlowState::Current)
        }
        LoadState::Error(problem) => DeterministicFlowStage::new(
            "复查计划",
            problem.message.clone(),
            DeterministicFlowState::Blocked,
        ),
        LoadState::Ready(Some(artifact))
        | LoadState::Stale {
            value: Some(artifact),
            ..
        } => {
            let state = if artifact_valid_until(artifact).is_some_and(|expires| now_ms >= expires) {
                ExecutionArtifactStatus::Expired
            } else {
                artifact.status
            };
            DeterministicFlowStage::new(
                "复查计划",
                if state.is_ready() {
                    if ready {
                        "待校验当前票据"
                    } else {
                        "等待当前参数交易检查"
                    }
                } else {
                    artifact_status_label(state)
                },
                if state.is_ready() {
                    DeterministicFlowState::Current
                } else {
                    artifact_flow_state(state, false)
                },
            )
        }
        LoadState::Ready(None) | LoadState::Stale { value: None, .. } => {
            DeterministicFlowStage::new("复查计划", "等待预览", DeterministicFlowState::Idle)
        }
    }
}

fn submission_stage(run: Option<&ExecutionRun>) -> DeterministicFlowStage {
    let Some(run) = run else {
        return DeterministicFlowStage::new("双腿提交", "尚未提交", DeterministicFlowState::Idle);
    };
    match run.state {
        ExecutionRunState::Previewed | ExecutionRunState::RiskChecked => {
            DeterministicFlowStage::new("双腿提交", "待人工确认", DeterministicFlowState::Current)
        }
        ExecutionRunState::SubmittingFirstLeg
        | ExecutionRunState::FirstLegPartial
        | ExecutionRunState::SubmittingSecondLeg => {
            DeterministicFlowStage::new("双腿提交", "提交中", DeterministicFlowState::Current)
        }
        ExecutionRunState::FailedSafe => {
            DeterministicFlowStage::new("双腿提交", "安全失败", DeterministicFlowState::Blocked)
        }
        ExecutionRunState::SecondLegSubmitted
        | ExecutionRunState::Hedged
        | ExecutionRunState::UnwindRequired
        | ExecutionRunState::Unwinding
        | ExecutionRunState::Closed => {
            DeterministicFlowStage::new("双腿提交", if !run.long_leg.order_ids.is_empty()
                && !run.short_leg.order_ids.is_empty() { "双腿订单已记录" } else { "原提交记录已更新" },
                DeterministicFlowState::Complete)
        }
    }
}

fn finality_stage(run: Option<&ExecutionRun>) -> DeterministicFlowStage {
    let Some(run) = run else {
        return DeterministicFlowStage::new(
            "受理 / 结果",
            "等待运行单",
            DeterministicFlowState::Idle,
        );
    };
    match run.state {
        ExecutionRunState::SecondLegSubmitted => DeterministicFlowStage::new(
            "受理 / 结果",
            "等待成交最终结果",
            DeterministicFlowState::Current,
        ),
        ExecutionRunState::Hedged if !run_legs_filled(run) => DeterministicFlowStage::new(
            "受理 / 结果",
            "双腿成交回报未齐，等待确认",
            DeterministicFlowState::Warning,
        ),
        ExecutionRunState::Hedged => DeterministicFlowStage::new(
            "受理 / 结果",
            "双腿最终结果已确认",
            DeterministicFlowState::Complete,
        ),
        ExecutionRunState::Closed => DeterministicFlowStage::new(
            "受理 / 结果",
            if run_legs_filled(run) { "原双腿成交已确认" } else { "原订单与补偿最终结果待核对" },
            if run_legs_filled(run) { DeterministicFlowState::Complete } else { DeterministicFlowState::Warning },
        ),
        ExecutionRunState::UnwindRequired | ExecutionRunState::Unwinding => {
            DeterministicFlowStage::new(
                "受理 / 结果",
                "存在裸腿，正在收口",
                DeterministicFlowState::Warning,
            )
        }
        ExecutionRunState::FailedSafe => {
            DeterministicFlowStage::new("受理 / 结果", "最终结果失败", DeterministicFlowState::Blocked)
        }
        _ => DeterministicFlowStage::new(
            "受理 / 结果",
            "等待双腿 受理确认",
            DeterministicFlowState::Current,
        ),
    }
}

fn exit_stage(run: Option<&ExecutionRun>) -> DeterministicFlowStage {
    match run.map(|run| run.state) {
        Some(ExecutionRunState::Hedged) => DeterministicFlowStage::new(
            "保护退出",
            if run.is_some_and(run_legs_filled) { "原运行已对冲 · 当前持仓待核对" }
                else { "成交未确认 · 不推断已持仓" },
            DeterministicFlowState::Current,
        ),
        Some(ExecutionRunState::UnwindRequired) => {
            DeterministicFlowStage::new("保护退出", "需要补偿收口", DeterministicFlowState::Warning)
        }
        Some(ExecutionRunState::Unwinding) => {
            DeterministicFlowStage::new("保护退出", "反向处理中", DeterministicFlowState::Current)
        }
        Some(ExecutionRunState::Closed) => {
            DeterministicFlowStage::new("保护退出", "执行已收口 · 核对平仓或补偿记录", DeterministicFlowState::Complete)
        }
        Some(ExecutionRunState::FailedSafe) => {
            DeterministicFlowStage::new("保护退出", "需人工复核", DeterministicFlowState::Blocked)
        }
        _ => DeterministicFlowStage::new("保护退出", "等待配对持仓", DeterministicFlowState::Idle),
    }
}

fn review_stage(run: Option<&ExecutionRun>) -> DeterministicFlowStage {
    match run.map(|run| run.state) {
        Some(ExecutionRunState::Closed) => {
            DeterministicFlowStage::new("复盘", "原运行记录可复核", DeterministicFlowState::Complete)
        }
        Some(ExecutionRunState::FailedSafe) => {
            DeterministicFlowStage::new("复盘", "失败记录可复核", DeterministicFlowState::Warning)
        }
        _ => DeterministicFlowStage::new("复盘", "等待最终结果", DeterministicFlowState::Idle),
    }
}

fn run_legs_filled(run: &ExecutionRun) -> bool {
    run.long_leg.state == shared_types::LiveOrderState::Filled
        && run.short_leg.state == shared_types::LiveOrderState::Filled
}

fn artifact_flow_state(status: ExecutionArtifactStatus, valid: bool) -> DeterministicFlowState {
    match status {
        ExecutionArtifactStatus::Ready if valid => DeterministicFlowState::Complete,
        ExecutionArtifactStatus::Ready => DeterministicFlowState::Current,
        ExecutionArtifactStatus::Expired | ExecutionArtifactStatus::Unknown => {
            DeterministicFlowState::Warning
        }
        ExecutionArtifactStatus::Blocked
        | ExecutionArtifactStatus::Missing
        | ExecutionArtifactStatus::Tampered => DeterministicFlowState::Blocked,
    }
}

const fn artifact_status_label(status: ExecutionArtifactStatus) -> &'static str {
    match status {
        ExecutionArtifactStatus::Ready => "READY",
        ExecutionArtifactStatus::Blocked => "BLOCKED",
        ExecutionArtifactStatus::Expired => "EXPIRED",
        ExecutionArtifactStatus::Missing => "MISSING",
        ExecutionArtifactStatus::Tampered => "TAMPERED",
        ExecutionArtifactStatus::Unknown => "UNKNOWN",
    }
}

#[cfg(test)]
#[path = "deterministic_flow/tests.rs"]
mod tests;
