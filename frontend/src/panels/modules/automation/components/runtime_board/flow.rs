use crate::panels::shared::{
    deterministic_flow_rail, webhook_flow_stage, DeterministicFlowStage, DeterministicFlowState,
};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    AutoProfitCloseConfig, AutomationDecision, AutomationDecisionKind, AutomationRuntimeState,
    AutomationRuntimeStatus, DeterministicExecutionArtifact, ExecutionArtifactStatus,
    WebhookEventKind, WebhookRuntimeStatus,
};

use super::super::super::protection_calibration::protection_capital_ready;

pub(super) fn automation_flow(
    state: &LoadState<AutomationRuntimeStatus>,
    webhook: &LoadState<WebhookRuntimeStatus>,
    protection: &LoadState<AutoProfitCloseConfig>,
    receipt: &LoadState<shared_types::AutomationExecutionReceipt>,
) -> impl IntoView {
    let confirmed = matches!(state, LoadState::Ready(_));
    let status = state.value();
    let latest = status.and_then(current_decision);
    let recorded_run = receipt.value().map(|receipt| &receipt.run);
    let artifact = recorded_run.map_or_else(
        || status.and_then(current_artifact),
        |run| {
            status.and_then(|status| {
                status
                    .last_decision
                    .iter()
                    .chain(status.recent_decisions.iter())
                    .filter_map(|decision| decision.execution_artifact.as_ref())
                    .find(|artifact| {
                        artifact.ticket_id == run.ticket_id
                            && artifact.opportunity_id == run.opportunity_id
                    })
            })
        },
    );
    let qualification = qualification_stage(status, latest);
    let artifact_stage = artifact.map_or_else(
        || DeterministicFlowStage::new("工件重验", "等待 READY 工件", DeterministicFlowState::Idle),
        |artifact| {
            let status = effective_artifact_status(artifact);
            DeterministicFlowStage::new(
                "工件重验",
                artifact_status_label(status),
                artifact_flow_state(status),
            )
        },
    );
    let (submission, finality) = submission_stages(status, latest);
    let exit_state = exit_stage(status, protection);
    let mut summary = flow_summary(status, latest);
    let mut stages = vec![
        qualification,
        current_webhook_stage(webhook, artifact),
        artifact_stage,
        submission,
        finality,
        exit_state,
        DeterministicFlowStage::new("复盘", "等待平仓终态", DeterministicFlowState::Idle),
    ];
    if let Some(run) = recorded_run {
        // A control event must not erase a recorded execution or mix in another ticket.
        summary.detail = format!("{} · {}", summary.label, run.run_id);
        summary.label = "运行回执闭环".to_owned();
        stages[0] = DeterministicFlowStage::new(
            "原机会",
            &run.opportunity_id,
            DeterministicFlowState::Idle,
        );
        stages[2] = DeterministicFlowStage::new(
            "运行票据",
            "原票据只读，不可复用",
            DeterministicFlowState::Idle,
        );
        stages[3] = recorded_submission_stage(run.state);
        if artifact.is_none() {
            stages[1] = DeterministicFlowStage::new(
                "Webhook",
                "暂无该运行投递证据",
                DeterministicFlowState::Idle,
            );
        }
    }
    apply_receipt_stages(&mut stages, receipt);
    if !confirmed {
        stages = [
            "机会",
            "Webhook",
            "工件重验",
            "提交",
            "终态",
            "退出保护",
            "复盘",
        ]
        .into_iter()
        .map(|label| {
            DeterministicFlowStage::new(label, "运行态待确认", DeterministicFlowState::Warning)
        })
        .collect();
    }
    let flow = deterministic_flow_rail("自动化确定性闭环", stages);
    view! {
        <section class="automation-flow-panel" data-tone=if confirmed { summary.tone } else { "warning" }>
            <header>
                <div>
                    <span>"七阶段执行证据"</span>
                    <strong>{if confirmed { summary.label } else { "运行态待确认".into() }}</strong>
                </div>
                <small>{if confirmed { summary.detail } else { "保留上次记录，等待后台重新确认".into() }}</small>
            </header>
            {flow}
        </section>
    }
}

fn apply_receipt_stages(
    stages: &mut [DeterministicFlowStage],
    state: &LoadState<shared_types::AutomationExecutionReceipt>,
) {
    use super::super::receipts::{exit_confirmed, leg_confirmed};
    use shared_types::ExecutionRunState;
    let Some(receipt) = state.value() else {
        return;
    };
    if !matches!(state, LoadState::Ready(_)) {
        for stage in &mut stages[3..] {
            stage.detail = "回执待确认".into();
            stage.state = DeterministicFlowState::Warning;
        }
        return;
    }
    let run = &receipt.run;
    let paper = receipt.mode == Some(shared_types::ExecutionMode::DryRun);
    let filled = leg_confirmed(&run.long_leg, paper) && leg_confirmed(&run.short_leg, paper);
    stages[4] = if filled {
        DeterministicFlowStage::new(
            "ACK / 终态",
            if paper {
                "模拟双腿成交已确认"
            } else {
                "双腿成交已确认"
            },
            DeterministicFlowState::Complete,
        )
    } else if matches!(
        run.state,
        ExecutionRunState::FailedSafe
            | ExecutionRunState::UnwindRequired
            | ExecutionRunState::Unwinding
    ) {
        DeterministicFlowStage::new(
            "ACK / 终态",
            "执行异常，查看逐腿回执",
            DeterministicFlowState::Blocked,
        )
    } else {
        DeterministicFlowStage::new(
            "ACK / 终态",
            "等待双腿成交证据",
            DeterministicFlowState::Current,
        )
    };
    if exit_confirmed(receipt) {
        stages[5] = DeterministicFlowStage::new(
            "保护退出",
            if paper {
                "模拟双腿平仓已确认"
            } else {
                "双腿平仓已确认"
            },
            DeterministicFlowState::Complete,
        );
        stages[6] = DeterministicFlowStage::new(
            "复盘",
            "回执可复盘，净收益待核算",
            DeterministicFlowState::Current,
        );
    } else if !receipt.close_runs.is_empty() {
        stages[5] = DeterministicFlowStage::new(
            "保护退出",
            "退出未闭环，查看平仓回执",
            DeterministicFlowState::Warning,
        );
    }
}

fn recorded_submission_stage(state: shared_types::ExecutionRunState) -> DeterministicFlowStage {
    use shared_types::ExecutionRunState;
    let (detail, state) = match state {
        ExecutionRunState::Previewed | ExecutionRunState::RiskChecked => {
            ("运行已创建，尚未提交", DeterministicFlowState::Idle)
        }
        ExecutionRunState::SubmittingFirstLeg
        | ExecutionRunState::FirstLegPartial
        | ExecutionRunState::SubmittingSecondLeg
        | ExecutionRunState::SecondLegSubmitted => {
            ("已发起，等待逐腿回执", DeterministicFlowState::Current)
        }
        ExecutionRunState::Hedged | ExecutionRunState::Closed => {
            ("提交已记录", DeterministicFlowState::Complete)
        }
        ExecutionRunState::FailedSafe
        | ExecutionRunState::UnwindRequired
        | ExecutionRunState::Unwinding => {
            ("执行异常，查看运行回执", DeterministicFlowState::Blocked)
        }
    };
    DeterministicFlowStage::new("双腿提交", detail, state)
}

pub(super) fn current_decision(status: &AutomationRuntimeStatus) -> Option<&AutomationDecision> {
    let has_current_cycle =
        status.active_run_count > 0 || (status.config.enabled && !status.config.paused);
    if has_current_cycle {
        status.last_decision.as_ref()
    } else {
        None
    }
}

pub(super) fn current_artifact(
    status: &AutomationRuntimeStatus,
) -> Option<&DeterministicExecutionArtifact> {
    let latest = current_decision(status)?;
    if let Some(artifact) = latest.execution_artifact.as_ref() {
        return Some(artifact);
    }

    let anchor = latest.opportunity_id.as_deref().or_else(|| {
        if status.active_run_count == 0 {
            return None;
        }
        status.recent_decisions.iter().find_map(|decision| {
            matches!(
                decision.kind,
                AutomationDecisionKind::Submitted | AutomationDecisionKind::Replayed
            )
            .then_some(decision.opportunity_id.as_deref())
            .flatten()
        })
    })?;

    status.recent_decisions.iter().find_map(|decision| {
        (decision.opportunity_id.as_deref() == Some(anchor))
            .then_some(decision.execution_artifact.as_ref())
            .flatten()
    })
}

fn current_webhook_stage(
    state: &LoadState<WebhookRuntimeStatus>,
    artifact: Option<&DeterministicExecutionArtifact>,
) -> DeterministicFlowStage {
    if !matches!(state, LoadState::Ready(_)) {
        return webhook_flow_stage(state, WebhookEventKind::Opportunity);
    }
    let Some(status) = state.value() else {
        return webhook_flow_stage(state, WebhookEventKind::Opportunity);
    };
    if !status.config.enabled
        || !status.config.url_configured
        || !status
            .config
            .event_kinds
            .contains(&WebhookEventKind::Opportunity)
    {
        return webhook_readiness_stage(state);
    }
    let Some(artifact) = artifact else {
        return webhook_readiness_stage(state);
    };
    let expected_event_id = format!("opportunity-{}", artifact.artifact_id);
    let Some(delivery) = status
        .recent_deliveries
        .iter()
        .find(|delivery| delivery.event_id == expected_event_id)
        .cloned()
    else {
        return DeterministicFlowStage::new(
            "Webhook",
            "等待当前机会投递",
            DeterministicFlowState::Current,
        );
    };
    let mut current = status.clone();
    current.queue_depth = 0;
    current.recent_deliveries = vec![delivery];
    webhook_flow_stage(&LoadState::Ready(current), WebhookEventKind::Opportunity)
}

fn webhook_readiness_stage(state: &LoadState<WebhookRuntimeStatus>) -> DeterministicFlowStage {
    let Some(status) = state.value() else {
        return DeterministicFlowStage::new(
            "Webhook",
            "读取投递状态",
            DeterministicFlowState::Current,
        );
    };
    if !status.config.enabled || !status.config.url_configured {
        return DeterministicFlowStage::new("Webhook", "未启用", DeterministicFlowState::Warning);
    }
    if !status
        .config
        .event_kinds
        .contains(&WebhookEventKind::Opportunity)
    {
        return DeterministicFlowStage::new(
            "Webhook",
            "未订阅机会",
            DeterministicFlowState::Warning,
        );
    }
    DeterministicFlowStage::new("Webhook", "通道已就绪", DeterministicFlowState::Idle)
}

struct AutomationFlowSummary {
    label: String,
    detail: String,
    tone: &'static str,
}

fn flow_summary(
    status: Option<&AutomationRuntimeStatus>,
    latest: Option<&AutomationDecision>,
) -> AutomationFlowSummary {
    let Some(status) = status else {
        return AutomationFlowSummary {
            label: "读取运行态".into(),
            detail: "正在连接自动化服务".into(),
            tone: "idle",
        };
    };
    if status.state == AutomationRuntimeState::Error
        || latest.is_some_and(|decision| decision.kind == AutomationDecisionKind::Failed)
    {
        return AutomationFlowSummary {
            label: "自动化阻断".into(),
            detail: "查看最近决策与完整流程".into(),
            tone: "blocked",
        };
    }
    if !status.config.enabled {
        return AutomationFlowSummary {
            label: "自动化已关闭".into(),
            detail: "启动后才会监控和提交".into(),
            tone: "idle",
        };
    }
    if status.config.paused {
        return AutomationFlowSummary {
            label: "自动化已暂停".into(),
            detail: "恢复后继续监控候选".into(),
            tone: "warning",
        };
    }
    if status.active_run_count > 0 {
        return AutomationFlowSummary {
            label: format!("正在处理 {} 条运行单", status.active_run_count),
            detail: "提交、终态与退出保护持续更新".into(),
            tone: "active",
        };
    }
    if status.state == AutomationRuntimeState::Submitting {
        return AutomationFlowSummary {
            label: "双腿提交中".into(),
            detail: "等待 ACK 与订单终态".into(),
            tone: "active",
        };
    }
    AutomationFlowSummary {
        label: "正在监控候选".into(),
        detail: latest
            .and_then(|decision| decision.symbol.clone())
            .map_or_else(
                || "等待通过全部门槛的机会".into(),
                |symbol| format!("最近判断 {symbol}"),
            ),
        tone: "active",
    }
}

fn qualification_stage(
    status: Option<&AutomationRuntimeStatus>,
    latest: Option<&AutomationDecision>,
) -> DeterministicFlowStage {
    match (status, latest) {
        (None, _) => {
            DeterministicFlowStage::new("资格判定", "读取运行态", DeterministicFlowState::Current)
        }
        (Some(status), _) if !status.config.enabled => {
            DeterministicFlowStage::new("资格判定", "自动化已关闭", DeterministicFlowState::Idle)
        }
        (Some(status), _) if status.config.paused => {
            DeterministicFlowStage::new("资格判定", "循环已暂停", DeterministicFlowState::Warning)
        }
        (_, Some(decision))
            if matches!(
                decision.kind,
                AutomationDecisionKind::OpportunityQualified
                    | AutomationDecisionKind::Submitted
                    | AutomationDecisionKind::Replayed
            ) =>
        {
            DeterministicFlowStage::new(
                "资格判定",
                decision
                    .symbol
                    .clone()
                    .unwrap_or_else(|| "机会已通过".to_owned()),
                DeterministicFlowState::Complete,
            )
        }
        (_, Some(decision)) if decision.kind == AutomationDecisionKind::PreviewBlocked => {
            DeterministicFlowStage::new("资格判定", "预检阻断", DeterministicFlowState::Blocked)
        }
        _ => DeterministicFlowStage::new("资格判定", "监控候选", DeterministicFlowState::Current),
    }
}

fn submission_stages(
    status: Option<&AutomationRuntimeStatus>,
    latest: Option<&AutomationDecision>,
) -> (DeterministicFlowStage, DeterministicFlowStage) {
    if latest.is_some_and(|decision| decision.kind == AutomationDecisionKind::Failed)
        || status.is_some_and(|status| status.state == AutomationRuntimeState::Error)
    {
        return (
            DeterministicFlowStage::new("双腿提交", "提交失败", DeterministicFlowState::Blocked),
            DeterministicFlowStage::new("ACK / 终态", "未达终态", DeterministicFlowState::Blocked),
        );
    }
    if latest.is_some_and(|decision| {
        matches!(
            decision.kind,
            AutomationDecisionKind::Submitted | AutomationDecisionKind::Replayed
        )
    }) {
        let replayed =
            latest.is_some_and(|decision| decision.kind == AutomationDecisionKind::Replayed);
        let run = latest
            .and_then(|decision| decision.execution_run_id.as_deref())
            .unwrap_or("运行单已创建");
        return (
            DeterministicFlowStage::new(
                "双腿提交",
                if replayed {
                    "幂等重放"
                } else {
                    "提交已受理"
                },
                DeterministicFlowState::Complete,
            ),
            DeterministicFlowStage::new(
                "ACK / 终态",
                format!("成交待核对 · {run}"),
                DeterministicFlowState::Current,
            ),
        );
    }
    if status.is_some_and(|status| status.state == AutomationRuntimeState::Submitting) {
        return (
            DeterministicFlowStage::new("双腿提交", "提交中", DeterministicFlowState::Current),
            DeterministicFlowStage::new("ACK / 终态", "等待双腿", DeterministicFlowState::Current),
        );
    }
    (
        DeterministicFlowStage::new("双腿提交", "尚未提交", DeterministicFlowState::Idle),
        DeterministicFlowStage::new("ACK / 终态", "等待运行单", DeterministicFlowState::Idle),
    )
}

fn exit_stage(
    status: Option<&AutomationRuntimeStatus>,
    protection: &LoadState<AutoProfitCloseConfig>,
) -> DeterministicFlowStage {
    if status.is_some_and(|status| status.active_run_count > 0) {
        let ready = protection_ready(status, protection);
        let detail = if ready {
            "保护已配置，退出待核对"
        } else {
            "退出保护未知"
        };
        DeterministicFlowStage::new(
            "保护退出",
            detail,
            if ready {
                DeterministicFlowState::Current
            } else {
                DeterministicFlowState::Warning
            },
        )
    } else {
        DeterministicFlowStage::new("保护退出", "等待配对持仓", DeterministicFlowState::Idle)
    }
}

fn artifact_flow_state(status: ExecutionArtifactStatus) -> DeterministicFlowState {
    match status {
        ExecutionArtifactStatus::Ready => DeterministicFlowState::Complete,
        ExecutionArtifactStatus::Expired | ExecutionArtifactStatus::Unknown => {
            DeterministicFlowState::Warning
        }
        ExecutionArtifactStatus::Blocked
        | ExecutionArtifactStatus::Missing
        | ExecutionArtifactStatus::Tampered => DeterministicFlowState::Blocked,
    }
}

fn protection_ready(
    status: Option<&AutomationRuntimeStatus>,
    state: &LoadState<AutoProfitCloseConfig>,
) -> bool {
    matches!(state, LoadState::Ready(_))
        && status.zip(state.value()).is_some_and(|(status, config)| {
            protection_capital_ready(config, status.config.capital_usd)
        })
}

pub(super) fn effective_artifact_status(
    artifact: &DeterministicExecutionArtifact,
) -> ExecutionArtifactStatus {
    if current_time_ms() >= artifact.expires_at_ms {
        ExecutionArtifactStatus::Expired
    } else {
        artifact.status
    }
}

pub(super) fn current_time_ms() -> i64 {
    i64::try_from(crate::state::polling::now_ms()).unwrap_or(i64::MAX)
}

pub(super) const fn artifact_status_label(status: ExecutionArtifactStatus) -> &'static str {
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
mod tests {
    use super::*;

    #[test]
    fn accepted_submission_is_not_proof_of_leg_finality() {
        let mut decision = AutomationDecision {
            id: "test-submit".into(),
            kind: AutomationDecisionKind::Submitted,
            opportunity_id: None,
            symbol: None,
            reason: "accepted".into(),
            execution_run_id: Some("test-run".into()),
            problem: None,
            execution_artifact: None,
            occurred_at_ms: 1,
        };
        for kind in [
            AutomationDecisionKind::Submitted,
            AutomationDecisionKind::Replayed,
        ] {
            decision.kind = kind;
            let (submission, finality) = submission_stages(None, Some(&decision));
            assert_eq!(submission.state, DeterministicFlowState::Complete);
            assert_eq!(finality.state, DeterministicFlowState::Current);
            assert!(finality.detail.contains("test-run"));
        }
    }

    #[test]
    fn stale_exit_protection_cannot_claim_monitoring() {
        let mut status = AutomationRuntimeStatus::default();
        status.active_run_count = 1;
        let protection = LoadState::Stale {
            value: AutoProfitCloseConfig::default(),
            problem: shared_types::ApiProblem::new("TIMEOUT", "stale"),
        };
        assert!(!protection_ready(Some(&status), &protection));
        assert_eq!(
            exit_stage(Some(&status), &protection).state,
            DeterministicFlowState::Warning
        );
    }
}
