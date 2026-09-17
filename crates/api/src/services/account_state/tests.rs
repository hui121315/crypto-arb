#![allow(clippy::panic)]
use super::*;

mod bybit_summary;
mod hyperliquid_equity;
mod reconciliation;
mod support;

use support::{balance_row, health_row, open_order_row, position_row};

#[test]
fn account_state_status_ignores_non_data_credential_attention() {
    let row = health_row(
        "hyperliquid:xyz",
        "credential_probe:order_permission",
        VenueOperationStatus::Warn,
    );
    assert_eq!(
        account_state_status(
            ListStatus::Fresh,
            ListStatus::Fresh,
            ListStatus::Fresh,
            &[],
            &[row],
            &[],
        ),
        ListStatus::Fresh
    );
}

#[test]
fn account_state_status_ignores_private_order_stream_attention() {
    let row = health_row(
        "hyperliquid:xyz",
        "private_ws_order_stream",
        VenueOperationStatus::Warn,
    );
    assert_eq!(
        account_state_status(
            ListStatus::Fresh,
            ListStatus::Fresh,
            ListStatus::Fresh,
            &[],
            &[row],
            &[],
        ),
        ListStatus::Fresh
    );
}

#[test]
fn account_state_status_degrades_on_current_position_attention() {
    let row = health_row("binance", "positions", VenueOperationStatus::Warn);

    assert_eq!(
        account_state_status(
            ListStatus::Fresh,
            ListStatus::Fresh,
            ListStatus::Fresh,
            &[],
            &[row],
            &[],
        ),
        ListStatus::Degraded
    );
}

#[test]
fn account_state_ignores_unconfigured_operation_attention() {
    let mut row = health_row(
        "gate",
        "private_ws_order_stream",
        VenueOperationStatus::Unknown,
    );
    row.configured = Some(false);

    assert_eq!(
        account_state_status(
            ListStatus::Fresh,
            ListStatus::Fresh,
            ListStatus::Fresh,
            &[],
            &[row],
            &[],
        ),
        ListStatus::Fresh
    );
}

#[test]
fn unconfigured_operation_does_not_invent_unknown_equity() {
    let mut row = health_row("gate", "balance", VenueOperationStatus::Blocked);
    row.configured = Some(false);
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
    let open_orders = VenueOpenOrdersEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        "account_open_orders_runtime",
        10,
        Vec::new(),
        Vec::new(),
    );

    assert!(
        account_equity_unknown_quality(&balances, &positions, &open_orders, &[row], &[], 10,)
            .is_empty()
    );
}

#[test]
fn account_state_exposes_kucoin_classic_futures_read_scope() {
    let rows = vec![health_row(
        "KuCoin",
        "credential_probe:account_mode_read",
        VenueOperationStatus::Ok,
    )];
    let quality = account_operation_field_quality(&account_operation_health_from_rows(&rows), 10);

    assert_eq!(quality.len(), 1);
    assert_eq!(quality[0].subject.venue.as_deref(), Some("KuCoin"));
    assert_eq!(quality[0].field, "classicFuturesPrivateReadScope");
    assert_eq!(quality[0].status, AccountFieldQualityStatus::Estimated);
    assert!(quality[0]
        .problem
        .as_ref()
        .is_some_and(|problem| problem.message.contains("Classic Futures")));
    assert_eq!(
        account_state_status(
            ListStatus::Fresh,
            ListStatus::Fresh,
            ListStatus::Fresh,
            &[],
            &[],
            &quality,
        ),
        ListStatus::Fresh
    );
}

#[test]
fn account_state_status_still_degrades_on_core_field_fallback() {
    let quality = [AccountFieldQuality::new(
        AccountFieldSubject::position("binance", "SOL", "long"),
        "markPrice",
        AccountFieldQualityStatus::Estimated,
        "position_entry_price_fallback",
        Some(10),
    )];

    assert_eq!(
        account_state_status(
            ListStatus::Fresh,
            ListStatus::Fresh,
            ListStatus::Fresh,
            &[],
            &[],
            &quality,
        ),
        ListStatus::Degraded
    );
}

#[test]
fn unconfigured_kucoin_does_not_claim_classic_futures_read_scope() {
    let mut row = health_row(
        "KuCoin",
        "credential_probe:account_mode_read",
        VenueOperationStatus::Blocked,
    );
    row.configured = Some(false);

    assert!(account_operation_field_quality(&[row], 10).is_empty());
}

#[test]
fn account_state_includes_open_order_rows_and_degrades() {
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
    let open_orders = VenueOpenOrdersEnvelope::new(
        vec![open_order_row("mock")],
        ListStatus::Degraded,
        "account_open_orders_runtime",
        10,
        vec![ApiProblem::new(
            codes::OPEN_ORDER_EVIDENCE_MISSING,
            "missing open order evidence",
        )],
        Vec::new(),
    );
    let snapshot = snapshot_from_parts(balances, positions, open_orders, &[], 10);
    assert_eq!(snapshot.status, ListStatus::Degraded);
    assert_eq!(snapshot.open_orders.row_count, 1);
    assert!(snapshot
        .problems
        .iter()
        .any(|problem| problem.code == codes::OPEN_ORDER_EVIDENCE_MISSING));
}

#[test]
fn account_scope_binding_applies_to_open_order_field_subjects() {
    let binding = AccountBindingEvidence {
        venue: "bybit".to_owned(),
        account_scope: Some("unified".to_owned()),
        status: shared_types::AccountBindingStatus::Verified,
        source: "credential_probe:account_mode_read".to_owned(),
        checked_at_ms: Some(10),
        freshness_ms: Some(0),
        credential_fingerprint: Some("fingerprint".to_owned()),
        problem: None,
    };
    let open_orders = VenueOpenOrdersEnvelope::new(
        vec![open_order_row("bybit")],
        ListStatus::Fresh,
        "account_open_orders_runtime",
        10,
        Vec::new(),
        Vec::new(),
    )
    .with_field_quality(vec![AccountFieldQuality::new(
        AccountFieldSubject::open_order("bybit", "order-1", "BTC", "buy"),
        "quantity",
        AccountFieldQualityStatus::Actual,
        "account_open_orders_runtime",
        Some(10),
    )])
    .with_account_bindings(vec![binding]);

    let snapshot = snapshot_from_parts(
        VenueBalanceEnvelope::new(
            Vec::new(),
            ListStatus::Fresh,
            "account_balance_runtime",
            10,
            Vec::new(),
            Vec::new(),
        ),
        VenuePositionEnvelope::new(
            Vec::new(),
            ListStatus::Fresh,
            "account_position_runtime",
            10,
            Vec::new(),
            Vec::new(),
        ),
        open_orders,
        &[],
        10,
    );

    assert_eq!(snapshot.account_bindings.len(), 1);
    assert!(snapshot.field_quality.iter().any(|row| {
        row.subject.kind == shared_types::AccountFieldSubjectKind::OpenOrder
            && row.subject.account_scope.as_deref() == Some("unified")
    }));
}
