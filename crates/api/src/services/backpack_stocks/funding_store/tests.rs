use super::*;
use funding_plan::tests::fixture;

#[test]
fn stock_funding_store_reserve_restart_cancel_expiry_and_request_retry_keep_one_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("funding.jsonl");
    let now = 10_000;
    let claims = Arc::new(WalletClaims::default());
    let store = FundingStore::load(Some(path.clone()), claims.clone());
    let plan = fixture(now);
    store.insert(plan.clone(), now).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(store.insert(plan.clone(), now).unwrap(), plan);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(claims
        .check("solana", &plan.request.wallet_address, now)
        .is_err());
    let mut bad = plan.request.clone();
    bad.funding_asset = "SOL".into();
    assert!(store
        .previous(&bad, &plan.terms.account_fingerprint)
        .is_err());
    assert!(store.previous(&plan.request, "other-key").is_err());
    drop(store);
    drop(claims);
    let claims = Arc::new(WalletClaims::default());
    let store = FundingStore::load(Some(path.clone()), claims.clone());
    assert_eq!(store.records(), vec![plan.clone()]);
    assert!(claims
        .check("solana", &plan.request.wallet_address, now)
        .is_err());
    let request = StockPlanRevisionRequest {
        plan_id: plan.plan_id.clone(),
        revision: 1,
    };
    let cancelled = store.cancel(&request, now + 1).unwrap();
    assert_eq!(cancelled.phase, StockFundingPlanPhase::Cancelled);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(store.cancel(&request, now + 2).unwrap(), cancelled);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(claims
        .check("solana", &plan.request.wallet_address, now)
        .is_ok());
    assert_eq!(store.insert(plan.clone(), now + 2).unwrap(), cancelled);
    let mut next = plan.clone();
    next.request.request_id = "local-funding-new-0002".into();
    next.plan_id = funding_plan::plan_id(&next.request, &next.terms).unwrap();
    store.insert(next.clone(), now + 2).unwrap();
    assert!(claims
        .check(
            "solana",
            &next.request.wallet_address,
            next.terms.valid_until_ms
        )
        .is_ok());
    assert_eq!(
        next.phase_at(next.terms.valid_until_ms),
        StockFundingPlanPhase::Expired
    );
    drop(store);
    let restored = FundingStore::load(Some(path), Arc::new(WalletClaims::default()));
    assert!(restored.problem().is_none());
}

#[test]
fn stock_funding_store_blocks_other_wallets_key_rotation_and_other_stock_plan_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let now = 10_000;
    let claims = Arc::new(WalletClaims::default());
    let store = FundingStore::load(Some(tmp.path().join("funding.jsonl")), claims.clone());
    let plan = fixture(now);
    store.insert(plan.clone(), now).unwrap();
    let mut other = plan.clone();
    other.request.request_id = "local-funding-other-0002".into();
    other.request.wallet_address = bs58::encode([9; 32]).into_string();
    other.terms.destination = other.request.wallet_address.clone();
    other.terms.account_fingerprint = "rotated-key".into();
    other.plan_id = funding_plan::plan_id(&other.request, &other.terms).unwrap();
    assert!(store
        .insert(other, now)
        .unwrap_err()
        .contains("股票补库计划"));
    let trading = plan_store::PlanStore::load(Some(tmp.path().join("stocks.jsonl")), claims);
    assert!(trading
        .reserve(plans::tests::fixture_plan(now), now)
        .unwrap_err()
        .contains("股票补库计划"));
    store
        .cancel(
            &StockPlanRevisionRequest {
                plan_id: plan.plan_id,
                revision: 1,
            },
            now + 1,
        )
        .unwrap();
    trading
        .reserve(plans::tests::fixture_plan(now), now + 1)
        .unwrap();
    let mut next = fixture(now);
    next.request.request_id = "local-funding-other-0003".into();
    next.plan_id = funding_plan::plan_id(&next.request, &next.terms).unwrap();
    assert!(store
        .insert(next, now + 1)
        .unwrap_err()
        .contains("股票套利计划"));
}

#[test]
fn stock_funding_store_preserves_corrupt_tail_and_exclusive_lock_fails_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("funding.jsonl");
    let plan = fixture(10_000);
    let store = FundingStore::load(Some(path.clone()), Arc::new(WalletClaims::default()));
    store.insert(plan, 10_000).unwrap();
    let other = FundingStore::load(Some(path.clone()), Arc::new(WalletClaims::default()));
    assert!(other.problem().unwrap().contains("其他实例"));
    drop(other);
    drop(store);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{incomplete")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let claims = Arc::new(WalletClaims::default());
    let store = FundingStore::load(Some(path.clone()), claims.clone());
    assert!(store.problem().is_some());
    assert!(claims
        .check("solana", &bs58::encode([8; 32]).into_string(), 10_000)
        .is_err());
    assert!(store.insert(fixture(10_001), 10_001).is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
