use super::*;

#[test]
fn margin_outcome_marks_unrelated_asset_as_non_margin_collateral() {
    let intent = order_intent("binance", ExecutionMode::Live, false);
    let balances = vec![VenueBalanceInfo {
        venue: "binance".into(),
        currency: "BTC".into(),
        total: 10.0,
        available: 10.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }];

    let outcome = margin_outcome(
        HedgePreflightStatus::Blocked,
        &[&intent],
        &balances,
        MarginBalanceEvidence {
            collateral_rows: balances.clone(),
            operation_health: vec![current_balance_operation_row("binance")],
            ..MarginBalanceEvidence::default()
        },
        Some("unrelated collateral".into()),
    );

    assert!(outcome.field_quality.iter().any(|row| {
        row.field == "margin_currency"
            && row.subject == AccountFieldSubject::account("binance")
            && row.status == AccountFieldQualityStatus::Missing
    }));
    assert!(outcome.field_quality.iter().any(|row| {
        row.field == "collateral_currency"
            && row.subject == AccountFieldSubject::balance("binance", "BTC")
            && row.status == AccountFieldQualityStatus::Actual
    }));
    assert!(outcome.row_health.iter().any(|row| {
        row.subject == AccountFieldSubject::balance("binance", "BTC") && row.source == "test"
    }));
}
