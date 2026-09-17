use super::*;

fn plan(id: &str) -> OnchainReplenishmentPlanResponse {
    let run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    let mut plan = run.plan;
    plan.plan_id = id.into();
    plan.valid_until_ms = 1_000_000;
    plan.legs[0].direction = OnchainTransferDirection::WithdrawToChain;
    plan
}

fn authorize(
    store: &OnchainReplenishmentPlanStore,
    plan: &OnchainReplenishmentPlanResponse,
) -> String {
    store.insert(plan.clone(), 10).unwrap();
    store
        .authorize(&plan.plan_id, &plan.plan_id, "operator", 20)
        .unwrap()
        .run
        .run_id
}

fn claim(
    store: &OnchainReplenishmentPlanStore,
    id: &str,
    plan: &OnchainReplenishmentPlanResponse,
) -> Result<ReplenishmentSubmitClaimOutcome, ReplenishmentSubmitClaimError> {
    store.claim_submission(
        id,
        "operator",
        plan.clone(),
        0,
        &store.submission_snapshot(),
        30,
    )
}

#[test]
fn replenishment_source_guard_new_plan_cannot_bypass_uncertain_withdrawal_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
    let original = plan("original");
    let first_id = authorize(&store, &original);
    let first = claim(&store, &first_id, &original).unwrap();
    store
        .pause_submission(&first_id, "response lost".into(), 31)
        .unwrap();
    drop(store);
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 40);
    let mut changed = plan("new-plan-new-key");
    changed.legs[0].venue = " BINANCE ".into();
    changed.legs[0].asset = "usdc".into();
    changed.legs[0].chain = "ethereum".into();
    changed.legs[0].destination.address = Some("different-destination".into());
    let id = authorize(&store, &changed);
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        store
            .submission_snapshot()
            .ensure_available(&changed.legs[0]),
        Err(ReplenishmentSubmitClaimError::SourceBusy(_))
    ));
    let error = claim(&store, &id, &changed).unwrap_err();
    let ReplenishmentSubmitClaimError::SourceBusy(message) = error else {
        panic!("{error:?}")
    };
    assert!(message.contains(&first_id));
    assert!(message.contains(&first.run.transfers[0].client_transfer_id));
    assert!(message.contains("本次未发送"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(store.run(&id, 40).unwrap().transfers.is_empty());
    assert!(claim(&store, &first_id, &original).unwrap().replayed);
}

#[test]
fn replenishment_source_guard_independent_cex_sources_do_not_invalidate_preflight() {
    let store = OnchainReplenishmentPlanStore::load_path(None, 10);
    let first = plan("first");
    let first_id = authorize(&store, &first);
    let before = store.submission_snapshot();
    for (id, venue, asset) in [("venue", "kraken", "USDC"), ("asset", "binance", "USDT")] {
        let mut other = plan(id);
        other.legs[0].venue = venue.into();
        other.legs[0].asset = asset.into();
        let other_id = authorize(&store, &other);
        claim(&store, &other_id, &other).unwrap();
    }
    store
        .claim_submission(&first_id, "operator", first, 0, &before, 30)
        .unwrap();
}

#[test]
fn replenishment_source_guard_wallet_shares_gas_but_solana_addresses_are_case_sensitive() {
    for (chain, original_wallet, new_wallet, blocks) in [
        ("ethereum", "0xabcDef", "0xABCdef", true),
        ("solana", "WalletAbc", "WalletAbc", true),
        ("solana", "WalletAbc", "Walletabc", false),
    ] {
        let store = OnchainReplenishmentPlanStore::load_path(None, 10);
        let mut first = plan("first");
        first.legs[0].direction = OnchainTransferDirection::DepositToCex;
        first.legs[0].chain = chain.into();
        first.legs[0].source_address = Some(original_wallet.into());
        let first_id = authorize(&store, &first);
        claim(&store, &first_id, &first).unwrap();
        let mut other = first.clone();
        other.plan_id = "second".into();
        other.legs[0].asset = "USDT".into();
        other.legs[0].venue = "kraken".into();
        other.legs[0].source_address = Some(new_wallet.into());
        let id = authorize(&store, &other);
        assert_eq!(
            matches!(
                claim(&store, &id, &other),
                Err(ReplenishmentSubmitClaimError::SourceBusy(_))
            ),
            blocks
        );
        other.plan_id = "different-chain".into();
        other.legs[0].chain = "base".into();
        let id = authorize(&store, &other);
        claim(&store, &id, &other).unwrap();
    }
}

#[test]
fn replenishment_source_guard_concurrent_claims_persist_only_one_transfer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
    let requests = [plan("one"), plan("two")].map(|plan| {
        let id = authorize(&store, &plan);
        (id, plan)
    });
    let barrier = std::sync::Barrier::new(2);
    let outcomes = std::thread::scope(|scope| {
        let handles = requests
            .into_iter()
            .map(|(id, plan)| {
                let store = &store;
                let barrier = &barrier;
                scope.spawn(move || {
                    let snapshot = store.submission_snapshot();
                    barrier.wait();
                    store.claim_submission(&id, "operator", plan, 0, &snapshot, 30)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r, Err(ReplenishmentSubmitClaimError::SourceBusy(_))))
            .count(),
        1
    );
    let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 40);
    assert_eq!(
        restored
            .runs(10, 40)
            .rows
            .iter()
            .map(|r| r.transfers.len())
            .sum::<usize>(),
        1
    );
}

#[test]
fn replenishment_source_guard_completed_transfer_invalidates_old_balance_then_releases_source() {
    let store = OnchainReplenishmentPlanStore::load_path(None, 10);
    let first = plan("first");
    let second = plan("second");
    let first_id = authorize(&store, &first);
    let second_id = authorize(&store, &second);
    let before = store.submission_snapshot();
    claim(&store, &first_id, &first).unwrap();
    store
        .record_source_status(
            &first_id,
            OnchainReplenishmentTransferStatus::SourceCompleted,
            "provider".into(),
            Some("hash".into()),
            Some(1),
            "fixture".into(),
            None,
            31,
        )
        .unwrap();
    assert!(matches!(
        claim(&store, &second_id, &second),
        Err(ReplenishmentSubmitClaimError::SourceBusy(_))
    ));
    store
        .record_destination_credit(
            &first_id,
            Decimal::new(125, 1),
            None,
            Some(2),
            "fixture".into(),
            32,
        )
        .unwrap();
    assert_eq!(
        store.claim_submission(&second_id, "operator", second.clone(), 0, &before, 33),
        Err(ReplenishmentSubmitClaimError::SourceChanged)
    );
    claim(&store, &second_id, &second).unwrap();
}

#[test]
fn replenishment_source_guard_definitive_preflight_rejection_releases_but_unknown_or_sent_does_not()
{
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
    let first = plan("first");
    let first_id = authorize(&store, &first);
    claim(&store, &first_id, &first).unwrap();
    let rejected = store
        .reject_before_send(&first_id, "fee exceeds cap; not sent".into(), 31)
        .unwrap();
    assert_eq!(rejected.status, OnchainReplenishmentRunStatus::Failed);
    assert!(rejected.recheck_request().is_none());
    assert!(rejected.next_action.contains("未发送"));
    drop(store);
    let store = OnchainReplenishmentPlanStore::load_path(Some(path), 40);
    let second = plan("second");
    let second_id = authorize(&store, &second);
    claim(&store, &second_id, &second).unwrap();
    store
        .record_submission_ack(
            &second_id,
            "provider".into(),
            None,
            41,
            "fixture".into(),
            41,
        )
        .unwrap();
    assert!(store
        .reject_before_send(&second_id, "late rejection".into(), 42)
        .is_err());
    let third = plan("third");
    let third_id = authorize(&store, &third);
    assert!(matches!(
        claim(&store, &third_id, &third),
        Err(ReplenishmentSubmitClaimError::SourceBusy(_))
    ));
    store
        .pause_submission(&second_id, "timeout".into(), 43)
        .unwrap();
    assert!(store
        .reject_before_send(&second_id, "not proven".into(), 44)
        .is_err());
    assert!(matches!(
        claim(&store, &third_id, &third),
        Err(ReplenishmentSubmitClaimError::SourceBusy(_))
    ));
}
