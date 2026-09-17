use super::*;

#[test]
fn close_run_keeps_partial_outcome_and_naked_exposure() {
    let submitted = close_leg_for_test(CloseLegStatus::Submitted, 200.0);
    let failed = close_leg_for_test(CloseLegStatus::Failed, 125.0);

    let run = close_run(
        CloseRunScope::Pair,
        vec![submitted, failed],
        1,
        close_context_for_test(2),
    );

    assert_eq!(run.status, CloseRunStatus::PartiallySubmitted);
    assert_eq!(run.submitted_order_count, 1);
    assert_eq!(run.failed_leg_count, 1);
    assert_eq!(run.naked_exposure_usd, 125.0);
    assert_eq!(
        run.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(shared_types::problem::codes::CLOSE_RUN_PARTIAL)
    );
    let details = run
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref());
    assert!(details.is_some());
    let details = details.unwrap_or(&serde_json::Value::Null);
    assert_eq!(
        details
            .get("failedLegs")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("notionalUsd"))
            .and_then(|value| value.as_f64()),
        Some(125.0)
    );
    assert_eq!(
        details
            .get("autoUnwindStatus")
            .and_then(|value| value.as_str()),
        Some("blocked_pending_manual_recheck")
    );
}

#[test]
fn close_run_marks_all_submitted_without_claiming_final_success() {
    let first = close_leg_for_test(CloseLegStatus::Submitted, 200.0);
    let second = close_leg_for_test(CloseLegStatus::Submitted, 125.0);

    let run = close_run(
        CloseRunScope::Pair,
        vec![first, second],
        1,
        close_context_for_test(2),
    );

    assert_eq!(run.status, CloseRunStatus::Submitted);
    assert_eq!(run.submitted_order_count, 2);
    assert_eq!(run.failed_leg_count, 0);
    assert_eq!(run.naked_exposure_usd, 0.0);
    assert!(run.message.contains("等待交易所成交终态"));
    assert!(run.problem.is_none());
}

#[test]
fn close_run_marks_all_failed_without_dropping_leg_problems() {
    let first = close_leg_for_test(CloseLegStatus::Failed, 200.0);
    let second = close_leg_for_test(CloseLegStatus::Failed, 125.0);

    let run = close_run(
        CloseRunScope::Pair,
        vec![first, second],
        1,
        close_context_for_test(2),
    );

    assert_eq!(run.status, CloseRunStatus::Failed);
    assert_eq!(run.submitted_order_count, 0);
    assert_eq!(run.failed_leg_count, 2);
    assert_eq!(run.legs.len(), 2);
    assert_eq!(run.naked_exposure_usd, 325.0);
}
