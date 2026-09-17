use super::super::recovery::tests::{pending_bridge, received};
use super::*;

fn fixture() -> (
    tempfile::TempDir,
    OnchainCrossChainRunStore,
    OnchainCrossChainRun,
    Preview,
    OnchainUnsignedTransaction,
) {
    let dir = tempfile::tempdir().unwrap();
    let (store, run) = pending_bridge(&dir.path().join("recovery.jsonl"));
    let mut report = received(&run);
    let token = run.build.legs[0].to_token.clone();
    report.receiving_token = Some(token.clone());
    report.token_resolution.as_mut().unwrap().address = token.clone();
    let identity = report
        .token_resolution
        .as_mut()
        .unwrap()
        .identity
        .as_mut()
        .unwrap();
    identity.address = token;
    identity.symbol = "TKN".into();
    let basis = super::super::recovery::basis(&run, &run.legs[1], &report).unwrap();
    report.receipt.as_mut().unwrap().basis = basis;
    let run = store
        .record_bridge_recovery(&run.run_id, 2, report, 3000)
        .unwrap();
    let disposition = run
        .accounting
        .as_ref()
        .unwrap()
        .disposition
        .as_ref()
        .unwrap();
    let mut input = disposition.remaining_assets[0].change.clone();
    input.amount_exact = "50".into();
    let target = disposition.original_capital.clone().unwrap();
    let transaction: OnchainUnsignedTransaction = serde_json::from_value(serde_json::json!({"kind":"evm_call","chain_id":1,
        "from":input.wallet,"to":format!("0x{:040x}", 9),"data":"0x1234","value":"0x0","gas":"0x5208"})).unwrap();
    let preview: Preview = serde_json::from_value(serde_json::json!({
        "sourceRunId":run.run_id,"sourceRunUpdatedAtMs":run.updated_at_ms,"assetIndex":0,
        "input":input,"target":target,"inputAmountRaw":"50000000","balanceAmountRaw":"200000000",
        "balanceSource":"fixture_rpc","balanceCheckedAtMs":3900,"routeId":"quote1","provider":"lifi",
        "expectedOutputAmountRaw":"49000000","minimumOutputAmountRaw":"48500000","feeUsd":0.1,"gasUsd":0.02,
        "quoteObservedAtMs":3950,"validUntilMs":14000,"blockers":[],"quoteReady":true,"submitReady":false,
        "requiresLiveAuthorization":true,"officialDocsUrl":"https://docs.li.fi/api-reference/get-a-quote-for-a-token-transfer"
    })).unwrap();
    (dir, store, run, preview, transaction)
}

fn authorize(plan: &Plan, key: &str) -> Authorize {
    Authorize {
        plan_id: plan.plan_id.clone(),
        idempotency_key: key.into(),
        confirmation: ONCHAIN_RECOVERY_RESERVATION_PHRASE.into(),
    }
}

#[test]
fn wallet_claims_recovery_and_execution_race_cancel_and_restore_use_the_same_journals() {
    use crate::services::{
        onchain_execution_run_store::{test_checkpoint, OnchainExecutionRunStore},
        onchain_wallet_claims::WalletClaims,
    };
    use std::sync::{Arc, Barrier};

    let (dir, store, _, preview, transaction) = fixture();
    let shared = Arc::new(WalletClaims::default());
    let store = Arc::new(store.with_wallet_claims(shared.clone()));
    let plan = store
        .save_recovery_plan(preview.clone(), transaction, 4000)
        .unwrap();
    let mut config = AppConfig::default();
    config.storage.onchain_execution_run_ledger_path =
        Some(dir.path().join("execution.jsonl").to_string_lossy().into());
    let execution = Arc::new(
        OnchainExecutionRunStore::load(&config)
            .store
            .with_wallet_claims(shared.clone()),
    );
    let mut checkpoint = test_checkpoint();
    checkpoint.build.chain = preview.input.chain.clone();
    checkpoint.build.wallet_address = preview.input.wallet.clone();
    checkpoint.response.updated_at_ms = 5000;
    let barrier = Arc::new(Barrier::new(2));
    let reserve = {
        let (store, barrier, plan) = (store.clone(), barrier.clone(), plan.clone());
        std::thread::spawn(move || {
            barrier.wait();
            store.reserve_recovery_plan(&authorize(&plan, "race"), "tester", 5000)
        })
    };
    let submit = {
        let (execution, barrier, checkpoint) =
            (execution.clone(), barrier.clone(), checkpoint.clone());
        std::thread::spawn(move || {
            barrier.wait();
            execution.append_pending(&checkpoint)
        })
    };
    let reserved = reserve.join().unwrap().is_ok();
    let submitted = submit.join().unwrap().is_ok();
    assert_ne!(reserved, submitted);
    assert!(shared.check(&preview.input.chain, &preview.input.wallet, 5001).is_err());
    if reserved {
        store.cancel_recovery_plan(&plan.plan_id, "tester", 5100).unwrap();
        checkpoint.response.updated_at_ms = 5101;
        execution.append_pending(&checkpoint).unwrap();
    }
    drop(store);
    drop(execution);
    drop(shared);

    let shared = Arc::new(WalletClaims::default());
    let restored = OnchainCrossChainRunStore::load_path(
        Some(dir.path().join("recovery.jsonl")), 6000,
    ).with_wallet_claims(shared.clone());
    let execution = OnchainExecutionRunStore::load(&config)
        .store
        .with_wallet_claims(shared.clone());
    assert!(shared.check(&preview.input.chain, &preview.input.wallet, 100_000).is_err());
    let mut next_preview = preview.clone();
    next_preview.route_id = Some("fresh-recovery".into());
    let tx: OnchainUnsignedTransaction = serde_json::from_value(serde_json::json!({
        "kind": "evm_call", "chain_id": 1, "from": preview.input.wallet,
        "to": format!("0x{:040x}", 9), "data": "0x1234", "value": "0x0", "gas": "0x5208"
    })).unwrap();
    let next = restored.save_recovery_plan(next_preview, tx, 6100).unwrap();
    assert!(restored
        .reserve_recovery_plan(&authorize(&next, "after-restart"), "tester", 6200)
        .unwrap_err()
        .contains("链上 / CEX 执行"));
    let mut finished = checkpoint.response.clone();
    finished.status = shared_types::OnchainExecutionRunStatus::Completed;
    finished.updated_at_ms = 6300;
    execution.append_run(&finished).unwrap();
    assert!(shared.check(&preview.input.chain, &preview.input.wallet, 6301).is_ok());
    restored
        .reserve_recovery_plan(&authorize(&next, "after-restart"), "tester", 6400)
        .unwrap();
    assert!(shared.check(&preview.input.chain, &preview.input.wallet, 13999).is_err());
    assert!(shared.check(&preview.input.chain, &preview.input.wallet, 14000).is_ok());
}

#[test]
fn recovery_plan_save_reserve_cancel_and_restart_are_idempotent() {
    let (dir, store, run, preview, transaction) = fixture();
    let plan = store
        .save_recovery_plan(preview.clone(), transaction.clone(), 4000)
        .unwrap();
    assert_eq!(plan.status, Status::AwaitingAuthorization);
    assert_eq!(
        store
            .save_recovery_plan(preview, transaction, 4500)
            .unwrap(),
        plan
    );
    let request = authorize(&plan, "reserve-once");
    let reserved = store
        .reserve_recovery_plan(&request, "tester", 5000)
        .unwrap();
    assert_eq!(reserved.status, Status::Reserved);
    assert!(reserved.reservation_active(5001));
    assert!(!reserved.reservation_active(14000));
    assert_eq!(
        store
            .reserve_recovery_plan(&request, "tester", 5100)
            .unwrap(),
        reserved
    );
    let restored =
        OnchainCrossChainRunStore::load_path(Some(dir.path().join("recovery.jsonl")), 6000);
    assert!(restored.readiness().is_ok());
    assert_eq!(restored.recovery_plan_rows(6000), vec![reserved]);
    assert_eq!(restored.run(&run.run_id, 6000).unwrap(), run);
    let cancelled = restored
        .cancel_recovery_plan(&plan.plan_id, "tester", 6500)
        .unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(
        restored
            .cancel_recovery_plan(&plan.plan_id, "tester", 6600)
            .unwrap(),
        cancelled
    );
    assert_eq!(
        restored
            .reserve_recovery_plan(&request, "tester", 6700)
            .unwrap(),
        cancelled
    );
    assert!(restored
        .reserve_recovery_plan(&authorize(&plan, "new-key"), "tester", 6800)
        .is_err());
    let restored =
        OnchainCrossChainRunStore::load_path(Some(dir.path().join("recovery.jsonl")), 7000);
    assert_eq!(restored.recovery_plan_rows(7000), vec![cancelled]);
}

#[test]
fn recovery_plan_expiry_wrong_actor_and_changed_source_cannot_extend_authorization() {
    let (dir, store, run, preview, transaction) = fixture();
    let plan = store
        .save_recovery_plan(preview.clone(), transaction.clone(), 4000)
        .unwrap();
    let request = authorize(&plan, "lease");
    assert!(store
        .reserve_recovery_plan(&request, "other", 5000)
        .is_err());
    let mut invalid = request.clone();
    invalid.confirmation.clear();
    assert!(store
        .reserve_recovery_plan(&invalid, "tester", 5000)
        .is_err());
    let reserved = store
        .reserve_recovery_plan(&request, "tester", 5000)
        .unwrap();
    assert!(store
        .cancel_recovery_plan(&plan.plan_id, "other", 5100)
        .is_err());
    let expired = store
        .reserve_recovery_plan(&request, "tester", 14000)
        .unwrap();
    assert_eq!(expired.status, Status::Expired);
    assert_eq!(expired.authorization, reserved.authorization);
    let mut newer = preview;
    newer.route_id = Some("new-quote".into());
    let other_plan = store.save_recovery_plan(newer, transaction, 6000).unwrap();
    assert!(store
        .reserve_recovery_plan(&authorize(&other_plan, "lease"), "tester", 6500)
        .is_err());
    store
        .request_recheck(&run.run_id, "tester", 2, 7000)
        .unwrap();
    assert!(store
        .reserve_recovery_plan(&authorize(&other_plan, "new-source"), "tester", 7100)
        .unwrap_err()
        .contains("已变化"));
    assert!(
        OnchainCrossChainRunStore::load_path(Some(dir.path().join("recovery.jsonl")), 7200)
            .readiness()
            .is_ok()
    );
}

#[test]
fn recovery_plan_reservations_are_atomic_for_the_whole_chain_wallet() {
    let (_dir, store, _, mut preview, transaction) = fixture();
    let one = store
        .save_recovery_plan(preview.clone(), transaction.clone(), 4000)
        .unwrap();
    preview.route_id = Some("other-quote".into());
    let two = store
        .save_recovery_plan(preview, transaction, 4001)
        .unwrap();
    let store = std::sync::Arc::new(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let tasks = [one, two]
        .into_iter()
        .enumerate()
        .map(|(i, plan)| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.reserve_recovery_plan(&authorize(&plan, &format!("key-{i}")), "tester", 5000)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = tasks
        .into_iter()
        .map(|task| task.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results
        .iter()
        .filter_map(|r| r.as_ref().err())
        .any(|e| e.contains("预留")));
    assert!(same_wallet("ethereum", "0xAbC", "ethereum", "0xabc"));
    assert!(!same_wallet("solana", "AbC", "solana", "abc"));
    assert!(!same_wallet("base", "0xabc", "ethereum", "0xabc"));
}

fn claim_normal(
    store: &OnchainCrossChainRunStore,
    run_id: &str,
    now: i64,
) -> Result<CrossChainLegClaimOutcome, CrossChainLegClaimError> {
    let template = super::super::accounting::tests::fixture();
    let leg = &template.legs[0];
    store.claim_leg(
        run_id,
        "tester",
        1,
        leg.submitted_input_amount_raw.clone().unwrap(),
        leg.minimum_output_amount_raw.clone().unwrap(),
        leg.provider_transaction_id
            .clone()
            .unwrap_or_else(|| "provider-1".into()),
        leg.swap_execution.clone(),
        None,
        100,
        10000,
        "104000000".into(),
        "100000000".into(),
        "300".into(),
        now,
    )
}

#[test]
fn recovery_plan_and_normal_cross_chain_submission_block_each_other() {
    let (_dir, store, run, preview, transaction) = fixture();
    let plan = store
        .save_recovery_plan(preview, transaction, 4000)
        .unwrap();
    store
        .reserve_recovery_plan(&authorize(&plan, "lease"), "tester", 5000)
        .unwrap();
    let normal = store
        .authorize(&run.build.build_id, "new-arbitrage", "tester", 5100)
        .unwrap()
        .run;
    assert!(
        matches!(claim_normal(&store, &normal.run_id, 5200), Err(CrossChainLegClaimError::InvalidQuote(p)) if p.contains("预留"))
    );
    store
        .cancel_recovery_plan(&plan.plan_id, "tester", 5300)
        .unwrap();
    claim_normal(&store, &normal.run_id, 5400).unwrap();
    let mut preview = plan.preview;
    preview.plan_id = None;
    preview.route_id = Some("requote".into());
    let transaction = store
        .recovery_plans
        .get(&plan.plan_id)
        .unwrap()
        .transaction
        .clone();
    let fresh = store
        .save_recovery_plan(preview, transaction, 5500)
        .unwrap();
    assert!(store
        .reserve_recovery_plan(&authorize(&fresh, "lease-2"), "tester", 5600)
        .unwrap_err()
        .contains("执行中"));
    store
        .pause(&normal.run_id, "广播结果未明".into(), 5700)
        .unwrap();
    assert!(store
        .reserve_recovery_plan(&authorize(&fresh, "after-pause"), "tester", 5800)
        .is_err());
}

#[test]
fn recovery_plan_journal_tampering_keeps_verified_prefix_and_failed_writes_do_not_reserve() {
    let (dir, store, _, preview, transaction) = fixture();
    let plan = store
        .save_recovery_plan(preview, transaction, 4000)
        .unwrap();
    let mut altered = store
        .recovery_plans
        .get(&plan.plan_id)
        .unwrap()
        .value()
        .clone();
    altered.plan.preview.minimum_output_amount_raw = Some("1".into());
    append_jsonl(
        &dir.path().join("recovery.jsonl"),
        &LogEntry {
            schema_version: SCHEMA_VERSION,
            build: None,
            run: None,
            recovery_plan: Some(altered),
        },
    )
    .unwrap();
    let restored =
        OnchainCrossChainRunStore::load_path(Some(dir.path().join("recovery.jsonl")), 5000);
    assert!(restored.readiness().is_err());
    assert_eq!(restored.recovery_plan_rows(5000), vec![plan.clone()]);
    assert!(restored
        .reserve_recovery_plan(&authorize(&plan, "blocked"), "tester", 5000)
        .is_err());
    std::fs::remove_file(dir.path().join("recovery.jsonl")).unwrap();
    std::fs::create_dir(dir.path().join("recovery.jsonl")).unwrap();
    assert!(store
        .reserve_recovery_plan(&authorize(&plan, "write-failed"), "tester", 5000)
        .is_err());
    assert_eq!(
        store.recovery_plan_rows(5000)[0].status,
        Status::AwaitingAuthorization
    );
}
