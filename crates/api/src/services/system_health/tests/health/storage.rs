use super::*;

#[test]
fn configured_audit_log_storage_failure_surfaces_runtime_problem() {
    let mut row = operation_health("system", "storage:audit_log", VenueOperationStatus::Blocked);
    row.problem = Some(shared_types::ApiProblem::new(
        "AUDIT_STORAGE_UNAVAILABLE",
        "Audit JSONL cannot be opened",
    ));

    let problems =
        operation_health_problems(&VenueOperationHealthSnapshot::new(vec![row], 10), true);

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, "audit_storage");
    assert_eq!(problems[0].operation, "storage:audit_log");
    assert_eq!(problems[0].code, "AUDIT_STORAGE_UNAVAILABLE");
    assert_eq!(problems[0].message, "Audit JSONL cannot be opened");
    assert_eq!(problems[0].venue.as_deref(), Some("system"));
}

#[test]
fn unconfigured_audit_log_storage_warn_stays_out_of_runtime_problems() {
    let mut row = operation_health("system", "storage:audit_log", VenueOperationStatus::Warn);
    row.configured = Some(false);

    let problems =
        operation_health_problems(&VenueOperationHealthSnapshot::new(vec![row], 10), true);

    assert!(problems.is_empty());
}

#[test]
fn operation_health_problems_ignore_unrecognized_and_unsupported_rows() {
    let rows = vec![
        operation_health(
            "okx",
            "credential_probe:made_up",
            VenueOperationStatus::Blocked,
        ),
        unsupported_operation_health("bybit", "private_ws_account_stream"),
        operation_health("gate", "storage:history", VenueOperationStatus::Blocked),
    ];

    let problems = operation_health_problems(&VenueOperationHealthSnapshot::new(rows, 10), true);

    assert!(problems.is_empty());
}
