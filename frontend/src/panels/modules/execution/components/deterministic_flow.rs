use crate::panels::modules::execution::data::ExecutionArtifactRuntime;
use crate::panels::modules::execution::data::ExecutionPreview;
use crate::panels::modules::execution::ExecutionSelection;
use crate::panels::shared::{
    deterministic_flow_rail, DeterministicFlowStage, DeterministicFlowState,
};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ExecutionArtifactStatus, ExecutionRun, ExecutionRunState};

pub(in crate::panels::modules::execution) fn execution_deterministic_flow(
    selection: Memo<ExecutionSelection>,
    preview: Memo<ExecutionPreview>,
    artifact: ExecutionArtifactRuntime,
    run: RwSignal<Option<ExecutionRun>>,
) -> impl IntoView {
    view! {
        {move || {
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
            let stages = vec![
                qualification_stage(&selection),
                DeterministicFlowStage::new(
                    "Webhook",
                    "来源页核验",
                    DeterministicFlowState::Idle,
                ),
                artifact_stage(&artifact_state, &validation, run),
                submission_stage(run),
                finality_stage(run),
                exit_stage(run),
                review_stage(run),
            ];
            let summary = summarize_flow(&selection, &stages);
            let awaiting_selection = selection.opportunity_id.trim().is_empty();
            view! {
                <section
                    class="execution-flow-overview"
                    class:awaiting-selection=awaiting_selection
                    data-state=flow_state_token(summary.state)
                >
                    <div class="execution-flow-current">
                        <span>"执行路径"</span>
                        <strong>{summary.label}</strong>
                        <em>{summary.detail}</em>
                    </div>
                    {awaiting_selection.then(|| view! {
                        <nav class="execution-flow-sources" aria-label="选择执行机会">
                            <a href="#futures" aria-label="前往期货套利选择机会">"期货套利"</a>
                            <a href="#opportunities" aria-label="前往机会扫描选择机会">"机会扫描"</a>
                        </nav>
                    })}
                    <details class="execution-flow-details">
                        <summary>"查看 7 个阶段"</summary>
                        {deterministic_flow_rail("对冲执行确定性闭环", stages)}
                    </details>
                </section>
            }
        }}
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecutionFlowSummary {
    label: &'static str,
    detail: String,
    state: DeterministicFlowState,
}

fn summarize_flow(
    selection: &ExecutionSelection,
    stages: &[DeterministicFlowStage],
) -> ExecutionFlowSummary {
    if selection.opportunity_id.trim().is_empty() {
        return ExecutionFlowSummary {
            label: "等待选择机会",
            detail: "选择可预检候选后创建新的双腿票据".to_owned(),
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
        label: stage.label,
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
) -> DeterministicFlowStage {
    if run.is_some() {
        return DeterministicFlowStage::new(
            "工件重验",
            "已绑定 ExecutionRun",
            DeterministicFlowState::Complete,
        );
    }
    if matches!(validation, LoadState::Loading) {
        return DeterministicFlowStage::new(
            "工件重验",
            "服务端重验中",
            DeterministicFlowState::Current,
        );
    }
    if let Some(result) = validation.value().and_then(Option::as_ref) {
        return DeterministicFlowStage::new(
            "工件重验",
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
            DeterministicFlowStage::new("工件重验", "绑定快照中", DeterministicFlowState::Current)
        }
        LoadState::Error(problem) => DeterministicFlowStage::new(
            "工件重验",
            problem.message.clone(),
            DeterministicFlowState::Blocked,
        ),
        LoadState::Ready(Some(artifact))
        | LoadState::Stale {
            value: Some(artifact),
            ..
        } => {
            let state = if current_time_ms() >= artifact.expires_at_ms {
                ExecutionArtifactStatus::Expired
            } else {
                artifact.status
            };
            DeterministicFlowStage::new(
                "工件重验",
                if state.is_ready() {
                    "待服务端重验"
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
            DeterministicFlowStage::new("工件重验", "等待预览", DeterministicFlowState::Idle)
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
            DeterministicFlowStage::new("双腿提交", "双腿已提交", DeterministicFlowState::Complete)
        }
    }
}

fn finality_stage(run: Option<&ExecutionRun>) -> DeterministicFlowStage {
    let Some(run) = run else {
        return DeterministicFlowStage::new(
            "ACK / 终态",
            "等待运行单",
            DeterministicFlowState::Idle,
        );
    };
    match run.state {
        ExecutionRunState::SecondLegSubmitted => DeterministicFlowStage::new(
            "ACK / 终态",
            "等待成交终态",
            DeterministicFlowState::Current,
        ),
        ExecutionRunState::Hedged | ExecutionRunState::Closed => DeterministicFlowStage::new(
            "ACK / 终态",
            "双腿终态已确认",
            DeterministicFlowState::Complete,
        ),
        ExecutionRunState::UnwindRequired | ExecutionRunState::Unwinding => {
            DeterministicFlowStage::new(
                "ACK / 终态",
                "存在裸腿，正在收口",
                DeterministicFlowState::Warning,
            )
        }
        ExecutionRunState::FailedSafe => {
            DeterministicFlowStage::new("ACK / 终态", "终态失败", DeterministicFlowState::Blocked)
        }
        _ => DeterministicFlowStage::new(
            "ACK / 终态",
            "等待双腿 ACK",
            DeterministicFlowState::Current,
        ),
    }
}

fn exit_stage(run: Option<&ExecutionRun>) -> DeterministicFlowStage {
    match run.map(|run| run.state) {
        Some(ExecutionRunState::Hedged) => DeterministicFlowStage::new(
            "保护退出",
            "保护策略监控中",
            DeterministicFlowState::Current,
        ),
        Some(ExecutionRunState::UnwindRequired) => {
            DeterministicFlowStage::new("保护退出", "需要补偿收口", DeterministicFlowState::Warning)
        }
        Some(ExecutionRunState::Unwinding) => {
            DeterministicFlowStage::new("保护退出", "双腿平仓中", DeterministicFlowState::Current)
        }
        Some(ExecutionRunState::Closed) => {
            DeterministicFlowStage::new("保护退出", "双腿已平仓", DeterministicFlowState::Complete)
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
            DeterministicFlowStage::new("复盘", "已写入终态记录", DeterministicFlowState::Complete)
        }
        Some(ExecutionRunState::FailedSafe) => {
            DeterministicFlowStage::new("复盘", "失败记录可复核", DeterministicFlowState::Warning)
        }
        _ => DeterministicFlowStage::new("复盘", "等待终态", DeterministicFlowState::Idle),
    }
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

fn current_time_ms() -> i64 {
    i64::try_from(crate::state::polling::now_ms()).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "deterministic_flow/tests.rs"]
mod tests;
