use super::super::*;
use super::*;

#[test]
fn public_funding_schedule_enriches_live_and_paper_positions() {
    let mut rate = funding();
    rate.next_funding_time = 125_000;
    let orders = vec![dry_order(
        "btc-paper",
        "okx",
        "BTC-USDT-SWAP",
        OrderSide::Buy,
        false,
    )];
    let rows = rows_from_sources(
        vec![position("long")],
        &orders,
        &[rate],
        true,
        &[],
        RiskAnnotation {
            now_ms: 5_000,
            ..risk_annotation()
        },
    );

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.next_funding_ms == Some(125_000)));
    assert!(rows
        .iter()
        .any(|row| row.origin == PositionOrigin::AccountPrivate));
    assert!(rows
        .iter()
        .any(|row| row.origin == PositionOrigin::ExecutionLedger));
    assert!(rows
        .iter()
        .all(|row| row.seconds_until_funding == Some(120)));
    assert!(funding_field_quality(&rows, 42).is_empty());
}

#[test]
fn private_future_funding_schedule_wins_and_expired_schedule_falls_back() {
    let mut rate = funding();
    rate.next_funding_time = 125_000;
    let mut private = position("long");
    private.next_funding_ms = Some(130_000);
    let rows = rows_from_positions(vec![private], &[rate.clone()], 5_000, 15.0, 8.0);
    assert_eq!(rows[0].next_funding_ms, Some(130_000));

    let mut expired_private = position("long");
    expired_private.next_funding_ms = Some(4_000);
    let rows = rows_from_positions(vec![expired_private], &[rate], 5_000, 15.0, 8.0);
    assert_eq!(rows[0].next_funding_ms, Some(125_000));
}

#[test]
fn recently_elapsed_public_funding_schedule_reports_settling() {
    let mut rate = funding();
    rate.next_funding_time = 4_000;
    let rows = rows_from_positions(vec![position("long")], &[rate], 5_000, 15.0, 8.0);

    assert_eq!(rows[0].next_funding_ms, Some(4_000));
    assert_eq!(rows[0].seconds_until_funding, Some(0));
    assert!(funding_field_quality(&rows, 42).is_empty());
}

#[test]
fn funding_schedule_past_rollover_grace_remains_missing() {
    let mut rate = funding();
    rate.next_funding_time = 4_000;
    let rows = rows_from_positions(vec![position("long")], &[rate], 125_001, 15.0, 8.0);

    assert_eq!(rows[0].next_funding_ms, None);
    assert!(funding_field_quality(&rows, 42)
        .iter()
        .any(|quality| quality.field == "nextFundingMs"));
}

#[test]
fn binance_zero_liquidation_price_renders_as_non_numeric_placeholder() {
    let mut row = position("long");
    row.exchange = "binance".to_owned();
    row.liquidation_price = Some(0.0);

    let rows = rows_from_positions(vec![row], &[], 5_000, 15.0, 8.0);

    assert_eq!(rows[0].liquidation_price, None);
    assert_eq!(rows[0].liquidation_distance_pct, None);
    assert_eq!(rows[0].severity, PositionSeverity::Unknown);
}

#[test]
fn closed_history_does_not_corrupt_new_paper_position_entry_price() {
    let mut old_open = dry_order("old-open", "kucoin", "BARDUSDTM", OrderSide::Sell, false);
    old_open.filled_quantity = Some(4_666.0);
    old_open.filled_price = Some(0.12816);
    old_open.updated_at_ms = 10;

    let mut old_close = dry_order("old-close", "kucoin", "BARD", OrderSide::Buy, true);
    old_close.filled_quantity = Some(4_666.0);
    old_close.filled_price = Some(0.129155);
    old_close.updated_at_ms = 20;

    let mut new_open = dry_order("new-open", "kucoin", "BARD", OrderSide::Sell, false);
    new_open.filled_quantity = Some(61.0);
    new_open.filled_price = Some(0.12905);
    new_open.updated_at_ms = 30;

    let rows = rows_from_sources(
        Vec::new(),
        &[new_open, old_close, old_open],
        &[],
        true,
        &[],
        risk_annotation(),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].side, PositionSide::Short);
    assert_eq!(rows[0].quantity, 61.0);
    assert!((rows[0].entry_price - 0.12905).abs() < f64::EPSILON);
}

#[test]
fn partial_reduce_preserves_entry_and_reduce_only_cannot_flip_position() {
    let mut open = dry_order("open", "okx", "BTC", OrderSide::Buy, false);
    open.filled_quantity = Some(2.0);
    open.filled_price = Some(100.0);
    open.updated_at_ms = 10;
    let mut partial = dry_order("partial", "okx", "BTC", OrderSide::Sell, true);
    partial.filled_quantity = Some(1.0);
    partial.filled_price = Some(110.0);
    partial.updated_at_ms = 20;

    let rows = rows_from_sources(
        Vec::new(),
        &[partial.clone(), open.clone()],
        &[],
        true,
        &[],
        risk_annotation(),
    );
    assert_eq!(rows[0].quantity, 1.0);
    assert_eq!(rows[0].entry_price, 100.0);

    partial.filled_quantity = Some(3.0);
    let rows = rows_from_sources(
        Vec::new(),
        &[partial, open],
        &[],
        true,
        &[],
        risk_annotation(),
    );
    assert!(rows.is_empty());
}

#[test]
fn recent_close_runs_are_newest_first_and_bounded() {
    let rows = (0..10)
        .map(|idx| close_run_fixture(&format!("close-{idx}"), idx))
        .collect::<Vec<_>>();

    let recent = recent_close_runs_from_rows(rows);

    assert_eq!(recent.len(), RECENT_CLOSE_RUN_LIMIT);
    assert_eq!(recent[0].id, "close-9");
    assert_eq!(recent[7].id, "close-2");
}
