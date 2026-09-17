use super::*;

#[test]
fn hyperliquid_evidence_requires_every_nonzero_spot_currency() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    service.account_summaries.insert(
        HYPERLIQUID_SPOT.to_owned(),
        VenueAccountSummary {
            venue: HYPERLIQUID_SPOT.to_owned(),
            account_type: "unifiedAccount".to_owned(),
            equity_scope: shared_types::AccountEquityScope::Unified,
            total_equity_usd: 100.0,
            total_available_balance_usd: 100.0,
            withdrawable_balance_usd: None,
            total_initial_margin_usd: 0.0,
            total_maintenance_margin_usd: 0.0,
            account_im_rate: 0.0,
            account_mm_rate: 0.0,
            source: "official-test".to_owned(),
            observed_at_ms: now_ms,
            freshness_ms: Some(0),
            problem: None,
        },
    );
    service.asset_valuations.insert(
        (HYPERLIQUID_SPOT.to_owned(), "USDC".to_owned()),
        VenueAssetValuation {
            venue: HYPERLIQUID_SPOT.to_owned(),
            currency: "USDC".to_owned(),
            usd_value: 99.0,
            source: "official-test".to_owned(),
            observed_at_ms: now_ms,
        },
    );

    assert!(service.hyperliquid_account_evidence_current(&["USDC".to_owned()], now_ms));
    assert!(!service
        .hyperliquid_account_evidence_current(&["HYPE".to_owned(), "USDC".to_owned()], now_ms));
}

#[test]
fn account_evidence_refresh_starts_before_binding_expiration() {
    let now_ms = 100_000;
    let observed_at_ms = now_ms - ACCOUNT_EVIDENCE_REFRESH_AFTER_MS - 1;

    assert!(evidence_is_valid(observed_at_ms, now_ms));
    assert!(!evidence_is_current(observed_at_ms, now_ms));
}

#[test]
fn account_evidence_refresh_claim_is_retry_bounded() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();

    assert!(service.claim_account_evidence_refresh(HYPERLIQUID, now_ms));
    assert!(!service.claim_account_evidence_refresh(HYPERLIQUID, now_ms + 1));
    assert!(service.claim_account_evidence_refresh(HYPERLIQUID, now_ms + ACCOUNT_EVIDENCE_RETRY_MS));
}

#[test]
fn failed_account_evidence_refresh_uses_longer_backoff() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();

    assert!(service.claim_account_evidence_refresh(HYPERLIQUID, now_ms));
    service.defer_account_evidence_refresh(HYPERLIQUID);

    assert!(
        !service.claim_account_evidence_refresh(HYPERLIQUID, now_ms + ACCOUNT_EVIDENCE_RETRY_MS,)
    );
    assert!(service.claim_account_evidence_refresh(
        HYPERLIQUID,
        common::time::now_ms() + ACCOUNT_EVIDENCE_FAILURE_RETRY_MS,
    ));
}

#[test]
fn account_evidence_refresh_ignores_zero_spot_assets() {
    let rows = [VenueBalanceInfo {
        venue: HYPERLIQUID_SPOT.to_owned(),
        currency: "USDC".to_owned(),
        total: 0.0,
        available: 0.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }];

    assert!(hyperliquid_nonzero_currencies(&rows).is_empty());
}
