use super::*;
use serde_json::json;

fn fixture() -> OnchainReplenishmentRun {
    let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    run.status = OnchainReplenishmentRunStatus::AwaitingSourceFinality;
    run.plan.legs[0].direction = OnchainTransferDirection::WithdrawToChain;
    run.plan.legs[0].economics.fee_amount_exact = Some("0.1".into());
    let transfer = &mut run.transfers[0];
    transfer.status = OnchainReplenishmentTransferStatus::Submitted;
    transfer.transaction_id = None;
    transfer.credited_amount_exact = None;
    transfer.withdrawal_unlocked = None;
    run
}

fn store(
    run: &OnchainReplenishmentRun,
) -> (
    tempfile::TempDir,
    common::config::AppConfig,
    OnchainReplenishmentPlanStore,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.jsonl");
    std::fs::write(&path, format!("{}\n", json!({"schemaVersion":1,"run":run}))).unwrap();
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let store = OnchainReplenishmentPlanStore::load(&config);
    (dir, config, store)
}

fn pending(run: &OnchainReplenishmentRun, now: i64) -> exchange::WithdrawalStatusEvidence {
    let request = withdrawal_status_request(run).unwrap();
    exchange::WithdrawalStatusEvidence {
        venue: request.venue,
        provider_withdrawal_id: request.provider_withdrawal_id.unwrap(),
        client_withdrawal_id: request.client_withdrawal_id,
        currency: request.currency,
        network: request.network,
        address: request.address,
        amount: Decimal::new(125, 1),
        transaction_fee: Decimal::new(1, 1),
        status: exchange::WithdrawalStatus::Pending,
        transaction_id: None,
        confirmations: None,
        checked_at_ms: now,
        source_url: "fixture withdrawal history".into(),
        problem: None,
    }
}

#[test]
fn replenishment_wait_pending_or_query_failure_pauses_at_deadline_but_late_success_advances() {
    for failure in [false, true] {
        let run = fixture();
        let (_dir, config, store) = store(&run);
        let deadline = run.automatic_wait_deadline_ms().unwrap();
        let first = record_withdrawal_check(
            &store,
            &run,
            Ok(Some(pending(&run, deadline - 1))),
            deadline - 1,
        )
        .unwrap();
        assert_eq!(
            first.status,
            OnchainReplenishmentRunStatus::AwaitingSourceFinality
        );
        assert_eq!(
            first.automatic_wait_deadline_ms(),
            Some(deadline),
            "polls never extend the original window"
        );
        let outcome = if failure {
            Err(exchange::ExchangeError::Api {
                exchange: "binance".into(),
                code: "fixture".into(),
                message: "transport unavailable".into(),
            })
        } else {
            Ok(Some(pending(&first, deadline)))
        };
        let paused = record_withdrawal_check(&store, &first, outcome, deadline).unwrap();
        assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
        assert_ne!(paused.status, OnchainReplenishmentRunStatus::Failed);
        assert!(paused.problem.as_ref().unwrap().contains("自动核验窗口"));
        assert_eq!(paused.transfers.len(), 1);
        assert_eq!(
            paused.transfers[0].provider_transfer_id,
            run.transfers[0].provider_transfer_id
        );
        assert_eq!(
            OnchainReplenishmentPlanStore::load(&config)
                .run(&run.run_id, deadline + 1)
                .unwrap()
                .transfers,
            paused.transfers
        );
    }
    let run = fixture();
    let (_dir, _, store) = store(&run);
    let now = run.automatic_wait_deadline_ms().unwrap() + 60_000;
    let mut complete = pending(&run, now);
    complete.status = exchange::WithdrawalStatus::Completed;
    complete.transaction_id = Some("confirmed-hash".into());
    let completed = record_withdrawal_check(&store, &run, Ok(Some(complete)), now).unwrap();
    assert_eq!(
        completed.status,
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit,
        "a proven late completion must not be thrown away as a timeout"
    );
}

#[test]
fn replenishment_wait_manual_recovery_ignores_old_deadline_but_still_stops_after_twelve_rounds() {
    let run = fixture();
    let (_dir, config, store) = store(&run);
    let now = run.automatic_wait_deadline_ms().unwrap() + 60_000;
    let paused = record_withdrawal_check(&store, &run, Ok(None), now).unwrap();
    let recovered = store
        .request_recheck(&paused.recheck_request().unwrap(), "test", now + 1)
        .unwrap();
    assert_eq!(recovered.automatic_wait_deadline_ms(), None);
    for index in 0..shared_types::ONCHAIN_REPLENISHMENT_RECOVERY_LIMIT {
        let at = now + 1 + i64::from(index) * 60_000;
        let claimed = store
            .claim_recovery_check(&run.run_id, at)
            .unwrap()
            .unwrap();
        let waiting = record_withdrawal_check(&store, &claimed, Ok(None), at).unwrap();
        assert_eq!(
            waiting.status,
            OnchainReplenishmentRunStatus::AwaitingSourceFinality
        );
        assert_eq!(waiting.recovery_checks, index + 1);
        assert_eq!(
            waiting.transfers[0].submission_attempted_at_ms,
            run.transfers[0].submission_attempted_at_ms
        );
        assert!(store
            .claim_recovery_check(&run.run_id, at + 1)
            .unwrap()
            .is_none());
    }
    let restored = OnchainReplenishmentPlanStore::load(&config);
    let paused = restored
        .claim_recovery_check(&run.run_id, now + 1 + 12 * 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
    assert!(replenishment_message(&paused).contains("只读核验 12/12 轮"));
    assert_eq!(paused.transfers.len(), 1);
    assert!(restored
        .claim_recovery_check(&run.run_id, now + 1 + 13 * 60_000)
        .unwrap()
        .is_none());
}

#[test]
fn replenishment_wait_destination_timeout_retains_source_finality_and_recovers_destination_only() {
    let mut run = fixture();
    run.status = OnchainReplenishmentRunStatus::AwaitingDestinationCredit;
    run.transfers[0].status = OnchainReplenishmentTransferStatus::SourceCompleted;
    run.transfers[0].transaction_id = Some("known-chain-hash".into());
    let (_dir, config, store) = store(&run);
    let now = run.automatic_wait_deadline_ms().unwrap();
    let paused = record_destination_pending(
        &store,
        &run,
        Some(1),
        "fixture RPC",
        "waiting for confirmations".into(),
        now,
    )
    .unwrap();
    assert_eq!(
        paused.transfers[0].status,
        OnchainReplenishmentTransferStatus::SourceCompleted
    );
    let restored = OnchainReplenishmentPlanStore::load(&config);
    let recovering = restored
        .request_recheck(&paused.recheck_request().unwrap(), "test", now + 1)
        .unwrap();
    assert_eq!(
        recovering.status,
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit
    );
    let claimed = restored
        .claim_recovery_check(&run.run_id, now + 1)
        .unwrap()
        .unwrap();
    let waiting = record_destination_pending(
        &restored,
        &claimed,
        Some(2),
        "fixture RPC",
        "indexing".into(),
        now + 1,
    )
    .unwrap();
    assert_eq!(
        waiting.status,
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit
    );
    let completed = restored
        .record_destination_credit(
            &run.run_id,
            Decimal::new(125, 1),
            None,
            Some(10),
            "fixture chain receipt".into(),
            now + 2,
        )
        .unwrap();
    assert_eq!(completed.status, OnchainReplenishmentRunStatus::Completed);
    assert!(completed.read_only_recovery);
    assert_eq!(
        completed.transfers[0].transaction_id,
        run.transfers[0].transaction_id
    );
}

#[test]
fn replenishment_wait_locked_credit_preserves_balance_and_rechecks_unlock_without_retransferring() {
    let mut run = fixture();
    run.status = OnchainReplenishmentRunStatus::AwaitingDestinationCredit;
    run.plan.legs[0].direction = OnchainTransferDirection::DepositToCex;
    run.transfers[0].status = OnchainReplenishmentTransferStatus::SourceCompleted;
    run.transfers[0].transaction_id = Some("deposit-hash".into());
    let (_dir, config, store) = store(&run);
    let now = run.automatic_wait_deadline_ms().unwrap();
    let mut evidence = exchange::DepositStatusEvidence {
        venue: "binance".into(),
        currency: "USDC".into(),
        network: "SOL".into(),
        address: "SolanaDepositAddress".into(),
        tag: None,
        amount: Decimal::new(125, 1),
        deposit_fee: Some(Decimal::ZERO),
        status: DepositStatus::CreditedLocked,
        transaction_id: "deposit-hash".into(),
        confirmations: Some(2),
        checked_at_ms: now,
        source_url: "fixture deposit history".into(),
        problem: None,
    };
    let paused = record_cex_credit(&store, &run, &evidence, now).unwrap();
    assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
    assert_eq!(
        paused.transfers[0].status,
        OnchainReplenishmentTransferStatus::DestinationCredited
    );
    assert_eq!(
        paused.transfers[0].credited_amount_exact.as_deref(),
        Some("12.5")
    );
    let restored = OnchainReplenishmentPlanStore::load(&config);
    let recovered = restored
        .request_recheck(&paused.recheck_request().unwrap(), "test", now + 1)
        .unwrap();
    assert_eq!(
        recovered.status,
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit
    );
    assert_eq!(recovered.transfers[0].withdrawal_unlocked, Some(false));
    for index in 0..12 {
        let at = now + 1 + index * 60_000;
        let claimed = restored
            .claim_recovery_check(&run.run_id, at)
            .unwrap()
            .unwrap();
        evidence.checked_at_ms = at;
        let waiting = record_cex_credit(&restored, &claimed, &evidence, at).unwrap();
        assert_eq!(
            waiting.status,
            OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        );
    }
    let paused = restored
        .claim_recovery_check(&run.run_id, now + 1 + 12 * 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(
        paused.transfers[0].status,
        OnchainReplenishmentTransferStatus::DestinationCredited
    );
    let rechecking = restored
        .request_recheck(
            &paused.recheck_request().unwrap(),
            "test",
            now + 2 + 12 * 60_000,
        )
        .unwrap();
    evidence.status = DepositStatus::Completed;
    evidence.checked_at_ms = now + 3 + 12 * 60_000;
    let completed =
        record_cex_credit(&restored, &rechecking, &evidence, evidence.checked_at_ms).unwrap();
    assert_eq!(completed.status, OnchainReplenishmentRunStatus::Completed);
    assert_eq!(completed.transfers.len(), 1);
    assert_eq!(completed.transfers[0].withdrawal_unlocked, Some(true));
    assert!(completed.recheck_request().is_none());
}

#[test]
fn replenishment_wait_chain_deadline_and_manual_recovery_have_distinct_limits() {
    let mut run = fixture();
    run.plan.legs[0].direction = OnchainTransferDirection::DepositToCex;
    let start = run.transfers[0].submission_attempted_at_ms;
    assert_eq!(
        run.automatic_wait_deadline_ms(),
        Some(start + ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS)
    );
    assert!(!expired(
        &run,
        ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS,
        start + ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS - 1
    ));
    assert!(expired(
        &run,
        ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS,
        start + ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS
    ));
    run.read_only_recovery = true;
    assert!(!expired(
        &run,
        ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS,
        start + 10 * ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS
    ));
    assert!(run.automatic_wait_deadline_ms().is_none());
}

#[tokio::test]
async fn replenishment_wait_kraken_pending_http_receipt_pauses_then_allows_a_new_read_only_check() {
    use super::super::deposit_credit_tests::{adapter, history_server};
    let mut run = fixture();
    let now = common::time::now_ms();
    run.plan.legs[0].venue = "kraken".into();
    run.plan.legs[0].network_evidence.network = Some("3e7f8072-cc6d-4394-982a-5f4ca6ab27dd".into());
    run.transfers[0].submission_attempted_at_ms =
        now - shared_types::ONCHAIN_REPLENISHMENT_SOURCE_WAIT_MS;
    let request = withdrawal_status_request(&run).unwrap();
    let body = json!({"withdrawals":[{
        "withdrawal_id":request.provider_withdrawal_id,"method_id":request.network,"status":"pending",
        "amount":{"asset":{"class":"currency","name":"USDC"},"amount":"12.5"},
        "fee":{"asset":{"class":"currency","name":"USDC"},"amount":"0.1"},
        "address_id":"ABR6SXP-SF6CY-VJMONY",
        "create_time":chrono::DateTime::from_timestamp_millis(request.submitted_at_ms).unwrap().to_rfc3339()
    }]});
    let server = history_server("kraken", body, None).await;
    let adapter = adapter("kraken", &server.url);
    let (_dir, config, store) = store(&run);
    let result = adapter.withdrawal_status(&request).await;
    assert_eq!(
        result.as_ref().unwrap().as_ref().unwrap().status,
        exchange::WithdrawalStatus::Pending
    );
    let paused = record_withdrawal_check(&store, &run, result, common::time::now_ms()).unwrap();
    assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
    let restored = OnchainReplenishmentPlanStore::load(&config);
    restored
        .request_recheck(
            &paused.recheck_request().unwrap(),
            "test",
            common::time::now_ms(),
        )
        .unwrap();
    let claimed = restored
        .claim_recovery_check(&run.run_id, common::time::now_ms())
        .unwrap()
        .unwrap();
    let result = adapter
        .withdrawal_status(&withdrawal_status_request(&claimed).unwrap())
        .await;
    let waiting =
        record_withdrawal_check(&restored, &claimed, result, common::time::now_ms()).unwrap();
    assert_eq!(
        waiting.status,
        OnchainReplenishmentRunStatus::AwaitingSourceFinality
    );
    assert_eq!(waiting.recovery_checks, 1);
    assert_eq!(waiting.transfers.len(), 1);
    assert_eq!(server.queries.lock().unwrap().len(), 2);
}
