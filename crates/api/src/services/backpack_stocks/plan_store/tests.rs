use super::super::plans::tests::fixture_plan;
use super::*;

fn changed(mut plan: StockExecutionPlan, suffix: &str) -> StockExecutionPlan {
    plan.request.request_id = format!("local-stock-plan-{suffix}");
    plan.terms.cex_instruction = Some(order_compile::compile(&plan.request, &plan.terms).unwrap());
    plan.plan_id = plan_id(&plan.request, &plan.terms).unwrap();
    plan
}

#[test]
fn stock_plan_store_idempotence_cancel_restart_and_expiry_release_shared_wallet() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let wallets = Arc::new(WalletClaims::default());
    let store = PlanStore::load(Some(path.clone()), wallets.clone());
    let plan = fixture_plan(10_000);
    let owner = &plan.request.wallet_address;
    store.reserve(plan.clone(), 10_000).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(store.reserve(plan.clone(), 10_001).unwrap(), plan);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(wallets.check("solana", owner, 10_001).is_err());
    let cancel = store.cancel(&plan.plan_id, 10_100).unwrap();
    assert_eq!(cancel.phase, StockPlanPhase::Cancelled);
    assert_eq!(cancel.revision, 2);
    assert_eq!(
        store
            .previous(&plan.request, &plan.terms.account_fingerprint)
            .unwrap(),
        Some(cancel)
    );
    assert!(wallets.check("solana", owner, 10_100).is_ok());
    let next = changed(plan.clone(), "next");
    store.reserve(next.clone(), 10_101).unwrap();
    drop(store);
    let restored_wallets = Arc::new(WalletClaims::default());
    let restored = PlanStore::load(Some(path.clone()), restored_wallets.clone());
    assert!(restored.problem().is_none());
    assert_eq!(restored.records().len(), 2);
    assert!(restored_wallets.check("solana", owner, 69_999).is_err());
    assert!(restored_wallets.check("solana", owner, 70_000).is_ok());
    assert_eq!(next.phase_at(70_000), StockPlanPhase::Expired);
    let after = std::fs::read(&path).unwrap();
    restored.cancel(&next.plan_id, 70_000).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), after);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn stock_plan_store_race_account_key_rotation_and_cross_module_conflicts_do_not_double_reserve() {
    let temp = tempfile::tempdir().unwrap();
    let wallets = Arc::new(WalletClaims::default());
    let store = Arc::new(PlanStore::load(
        Some(temp.path().join("plans.jsonl")),
        wallets.clone(),
    ));
    let first = fixture_plan(10_000);
    let mut second = changed(first.clone(), "second");
    second.terms.account_fingerprint = "rotated-key-same-configured-account".into();
    second.request.wallet_address = bs58::encode([8; 32]).into_string();
    second.terms.chain_cost.wallet_address = second.request.wallet_address.clone();
    second.terms.cex_instruction =
        Some(order_compile::compile(&second.request, &second.terms).unwrap());
    second.plan_id = plan_id(&second.request, &second.terms).unwrap();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let tasks = [first.clone(), second]
        .into_iter()
        .map(|p| {
            let (store, gate) = (store.clone(), gate.clone());
            std::thread::spawn(move || {
                gate.wait();
                store.reserve(p, 10_000)
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tasks
            .into_iter()
            .map(|h| h.join().unwrap().is_ok())
            .filter(|ok| *ok)
            .count(),
        1
    );
    assert_eq!(store.records().len(), 1);
    let active = store.records().remove(0);
    store.cancel(&active.plan_id, 10_001).unwrap();
    let other = Owner::new(Module::CrossChain, "local-cross-chain");
    wallets
        .commit(
            other.clone(),
            Some(Hold::wallet("solana", &first.request.wallet_address, None).unwrap()),
            10_002,
            || Ok(()),
        )
        .unwrap();
    let next = changed(first, "cross-module");
    assert!(store
        .reserve(next.clone(), 10_002)
        .unwrap_err()
        .contains("钱包已由"));
    assert!(store.problem().is_none());
    wallets.commit(other, None, 10_003, || Ok(())).unwrap();
    store.reserve(next, 10_003).unwrap();
}

#[test]
fn stock_plan_store_preserves_corrupt_tail_and_exclusive_owner_blocks_new_claims() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let store = PlanStore::load(Some(path.clone()), Default::default());
    let plan = fixture_plan(10_000);
    store.reserve(plan.clone(), 10_000).unwrap();
    let other = PlanStore::load(Some(path.clone()), Default::default());
    assert!(other.problem().unwrap().contains("另一实例"));
    drop(other);
    drop(store);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"version\":1")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let wallets = Arc::new(WalletClaims::default());
    let restored = PlanStore::load(Some(path.clone()), wallets.clone());
    assert_eq!(restored.records(), vec![plan.clone()]);
    assert!(restored.problem().unwrap().contains("不完整"));
    assert!(restored.cancel(&plan.plan_id, 10_001).is_err());
    assert!(restored.reserve(changed(plan, "blocked"), 10_001).is_err());
    assert!(wallets
        .check("solana", &bs58::encode([9; 32]).into_string(), 99_999)
        .is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn stock_plan_store_unresolved_submission_replay_never_expires_or_cancels() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let store = PlanStore::load(Some(path.clone()), Default::default());
    let mut plan = fixture_plan(10_000);
    store.reserve(plan.clone(), 10_000).unwrap();
    drop(store);
    // Fault fixture only; no production order submitter exists for stock plans yet.
    plan.phase = StockPlanPhase::SubmissionUnknown;
    plan.revision = 2;
    plan.updated_at_ms = 10_001;
    let mut bytes = serde_json::to_vec(&Entry {
        version: 1,
        plan: plan.clone(),
    })
    .unwrap();
    bytes.push(b'\n');
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let wallets = Arc::new(WalletClaims::default());
    let restored = PlanStore::load(Some(path), wallets.clone());
    assert!(restored.problem().is_none());
    assert_eq!(restored.records(), vec![plan.clone()]);
    assert!(plan.holds_funds(1_000_000));
    assert!(restored
        .cancel(&plan.plan_id, 1_000_000)
        .unwrap_err()
        .contains("未明提交"));
    assert!(wallets
        .check("solana", &plan.request.wallet_address, 1_000_000)
        .is_err());
}

#[test]
fn stock_plan_store_write_failure_does_not_publish_reservation_or_claim_wallet_free() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let wallets = Arc::new(WalletClaims::default());
    let store = PlanStore::load(Some(path.clone()), wallets.clone());
    std::fs::create_dir(&path).unwrap();
    let plan = fixture_plan(10_000);
    assert!(store
        .reserve(plan.clone(), 10_000)
        .unwrap_err()
        .contains("写入结果未核清"));
    assert!(store.records().is_empty());
    assert!(wallets
        .check("solana", &plan.request.wallet_address, 10_001)
        .is_err());
}

#[test]
fn stock_plan_legacy_hash_survives_replay_and_mismatched_compiled_order_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let mut plan = fixture_plan(10_000);
    plan.terms.cex_instruction = None;
    plan.plan_id = plan_id(&plan.request, &plan.terms).unwrap();
    let json = serde_json::to_string(&Entry {
        version: 1,
        plan: plan.clone(),
    })
    .unwrap();
    assert!(!json.contains("cexInstruction"));
    std::fs::write(&path, format!("{json}\n")).unwrap();
    let restored = PlanStore::load(Some(path), Default::default());
    assert!(restored.problem().is_none());
    assert_eq!(restored.records(), vec![plan.clone()]);
    restored.cancel(&plan.plan_id, 10_001).unwrap();
    let mut bad = fixture_plan(10_000);
    if let Some(StockCexInstruction::OrderBook { quantity, .. }) = &mut bad.terms.cex_instruction {
        *quantity = "100".into();
    }
    bad.plan_id = plan_id(&bad.request, &bad.terms).unwrap();
    assert!(validate(&bad).unwrap_err().contains("指令"));
    assert!(restored.reserve(bad, 10_001).is_err());
}
