fn assert_restored_order_snapshot_health(journal: &OrderJournal) {
    let snapshot = journal.order_snapshot_storage_snapshot();
    assert_eq!(snapshot.record_count, 1);
    assert!(snapshot.replayed_records >= 1);
    assert_eq!(snapshot.replay_failures, 0);
    assert_eq!(
        journal.execution_ledger_storage_snapshot().replay_failures,
        0
    );
}
