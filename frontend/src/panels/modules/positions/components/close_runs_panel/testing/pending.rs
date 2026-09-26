use super::*;

#[test]
fn pending_compensation_without_actions_keeps_reconciliation_visible() {
    let mut run = close_run("close-pending", CloseRunStatus::CompensationSubmitted, 1);
    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.status = CloseRunUnwindPlanStatus::CompensationSubmitted;
        plan.next_actions.clear();
        plan.remaining_positions.clear();
    }

    assert_eq!(close_run_next_action_detail(&run), "等待后端最终结果对账");
    assert_eq!(
        close_run_remaining_positions_detail(&run),
        "当前数据没有未对冲仓位，剩余订单的处理结果仍待确认"
    );
}
