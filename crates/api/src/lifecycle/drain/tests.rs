use super::*;

#[test]
fn clean_report_accepts_drained_and_unbuffered_stores() {
    let report = ShutdownDrainReport::new(vec![
        clean_outcome(COMPONENT_BACKGROUND_TASKS, DrainStatus::Drained, "done"),
        clean_outcome(
            COMPONENT_HISTORY_STORE,
            DrainStatus::NoPendingBuffer,
            "sync",
        ),
    ]);

    assert!(report.ensure_clean().is_ok());
}

#[test]
fn failed_report_names_every_unclean_component() {
    let report = ShutdownDrainReport::new(vec![
        failed_outcome(COMPONENT_TRADING_SQL_JOURNAL, "ledger failed"),
        DrainOutcome {
            component: COMPONENT_AUDIT_LOG,
            status: DrainStatus::TimedOut,
            detail: "audit timeout".to_owned(),
        },
    ]);

    let error = report
        .ensure_clean()
        .err()
        .map_or_else(String::new, |error| error.to_string());
    assert!(error.contains("trading_sql_journal=failed"));
    assert!(error.contains("audit_log_jsonl=timed_out"));
}

#[test]
fn failed_producer_marks_synchronous_writes_unconfirmed() {
    let outcome = synchronous_store_outcome(COMPONENT_PORTFOLIO_NAV, false, "sync".into());

    assert_eq!(outcome.status, DrainStatus::TimedOut);
    assert!(outcome
        .detail
        .contains("in-flight synchronous write is unconfirmed"));
}
