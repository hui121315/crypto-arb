use super::*;

#[test]
fn account_state_merges_child_evidence_and_marks_unknown_equity() {
    let balances = VenueBalanceEnvelope::new(
        vec![balance_row("mock")],
        ListStatus::Fresh,
        "account_balance_runtime",
        10,
        Vec::new(),
        vec![health_row("mock", "balance", VenueOperationStatus::Ok)],
    );
    let positions = VenuePositionEnvelope::new(
        vec![position_row("mock")],
        ListStatus::Fresh,
        "account_position_runtime",
        10,
        Vec::new(),
        vec![health_row("mock", "positions", VenueOperationStatus::Ok)],
    );
    let snapshot = snapshot_from_envelopes(balances, positions, 10);
    assert_eq!(snapshot.status, ListStatus::Degraded);
    assert_eq!(snapshot.source, ACCOUNT_STATE_SOURCE);
    assert_eq!(snapshot.balances.row_count, 1);
    assert_eq!(snapshot.positions.row_count, 1);
    assert!(snapshot
        .field_quality
        .iter()
        .any(|row| row.field == EQUITY_FIELD && row.status == AccountFieldQualityStatus::Unknown));
    assert!(snapshot
        .problems
        .iter()
        .any(|problem| problem.code == codes::ACCOUNT_FIELD_UNKNOWN));
}

#[test]
fn account_state_preserves_child_field_quality() {
    let balances = VenueBalanceEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        "account_balance_runtime",
        10,
        vec![ApiProblem::new(
            codes::BALANCE_EVIDENCE_MISSING,
            "missing balance",
        )],
        Vec::new(),
    )
    .with_field_quality(vec![AccountFieldQuality::new(
        AccountFieldSubject::balance("mock", "USDT"),
        "available",
        AccountFieldQualityStatus::Missing,
        "account_balance_runtime",
        Some(10),
    )]);
    let positions = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        "account_position_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );
    let snapshot = snapshot_from_envelopes(balances, positions, 10);
    assert_eq!(snapshot.status, ListStatus::Degraded);
    assert!(snapshot
        .field_quality
        .iter()
        .any(|row| row.field == "available" && row.status == AccountFieldQualityStatus::Missing));
    assert!(snapshot
        .problems
        .iter()
        .any(|problem| problem.code == codes::BALANCE_EVIDENCE_MISSING));
}

#[test]
fn account_state_merges_account_operation_health_rows() {
    let rows = vec![
        health_row(
            "hyperliquid:xyz",
            "credential_probe:open_orders_read",
            VenueOperationStatus::Ok,
        ),
        health_row(
            "hyperliquid:xyz",
            "credential_probe:open_orders_read",
            VenueOperationStatus::Ok,
        ),
        health_row(
            "hyperliquid:xyz",
            "credential_probe:account_mode_read",
            VenueOperationStatus::Ok,
        ),
        health_row(
            "hyperliquid:xyz",
            "credential_probe:order_permission",
            VenueOperationStatus::Warn,
        ),
        health_row(
            "hyperliquid:xyz",
            "private_ws_account_stream",
            VenueOperationStatus::Ok,
        ),
        health_row(
            "hyperliquid:xyz",
            "private_ws_order_stream",
            VenueOperationStatus::Ok,
        ),
        health_row("hyperliquid:xyz", "balance", VenueOperationStatus::Ok),
    ];
    let balances = VenueBalanceEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        "account_balance_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );
    let positions = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        "account_position_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );
    let snapshot = snapshot_from_parts(
        balances,
        positions,
        VenueOpenOrdersEnvelope::default(),
        &account_operation_health_from_rows(&rows),
        10,
    );
    let operations = snapshot
        .operation_health
        .iter()
        .map(|row| row.operation.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(snapshot.operation_health.len(), 5);
    assert!(operations.contains("credential_probe:open_orders_read"));
    assert!(operations.contains("credential_probe:account_mode_read"));
    assert!(operations.contains("credential_probe:order_permission"));
    assert!(operations.contains("private_ws_account_stream"));
    assert!(operations.contains("private_ws_order_stream"));
    assert!(!operations.contains("balance"));
}
