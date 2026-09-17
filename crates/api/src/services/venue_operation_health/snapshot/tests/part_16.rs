#[test]
fn task_registry_rows_expose_enabled_lag_and_retry_contract() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register_disabled("ledger_projection_jobs", 250);
    registry.register("funding", 5_000);
    let observed_at_ms = common::time::now_ms();
    registry.mark_retry_scheduled("funding", "rate limited", observed_at_ms + 5_000);

    let rows = task_registry_rows(&registry, observed_at_ms + 1_000);

    assert_task_contract_summary(task_health_row(&rows, OP_BACKGROUND_TASKS));
    assert_disabled_task_contract(task_health_row(
        &rows,
        "background_task:ledger_projection_jobs",
    ));
    assert_retrying_task_contract(task_health_row(&rows, "background_task:funding"));
}

fn task_health_row<'a>(
    rows: &'a [VenueOperationHealth],
    operation: &str,
) -> &'a VenueOperationHealth {
    rows.iter()
        .find(|row| row.operation == operation)
        .expect("task health row")
}

fn assert_task_contract_summary(summary: &VenueOperationHealth) {
    assert_eq!(summary.status, VenueOperationStatus::Blocked);
    assert_eq!(summary.configured, Some(true));
    assert_eq!(summary.requested, Some(1));
    assert!(summary.message.contains("disabled 1"));
}

fn assert_disabled_task_contract(disabled: &VenueOperationHealth) {
    assert_eq!(disabled.status, VenueOperationStatus::Unknown);
    assert_eq!(disabled.configured, Some(false));
    assert_eq!(disabled.requested, Some(0));
    assert_eq!(disabled.freshness_ms, None);
    assert!(disabled.problem.is_none());
}

fn assert_retrying_task_contract(funding: &VenueOperationHealth) {
    assert_eq!(funding.configured, Some(true));
    assert_eq!(funding.retry_after_ms, Some(4_000));
    assert!(funding.freshness_ms.is_some_and(|lag_ms| lag_ms >= 1_000));
    let evidence = funding.evidence.as_ref().expect("task evidence");
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "enabled=true"));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "retry_after_ms=4000"));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item.starts_with("lag_ms=")));
}
