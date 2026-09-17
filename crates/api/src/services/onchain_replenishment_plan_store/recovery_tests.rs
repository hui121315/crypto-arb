use super::*;

fn fixture(
    key: &str,
    status: OnchainReplenishmentRunStatus,
    updated_at_ms: i64,
) -> OnchainReplenishmentRun {
    let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    run.run_id = run_id(key);
    run.idempotency_key = key.to_owned();
    run.plan.plan_id = format!("plan-{key}");
    run.plan.valid_until_ms = 1_000_000;
    run.authorization.actor = "operator".to_owned();
    run.authorization.valid_until_ms = 1_000_000;
    run.status = status;
    run.updated_at_ms = updated_at_ms;
    run
}

fn line(run: &OnchainReplenishmentRun) -> String {
    format!(
        "{}\n",
        serde_json::to_string(&LogEntry {
            schema_version: SCHEMA_VERSION,
            plan: None,
            run: Some(run.clone()),
        })
        .unwrap()
    )
}

#[test]
fn replenishment_recheck_keeps_original_scope_and_cannot_resume_funds_after_restart() {
    for two_legs in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        let mut run = fixture("recheck", OnchainReplenishmentRunStatus::Paused, 60);
        run.transfers[0].status = OnchainReplenishmentTransferStatus::SourceCompleted;
        run.transfers[0].credited_amount_exact = None;
        if two_legs { run.plan.legs.push(run.plan.legs[0].clone()); }
        std::fs::write(&path, line(&run)).unwrap();
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 100);
        assert!(store.wallet_claims.check("solana", "other-wallet", 100).is_err());
        let request = run.recheck_request().unwrap();
        assert!(store.request_recheck(&request, "another-actor", 100).is_err());
        let mut wrong = request.clone(); wrong.expected_client_transfer_id = "different-transfer".into();
        assert!(store.request_recheck(&wrong, "operator", 100).is_err());
        let checked = store.request_recheck(&request, "operator", 100).unwrap();
        assert_eq!(checked.plan, run.plan);
        assert_eq!(checked.authorization, run.authorization);
        assert_eq!(checked.transfers[0].transaction_id, run.transfers[0].transaction_id);
        assert_eq!(checked.transfers[0].submission_attempted_at_ms, 20);
        assert!(checked.read_only_recovery);
        assert!(store.is_read_only_wallet_update(&checked));
        let mut new_intent = checked.clone();
        new_intent.transfers.push(checked.transfers[0].clone());
        assert!(!store.is_read_only_wallet_update(&new_intent));
        let mut changed_hash = checked.clone();
        changed_hash.transfers[0].transaction_id = Some("different-transaction".into());
        assert!(!store.is_read_only_wallet_update(&changed_hash));
        assert!(store.wallet_claims.check("solana", "other-wallet", 100).is_err());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(store.request_recheck(&request, "operator", 101).unwrap(), checked);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        store.claim_recovery_check(&run.run_id, 100).unwrap().unwrap();
        assert!(store.claim_recovery_check(&run.run_id, 101).unwrap().is_none());
        let restored = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 102);
        assert_eq!(restored.run(&run.run_id, 102).unwrap().recovery_checks, 1);
        let credited = restored.record_destination_credit(&run.run_id, Decimal::new(125,1), Some(true), Some(40), "fixture receipt".into(), 103).unwrap();
        assert_eq!(credited.status, if two_legs { OnchainReplenishmentRunStatus::Paused } else { OnchainReplenishmentRunStatus::Completed });
        assert!(credited.recheck_request().is_none());
        assert_eq!(credited.transfers.len(), 1);
        assert!(restored.claim_submission(&run.run_id, "operator", run.plan.clone(), 1, &restored.submission_snapshot(), 104).is_err());
        let again = OnchainReplenishmentPlanStore::load_path(Some(path), 105);
        assert!(again.run(&run.run_id, 105).unwrap().read_only_recovery);
        assert!(again.wallet_claims.check("solana", "other-wallet", 105).is_ok());
    }
}

#[test]
fn replenishment_recheck_caps_queries_and_requires_known_broadcast_for_chain_deposit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let mut run = fixture("bounded", OnchainReplenishmentRunStatus::Paused, 60);
    run.transfers[0].status = OnchainReplenishmentTransferStatus::Paused;
    run.transfers[0].transaction_id = None;
    assert!(run.recheck_request().is_none());
    run.plan.legs[0].direction = shared_types::OnchainTransferDirection::WithdrawToChain;
    assert!(run.recheck_request().is_some(), "ambiguous exchange submission can be queried by original client ID");
    std::fs::write(&path, line(&run)).unwrap();
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 100);
    let request = run.recheck_request().unwrap();
    store.request_recheck(&request, "operator", 100).unwrap();
    for i in 0..12 {
        let checked = store.claim_recovery_check(&run.run_id, 100+i*60_000).unwrap().unwrap();
        assert_eq!(checked.recovery_checks, (i+1) as u8);
        assert_eq!(checked.status, OnchainReplenishmentRunStatus::AwaitingSourceFinality);
    }
    let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 720_100);
    let paused = restored.claim_recovery_check(&run.run_id, 720_100).unwrap().unwrap();
    assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
    assert_eq!(paused.transfers[0].client_transfer_id, run.transfers[0].client_transfer_id);
    assert_eq!(paused.transfers.len(), 1);
    assert!(restored.claim_recovery_check(&run.run_id, 780_100).unwrap().is_none());
}

#[test]
fn replenishment_recovery_retains_all_unresolved_runs_and_retired_idempotency_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replenishment.jsonl");
    let retired = fixture("retired", OnchainReplenishmentRunStatus::Completed, 1);
    let mut history = line(&retired);
    let states = [
        OnchainReplenishmentRunStatus::Submitting,
        OnchainReplenishmentRunStatus::AwaitingSourceFinality,
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit,
        OnchainReplenishmentRunStatus::Paused,
        OnchainReplenishmentRunStatus::Failed,
    ];
    for index in 0..129 {
        history.push_str(&line(&fixture(
            &format!("active-{index}"),
            states[index % states.len()],
            index as i64 + 2,
        )));
    }
    std::fs::write(&path, history).unwrap();
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 200);
    assert_eq!(store.runs(1, 200).rows.len(), 129);
    assert!(store.run(&run_id("active-0"), 200).is_some());
    assert!(store.run(&retired.run_id, 200).is_none());
    assert!(matches!(
        store.runs(1, 200).rows[0].status,
        OnchainReplenishmentRunStatus::Submitting
            | OnchainReplenishmentRunStatus::AwaitingSourceFinality
            | OnchainReplenishmentRunStatus::AwaitingDestinationCredit
    ));
    store.insert(retired.plan.clone(), 200).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        store.authorize(&retired.plan.plan_id, "retired", "operator", 201),
        Err(ReplenishmentAuthorizeError::IdempotencyConflict)
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    let new_plan = fixture(
        "fresh",
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit,
        202,
    )
    .plan;
    store.insert(new_plan.clone(), 202).unwrap();
    store
        .authorize(&new_plan.plan_id, "fresh", "operator", 203)
        .unwrap();
    assert_eq!(store.runs(128, 204).rows.len(), 130);
    drop(store);
    let restored = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 210);
    assert_eq!(restored.runs(1, 210).rows.len(), 130);
    assert!(restored.run(&run_id("active-0"), 210).is_some());
    assert_eq!(
        restored.authorize(&retired.plan.plan_id, "retired", "operator", 211),
        Err(ReplenishmentAuthorizeError::IdempotencyConflict)
    );
}

#[test]
fn replenishment_recovery_replay_cannot_change_authorizing_actor() {
    let store = OnchainReplenishmentPlanStore::load_path(None, 10);
    let plan = fixture(
        "scope",
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit,
        10,
    )
    .plan;
    store.insert(plan.clone(), 10).unwrap();
    store
        .authorize(&plan.plan_id, "scope", "operator", 11)
        .unwrap();
    assert!(
        store
            .authorize(&plan.plan_id, "scope", "operator", 12)
            .unwrap()
            .replayed
    );
    assert_eq!(
        store.authorize(&plan.plan_id, "scope", "another-actor", 12),
        Err(ReplenishmentAuthorizeError::IdempotencyConflict)
    );
}

#[test]
fn replenishment_recovery_does_not_skip_damage_or_append_to_a_torn_tail() {
    for malformed in [
        "{bad json}\n",
        "{\"schemaVersion\":2,\"plan\":null,\"run\":null}\n",
        "{\"schemaVersion\":1",
        "{\"schemaVersion\":1,\"plan\":null,\"run\":null}\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let good = fixture(
            "before-damage",
            OnchainReplenishmentRunStatus::AwaitingDestinationCredit,
            10,
        );
        let later = fixture("after-damage", OnchainReplenishmentRunStatus::Completed, 20);
        let content = format!(
            "{}{malformed}{}",
            line(&good),
            if malformed.ends_with('\n') {
                line(&later)
            } else {
                String::new()
            }
        );
        std::fs::write(&path, &content).unwrap();
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 30);
        assert!(store.readiness().unwrap_err().contains("第 2 行"));
        let snapshot = store.runs(128, 30);
        assert_eq!(snapshot.rows.len(), 1);
        assert!(snapshot.recovery_problem.is_some());
        assert!(store.insert(later.plan.clone(), 31).is_err());
        assert!(
            store
                .record_destination_credit(
                    &good.run_id,
                    Decimal::new(125, 1),
                    Some(true),
                    Some(20),
                    "history".to_owned(),
                    32
                )
                .is_err()
        );
        assert_eq!(store.run(&good.run_id, 32).unwrap().status, good.status);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }
}

#[test]
fn replenishment_recovery_requires_a_record_delimiter_and_readable_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");
    let row = fixture(
        "complete-json-without-delimiter",
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit,
        10,
    );
    std::fs::write(&path, line(&row).trim_end()).unwrap();
    let store = OnchainReplenishmentPlanStore::load_path(Some(path), 20);
    assert!(store.readiness().unwrap_err().contains("未完整写入"));
    assert!(store.runs(20, 20).rows.is_empty());
    let store = OnchainReplenishmentPlanStore::load_path(Some(dir.path().to_path_buf()), 20);
    assert!(store.readiness().is_err());
    assert!(store.runs(20, 20).recovery_problem.is_some());
}

#[test]
fn replenishment_recovery_write_failure_stays_blocked_until_clean_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");
    let backup = dir.path().join("original.jsonl");
    let original = fixture(
        "original",
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit,
        10,
    );
    std::fs::write(&path, line(&original)).unwrap();
    let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 20);
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    let plan = fixture(
        "new",
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit,
        21,
    )
    .plan;
    assert!(store.insert(plan.clone(), 21).is_err());
    assert!(store.runs(20, 21).recovery_problem.is_some());
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    assert!(store.insert(plan.clone(), 22).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), line(&original));
    drop(store);
    let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 23);
    restored.readiness().unwrap();
    restored.insert(plan, 24).unwrap();
}
