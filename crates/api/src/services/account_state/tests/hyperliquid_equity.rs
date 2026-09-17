use super::support::{balance_row, health_row};
use super::*;

#[test]
fn unified_hyperliquid_children_use_spot_equity_summary() {
    let summaries = vec![hyperliquid_spot_summary()];
    let bindings = vec![hyperliquid_binding("unifiedAccount")];
    let balances = VenueBalanceEnvelope::new(
        vec![
            balance_row("hyperliquid:spot"),
            balance_row("hyperliquid:xyz"),
        ],
        ListStatus::Fresh,
        "account_balance_runtime",
        10,
        Vec::new(),
        Vec::new(),
    )
    .with_account_summaries(summaries);
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
    let health = [health_row(
        "hyperliquid:xyz",
        "positions",
        VenueOperationStatus::Ok,
    )];

    let quality =
        account_equity_unknown_quality(&balances, &positions, &open_orders, &health, &bindings, 10);

    assert!(quality.is_empty());
}

#[test]
fn unified_hyperliquid_without_spot_summary_stays_fail_closed() {
    let bindings = vec![hyperliquid_binding("unifiedAccount")];
    let balances = VenueBalanceEnvelope::new(
        vec![balance_row("hyperliquid:spot")],
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

    let quality =
        account_equity_unknown_quality(&balances, &positions, &open_orders, &[], &bindings, 10);

    assert_eq!(quality.len(), 1);
    assert_eq!(quality[0].subject.venue.as_deref(), Some("hyperliquid"));
}

fn hyperliquid_binding(scope: &str) -> AccountBindingEvidence {
    AccountBindingEvidence {
        venue: "hyperliquid".to_owned(),
        account_scope: Some(scope.to_owned()),
        status: shared_types::AccountBindingStatus::Verified,
        source: "userAbstraction".to_owned(),
        checked_at_ms: Some(10),
        freshness_ms: Some(0),
        credential_fingerprint: None,
        problem: None,
    }
}

fn hyperliquid_spot_summary() -> VenueAccountSummary {
    VenueAccountSummary {
        venue: "hyperliquid:spot".to_owned(),
        account_type: "spot".to_owned(),
        equity_scope: shared_types::AccountEquityScope::Spot,
        total_equity_usd: 25.0,
        total_available_balance_usd: 20.0,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: 0.0,
        total_maintenance_margin_usd: 0.0,
        account_im_rate: 0.0,
        account_mm_rate: 0.0,
        source: "spotMetaAndAssetCtxs.markPx".to_owned(),
        observed_at_ms: 10,
        freshness_ms: Some(0),
        problem: None,
    }
}
