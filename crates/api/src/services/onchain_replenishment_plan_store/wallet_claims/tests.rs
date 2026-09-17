use super::*;

#[test]
fn wallet_claims_deposit_blocks_other_modules_and_does_not_expire_on_pause_or_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deposit.jsonl");
    let claims = Arc::new(WalletClaims::default());
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10)
        .with_wallet_claims(claims.clone());
    let plan = super::super::tests::deposit_plan("deposit", 10, 100_000);
    let leg = plan.legs[0].clone();
    store.insert(plan.clone(), 10).unwrap();
    let run = store.authorize("deposit", "one", "tester", 20).unwrap().run;
    let other = Owner::new(Module::Execution, "busy");
    claims
        .commit(
            other.clone(),
            Some(Hold::wallet(&leg.chain, leg.source_address.as_deref().unwrap(), None).unwrap()),
            21,
            || Ok(()),
        )
        .unwrap();
    assert!(store
        .claim_submission(
            &run.run_id,
            "tester",
            plan.clone(),
            0,
            &store.submission_snapshot(),
            30
        )
        .is_err());
    assert!(store.run(&run.run_id, 30).unwrap().transfers.is_empty());
    claims.commit(other, None, 31, || Ok(())).unwrap();
    store
        .claim_submission(
            &run.run_id,
            "tester",
            plan,
            0,
            &store.submission_snapshot(),
            32,
        )
        .unwrap();
    assert!(claims
        .check(
            &leg.chain,
            leg.source_address.as_deref().unwrap(),
            1_000_000
        )
        .is_err());
    drop(store);
    drop(claims);
    let claims = Arc::new(WalletClaims::default());
    let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 1_000_000)
        .with_wallet_claims(claims.clone());
    assert!(!restored
        .run(&run.run_id, 1_000_000)
        .unwrap()
        .transfers
        .is_empty());
    assert!(claims
        .check(
            &leg.chain,
            leg.source_address.as_deref().unwrap(),
            1_000_000
        )
        .is_err());
}
