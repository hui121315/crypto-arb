use super::*;

#[test]
fn bybit_unified_summary_drives_actual_account_field_quality() {
    let balances = VenueBalanceEnvelope::new(
        vec![balance_row("bybit")],
        ListStatus::Fresh,
        "account_balance_runtime",
        10,
        Vec::new(),
        Vec::new(),
    )
    .with_account_summaries(vec![bybit_account_summary(None)]);
    let positions = VenuePositionEnvelope::new(
        vec![position_row("bybit")],
        ListStatus::Fresh,
        "account_position_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );
    let open_orders = VenueOpenOrdersEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        "account_open_orders_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );

    let snapshot = snapshot_from_parts(balances, positions, open_orders, &[], 10);

    assert_eq!(snapshot.status, ListStatus::Fresh);
    let bybit_quality = snapshot
        .field_quality
        .iter()
        .filter(|row| row.subject.venue.as_deref() == Some("bybit"))
        .collect::<Vec<_>>();
    assert_eq!(bybit_quality.len(), 8);
    assert!(bybit_quality
        .iter()
        .filter(|row| row.field != "withdrawableBalance")
        .all(|row| {
            row.status == AccountFieldQualityStatus::Actual
                && row.source == "bybit.v5.wallet-balance"
                && row.problem.is_none()
        }));
    assert!(bybit_quality.iter().any(|row| {
        row.field == "withdrawableBalance"
            && row.status == AccountFieldQualityStatus::Missing
            && row.problem.is_some()
    }));
    assert!(!snapshot.field_quality.iter().any(|row| {
        row.subject.venue.as_deref() == Some("bybit")
            && row.field == EQUITY_FIELD
            && row.status == AccountFieldQualityStatus::Unknown
    }));
}

#[test]
fn bybit_unified_summary_problem_degrades_all_account_facts() {
    let problem = ApiProblem::new(
        "BYBIT_ACCOUNT_SUMMARY_INVALID",
        "wallet account metrics failed validation",
    )
    .with_source("bybit.v5.wallet-balance");
    let balances = VenueBalanceEnvelope::new(
        vec![balance_row("bybit")],
        ListStatus::Fresh,
        "account_balance_runtime",
        10,
        Vec::new(),
        Vec::new(),
    )
    .with_account_summaries(vec![bybit_account_summary(Some(problem))]);
    let positions = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        "account_position_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );
    let open_orders = VenueOpenOrdersEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        "account_open_orders_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );

    let snapshot = snapshot_from_parts(balances, positions, open_orders, &[], 10);

    assert_eq!(snapshot.status, ListStatus::Degraded);
    assert!(snapshot
        .problems
        .iter()
        .any(|problem| problem.code == "BYBIT_ACCOUNT_SUMMARY_INVALID"));
    let bybit_quality = snapshot
        .field_quality
        .iter()
        .filter(|row| row.subject.venue.as_deref() == Some("bybit"))
        .collect::<Vec<_>>();
    assert_eq!(bybit_quality.len(), 8);
    assert!(bybit_quality
        .iter()
        .all(|row| row.status != AccountFieldQualityStatus::Actual));
}

fn bybit_account_summary(problem: Option<ApiProblem>) -> VenueAccountSummary {
    VenueAccountSummary {
        venue: "bybit".to_owned(),
        account_type: "UNIFIED".to_owned(),
        equity_scope: shared_types::AccountEquityScope::Unified,
        total_equity_usd: 10_262.913_350_23,
        total_available_balance_usd: 9_556.605_655_5,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: 127.857_316_14,
        total_maintenance_margin_usd: 54.328_462_87,
        account_im_rate: 0.021,
        account_mm_rate: 0.009,
        source: "bybit.v5.wallet-balance".to_owned(),
        observed_at_ms: 10,
        freshness_ms: Some(0),
        problem,
    }
}
