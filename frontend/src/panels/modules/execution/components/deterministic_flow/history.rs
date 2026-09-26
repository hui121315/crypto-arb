use super::{exit_stage, finality_stage, review_stage, run_legs_filled, ExecutionFlowSummary};
use crate::panels::modules::execution::components::run_state_label;
use crate::panels::shared::{DeterministicFlowStage, DeterministicFlowState};
use shared_types::{ApiProblem, ExecutionRun, ExecutionRunState};

pub(super) fn summary(
    run: Option<&ExecutionRun>,
    problem: Option<&ApiProblem>,
) -> ExecutionFlowSummary {
    if let Some(problem) = problem {
        let previous = run
            .map(|run| format!("上次状态：{} · ", run_state_label(run)))
            .unwrap_or_default();
        return ExecutionFlowSummary {
            label: "历史状态待确认".into(),
            detail: format!("{previous}{}", problem.message),
            state: DeterministicFlowState::Warning,
        };
    }
    let Some(run) = run else {
        return ExecutionFlowSummary {
            label: "读取原执行记录".into(),
            detail: "等待匹配的交易记录，不创建新票据".into(),
            state: DeterministicFlowState::Current,
        };
    };
    let issue = run
        .finality_problem
        .as_ref()
        .or(run.unwind_problem.as_ref())
        .or(run.valuation_problem.as_ref());
    ExecutionFlowSummary {
        label: run_state_label(run),
        detail: format!(
            "只读 · {}",
            issue
                .map(|p| p.message.as_str())
                .unwrap_or(&run.status_reason)
        ),
        state: if issue.is_some() {
            DeterministicFlowState::Warning
        } else {
            match run.state {
                ExecutionRunState::FailedSafe | ExecutionRunState::UnwindRequired => {
                    DeterministicFlowState::Blocked
                }
                ExecutionRunState::FirstLegPartial | ExecutionRunState::Unwinding => {
                    DeterministicFlowState::Warning
                }
                ExecutionRunState::Hedged if !run_legs_filled(run) => {
                    DeterministicFlowState::Warning
                }
                ExecutionRunState::Hedged | ExecutionRunState::Closed => {
                    DeterministicFlowState::Complete
                }
                _ => DeterministicFlowState::Current,
            }
        },
    }
}

pub(super) fn stages(
    run: Option<&ExecutionRun>,
    problem: Option<&ApiProblem>,
) -> Vec<DeterministicFlowStage> {
    let Some(run) = run else {
        return ["原运行", "订单最终结果", "持仓 / 退出", "复盘"]
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                DeterministicFlowStage::new(
                    label,
                    if index == 0 {
                        problem
                            .map(|p| p.message.as_str())
                            .unwrap_or("读取指定记录")
                    } else {
                        "等待原运行记录"
                    },
                    if index != 0 {
                        DeterministicFlowState::Idle
                    } else if problem.is_some() {
                        DeterministicFlowState::Warning
                    } else {
                        DeterministicFlowState::Current
                    },
                )
            })
            .collect();
    };
    let state = summary(Some(run), problem);
    vec![
        DeterministicFlowStage::new(
            "原运行",
            format!("{} · {}", run.run_id, state.label),
            state.state,
        ),
        finality_stage(Some(run)),
        exit_stage(Some(run)),
        review_stage(Some(run)),
    ]
}
