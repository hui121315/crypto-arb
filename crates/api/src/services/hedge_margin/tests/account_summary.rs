use super::*;

#[test]
fn margin_rows_prefer_verified_account_summary_available_facts() {
    let intent = order_intent("bybit", ExecutionMode::Live, false);
    let balances = vec![VenueBalanceInfo {
        venue: "bybit".into(),
        currency: "USDT".into(),
        total: 10.0,
        available: 1.0,
        frozen: 9.0,
        unrealized_pnl: 0.0,
    }];
    let summaries = vec![VenueAccountSummary {
        venue: "bybit".into(),
        account_type: "UNIFIED".into(),
        equity_scope: AccountEquityScope::Unified,
        total_equity_usd: 100.0,
        total_available_balance_usd: 80.0,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: 20.0,
        total_maintenance_margin_usd: 5.0,
        account_im_rate: 0.2,
        account_mm_rate: 0.05,
        source: "bybit wallet".into(),
        observed_at_ms: 1,
        freshness_ms: Some(0),
        problem: None,
    }];

    let rows = account_margin_rows(&balances, &[&intent], &summaries);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].currency, "USD");
    assert_eq!(rows[0].total, 100.0);
    assert_eq!(rows[0].available, 80.0);
    assert_eq!(rows[0].frozen, 20.0);

    let outcome = margin_outcome(
        HedgePreflightStatus::Passed,
        &[&intent],
        &rows,
        MarginBalanceEvidence {
            operation_health: vec![current_balance_operation_row("bybit")],
            account_summaries: summaries,
            collateral_rows: balances,
            ..MarginBalanceEvidence::default()
        },
        None,
    );

    assert!(outcome.field_quality.iter().any(|row| {
        row.field == "margin_currency"
            && row.subject == AccountFieldSubject::balance("bybit", "USD")
            && row.source == "bybit wallet"
            && row.status == AccountFieldQualityStatus::Actual
    }));
    assert!(outcome.field_quality.iter().any(|row| {
        row.field == "collateral_currency"
            && row.subject == AccountFieldSubject::balance("bybit", "USDT")
            && row.status == AccountFieldQualityStatus::Actual
    }));
    assert!(outcome.field_quality.iter().any(|row| {
        row.field == "account_equity_source"
            && row.subject == AccountFieldSubject::account("bybit")
            && row.source == "bybit wallet"
    }));
    assert!(outcome.row_health.iter().any(|row| {
        row.subject == AccountFieldSubject::balance("bybit", "USDT") && row.source == "test"
    }));
    assert!(outcome.row_health.iter().any(|row| {
        row.subject == AccountFieldSubject::account("bybit") && row.source == "bybit wallet"
    }));
}
