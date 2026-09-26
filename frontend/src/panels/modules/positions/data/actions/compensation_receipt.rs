use crate::api::rest::ApiError;
use crate::panels::shared::operation_journal::PendingOperation;
use shared_types::{
    ActionEvidence, ActionRunKind, ActionState, ApiProblem, CloseLegStatus, CloseRun,
    CloseRunStatus, CloseRunUnwindPlanStatus, LiveOrderState, OrderRecord, OrderSource,
};

pub(super) fn mismatch() -> ApiError {
    ApiError::client(
        "REMEDY_RECEIPT_MISMATCH",
        "返回结果与原操作不一致，保留原请求，请先核对，不要重复提交",
    )
}

pub(super) fn evidence(attempt: &PendingOperation) -> ActionEvidence {
    let mut evidence = attempt
        .context
        .evidence()
        .with_action_kind(attempt.kind)
        .with_action_run_id(attempt.run_id.clone());
    if attempt.kind != ActionRunKind::TradingOrderCancel {
        evidence = evidence.with_run_id(Some(attempt.target.clone()));
    }
    evidence
}

pub(super) fn run_outcome(
    attempt: &PendingOperation,
    run: &CloseRun,
) -> Result<(ActionState, bool), ApiError> {
    if run.id != attempt.target {
        return Err(mismatch());
    }
    let id = attempt.run_id.as_deref().ok_or_else(mismatch)?;
    let plan = run.unwind_plan.as_ref().ok_or_else(mismatch)?;
    let (state, terminal) = if attempt.kind == ActionRunKind::PortfolioCloseManualTerminal {
        let record = plan
            .manual_terminal_evidence
            .as_ref()
            .ok_or_else(mismatch)?;
        if record.action_run_id.as_deref() != Some(id)
            || record.reason.trim().is_empty()
            || record.recorded_at_ms <= 0
            || record.snapshot_version.is_empty()
            || run.status != CloseRunStatus::ManuallyResolved
            || plan.status != CloseRunUnwindPlanStatus::ManualTerminalRecorded
        {
            return Err(mismatch());
        }
        (
            ActionState::succeeded("人工处理结果已记录；不代表系统已平仓或已盈利"),
            true,
        )
    } else if attempt.kind == ActionRunKind::PortfolioCloseCompensation {
        let mut matching = plan
            .compensation_attempts
            .iter()
            .filter(|a| a.action_run_id.as_deref() == Some(id));
        let leg = matching.next().ok_or_else(mismatch)?;
        if matching.next().is_some() {
            return Err(mismatch());
        }
        let order = leg.order.as_ref().ok_or_else(mismatch)?;
        if !valid_order(order)
            || order.intent.exchange != leg.venue
            || order.intent.symbol != leg.symbol
            || order.intent.side != leg.compensation_order_side
            || !leg.target_quantity.is_finite()
            || leg.target_quantity <= 0.0
        {
            return Err(mismatch());
        }
        let terminal = match leg.status {
            CloseLegStatus::Filled => {
                order.state == LiveOrderState::Filled
                    && order
                        .filled_quantity
                        .is_some_and(|qty| qty.is_finite() && qty + 1e-10 >= leg.target_quantity)
                    && leg.confirmed_filled_at_ms.is_some_and(|at| at > 0)
            }
            CloseLegStatus::Cancelled => order.state == LiveOrderState::Cancelled,
            CloseLegStatus::Rejected => order.state == LiveOrderState::Rejected,
            CloseLegStatus::Failed => order.state == LiveOrderState::Failed,
            _ => false,
        };
        let declared_terminal = matches!(
            leg.status,
            CloseLegStatus::Filled
                | CloseLegStatus::Cancelled
                | CloseLegStatus::Rejected
                | CloseLegStatus::Failed
        );
        if declared_terminal && !terminal {
            return Err(mismatch());
        }
        let state = match leg.status {
            CloseLegStatus::Filled => {
                ActionState::succeeded("原补救单已确认成交；整笔处理结果以平仓记录为准")
            }
            CloseLegStatus::Cancelled => {
                ActionState::succeeded("原补救单已确认撤销；剩余风险仍需处理")
            }
            CloseLegStatus::Rejected | CloseLegStatus::Failed => ActionState::failed(
                "原补救单未成交",
                leg.problem.clone().unwrap_or_else(|| {
                    ApiProblem::new("CLOSE_RUN_COMPENSATION_FAILED", "已取得原补救订单失败最终结果")
                }),
            ),
            _ => ActionState::accepted("补救单已受理，等待订单最终结果；可撤销已确认的补救单"),
        };
        (state, terminal)
    } else {
        return Err(mismatch());
    };
    Ok((state.with_evidence(evidence(attempt)), terminal))
}

fn valid_order(order: &OrderRecord) -> bool {
    order.intent.source == OrderSource::CloseRunCompensation
        && !order.intent.id.is_empty()
        && order.intent.quantity.is_finite()
        && order.intent.quantity > 0.0
}

pub(super) fn terminal_order(order: &OrderRecord) -> bool {
    matches!(
        order.state,
        LiveOrderState::Cancelled
            | LiveOrderState::Filled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}

pub(super) fn cancel_outcome(
    attempt: &PendingOperation,
    order: &OrderRecord,
) -> Result<ActionState, ApiError> {
    if attempt.kind != ActionRunKind::TradingOrderCancel
        || attempt.run_id.is_none()
        || order.intent.id != attempt.target
        || !valid_order(order)
    {
        return Err(mismatch());
    }
    let state = match order.state {
        LiveOrderState::Cancelled => {
            ActionState::succeeded("补救撤单：已确认取消；不代表整笔平仓已完成")
        }
        LiveOrderState::Filled => {
            ActionState::succeeded("补救撤单：订单已成交，未撤销；请核对剩余持仓")
        }
        LiveOrderState::Rejected | LiveOrderState::Failed => ActionState::failed(
            "补救订单已终止",
            ApiProblem::new(
                "COMPENSATION_ORDER_TERMINAL",
                "订单拒绝或失败，不代表撤单成功",
            ),
        ),
        LiveOrderState::PartiallyFilled => {
            ActionState::accepted("补救撤单：部分成交，等待剩余数量撤单最终结果")
        }
        _ => ActionState::accepted("补救撤单：撤单已提交，等待交易所最终结果"),
    };
    Ok(state.with_evidence(ActionEvidence::from_order_record(order).merged(evidence(attempt))))
}
