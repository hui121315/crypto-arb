use super::deposit_credit_tests::{adapter, history_server};
use super::*;
use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;
use serde_json::json;

fn ledger(
    venue: &str,
    now: i64,
) -> (
    tempfile::TempDir,
    common::config::AppConfig,
    OnchainReplenishmentRun,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replenishment.jsonl");
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    run.status = OnchainReplenishmentRunStatus::AwaitingSourceFinality;
    run.updated_at_ms = now;
    run.plan.legs[0].venue = venue.into();
    if venue == "kraken" {
        run.plan.legs[0].network_evidence.network = Some("3e7f8072-cc6d-4394-982a-5f4ca6ab27dd".into());
    }
    run.plan.legs[0].direction = OnchainTransferDirection::WithdrawToChain;
    run.plan.legs[0].economics.fee_amount_exact = Some("0.1".into());
    run.transfers[0].status = OnchainReplenishmentTransferStatus::Submitted;
    run.transfers[0].transaction_id = None;
    run.transfers[0].provider_transfer_id = Some("provider-withdrawal".into());
    run.transfers[0].credited_amount_exact = None;
    run.transfers[0].withdrawal_unlocked = None;
    run.transfers[0].submission_attempted_at_ms = now;
    run.transfers[0].last_checked_at_ms = None;
    std::fs::write(path, format!("{}\n", json!({"schemaVersion":1,"run":run}))).unwrap();
    (dir, config, run)
}

async fn history(
    venue: &str,
    fee: &str,
    run: &OnchainReplenishmentRun,
) -> (
    super::deposit_credit_tests::HistoryServer,
    exchange::WithdrawalStatusEvidence,
) {
    let request = withdrawal_status_request(run).unwrap();
    let (path, body) = if venue == "binance" {
        (
            "/sapi/v1/capital/withdraw/history",
            json!([{
                "id":"provider-withdrawal","withdrawOrderId":request.client_withdrawal_id,
                "coin":"USDC","network":"SOL","address":request.address,"amount":"12.5",
                "transactionFee":fee,"status":6,"confirmNo":3,"txId":"tx-withdrawal"
            }]),
        )
    } else if venue == "bybit" {
        (
            "/v5/asset/withdraw/query-record",
            json!({"retCode":0,"retMsg":"OK","result":{"rows":[{
                "withdrawId":"provider-withdrawal","txID":"tx-withdrawal",
                "coin":"USDC","chain":"SOL","amount":"12.5","withdrawFee":fee,
                "status":"success","toAddress":request.address,"tag":"","withdrawType":0,
                "createTime":request.submitted_at_ms.to_string(),"tax":"0"
            }],"nextPageCursor":""}}),
        )
    } else if venue == "kraken" {
        ("/funding/v1/withdrawals", json!({"withdrawals":[{
            "withdrawal_id":"provider-withdrawal", "method_id":request.network,"status":"success",
            "amount":{"asset":{"class":"currency","name":"USDC"},"amount":"12.5"},
            "fee":{"asset":{"class":"currency","name":"USDC"},"amount":fee},
            "address_id":"ABR6SXP-SF6CY-VJMONY", "onchain_transaction":"tx-withdrawal",
            "create_time":chrono::DateTime::from_timestamp_millis(request.submitted_at_ms).unwrap().to_rfc3339()
        }]}))
    } else {
        (
            "/api/v3/account/withdrawal-records",
            json!({"code":"00000","msg":"success","data":[{
                "orderId":"provider-withdrawal","clientOid":request.client_withdrawal_id,
                "recordId":"tx-withdrawal","coin":"USDC","type":"withdraw","dest":"on_chain",
                "size":"12.5","status":"success","toAddress":request.address,"chain":"SOL",
                "fee":fee,"confirm":"3"
            }]}),
        )
    };
    let server = history_server(venue, body, Some(path)).await;
    let evidence = adapter(venue, &server.url)
        .withdrawal_status(&request)
        .await
        .unwrap()
        .unwrap();
    (server, evidence)
}

#[tokio::test]
async fn replenishment_recheck_http_history_recovers_original_id_without_resubmitting() {
    for venue in ["binance", "bitget", "bybit", "kraken"] {
        let now = common::time::now_ms();
        let (_dir, config, run) = ledger(venue, now);
        let store = OnchainReplenishmentPlanStore::load(&config);
        let paused = store.pause_submission(&run.run_id, "submission response was lost".into(), now).unwrap();
        let request = paused.recheck_request().unwrap();
        let recovered = store.request_recheck(&request, "test", now).unwrap();
        let read = store.claim_recovery_check(&run.run_id, now).unwrap().unwrap();
        let original = withdrawal_status_request(&run).unwrap();
        let retried = withdrawal_status_request(&read).unwrap();
        assert_eq!(retried.client_withdrawal_id, original.client_withdrawal_id);
        assert_eq!(retried.provider_withdrawal_id, original.provider_withdrawal_id);
        assert_eq!(retried.submitted_at_ms, original.submitted_at_ms);
        let (server, evidence) = history(venue, "0.1", &recovered).await;
        let query = server.queries.lock().unwrap().clone();
        assert_eq!(query.len(), 1);
        assert!(query[0].contains(match venue {
            "binance" => "withdrawOrderId=test-transfer",
            "bybit" => "withdrawID=provider-withdrawal",
            "kraken" => "scope%5Bmethod_id%5D=3e7f8072-cc6d-4394-982a-5f4ca6ab27dd",
            _ => "clientOid=test-transfer",
        }));
        let confirmed = store.record_withdrawal_status(&run.run_id, evidence, now).unwrap();
        assert_eq!(confirmed.status, OnchainReplenishmentRunStatus::AwaitingDestinationCredit);
        assert_eq!(confirmed.transfers[0].transaction_id.as_deref(), Some("tx-withdrawal"));
        assert_eq!(confirmed.transfers[0].withdrawal_cost.as_ref().unwrap().fee_exact, "0.1");
        let restored = OnchainReplenishmentPlanStore::load(&config);
        assert!(restored.run(&run.run_id, now).unwrap().read_only_recovery);
        let credited = restored.record_destination_credit(&run.run_id, Decimal::new(125,1), Some(true), Some(40), "fixture exact wallet receipt".into(), now+1).unwrap();
        assert_eq!(credited.status, OnchainReplenishmentRunStatus::Completed);
        assert_eq!(credited.transfers.len(), 1);
        assert!(restored.claim_submission(&run.run_id, "test", run.plan.clone(), 0, &restored.submission_snapshot(), now+2).is_err());
    }
}

#[tokio::test]
async fn withdrawal_cost_http_receipt_is_atomic_durable_and_cannot_hide_an_over_budget_fee() {
    for venue in ["binance", "bitget", "bybit", "kraken"] {
        for fee in ["0", "0.1", "0.2"] {
            let now = common::time::now_ms();
            let (_dir, config, run) = ledger(venue, now);
            let (server, evidence) = history(venue, fee, &run).await;
            assert_eq!(evidence.transaction_fee, fee.parse::<Decimal>().unwrap());
            let checked = evidence.checked_at_ms;
            let store = OnchainReplenishmentPlanStore::load(&config);
            let updated = store
                .record_withdrawal_status(&run.run_id, evidence, checked - 1_000)
                .unwrap();
            assert_eq!(updated.updated_at_ms, checked);
            assert_eq!(updated.transfers[0].last_checked_at_ms, Some(checked));
            assert_eq!(
                updated.status,
                OnchainReplenishmentRunStatus::AwaitingDestinationCredit
            );
            let cost = updated.transfers[0].withdrawal_cost.as_ref().unwrap();
            assert!(cost.confirmed);
            assert_eq!(cost.asset, "USDC");
            assert_eq!(cost.fee_exact, fee);
            assert_eq!(cost.reported_amount_exact, "12.5");
            assert!(cost.source.starts_with(&server.url));
            assert!(replenishment_message(&updated).contains(&format!("实扣提币费 {fee} USDC")));
            if fee == "0.2" {
                assert!(updated.problem.as_deref().unwrap().contains("超出计划"));
                assert_eq!(
                    validate_completed_withdrawal_fees(&updated)
                        .unwrap_err()
                        .code(),
                    "ONCHAIN_REPLENISHMENT_WITHDRAWAL_FEE_EXCEEDED"
                );
            } else {
                assert!(validate_completed_withdrawal_fees(&updated).is_ok());
            }
            let bytes = std::fs::read_to_string(
                config
                    .storage
                    .onchain_replenishment_ledger_path
                    .as_ref()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                bytes.lines().count(),
                2,
                "status and cost must share one append"
            );
            let restored = OnchainReplenishmentPlanStore::load(&config)
                .run(&run.run_id, checked + 1)
                .unwrap();
            assert_eq!(restored.transfers, updated.transfers);
            assert_eq!(server.queries.lock().unwrap().len(), 1);
        }
    }
}

#[test]
fn bybit_withdrawal_recovery_requires_persisted_provider_identity() {
    let now = common::time::now_ms();
    let (_dir, config, run) = ledger("bybit", now);
    let store = OnchainReplenishmentPlanStore::load(&config);
    let paused = store.pause_submission(&run.run_id,"awaiting receipt".into(),now).unwrap();
    assert!(paused.recheck_request().is_some());
    let mut no_ack = paused;
    no_ack.transfers[0].provider_transfer_id = None;
    assert!(no_ack.recheck_request().is_none());
    no_ack.plan.legs[0].venue = "kraken".into();
    assert!(no_ack.recheck_request().is_none(), "Kraken also requires its provider receipt");
    no_ack.plan.legs[0].venue = "binance".into();
    assert!(no_ack.recheck_request().is_some(),"Binance can query by its original client withdrawal ID");
}

#[test]
fn kraken_withdrawal_inconsistent_ack_keeps_receipt_for_read_only_recovery() {
    let now = common::time::now_ms();
    let (_dir, config, mut run) = ledger("kraken", now);
    run.status = OnchainReplenishmentRunStatus::Submitting;
    run.transfers[0].provider_transfer_id = None;
    run.transfers[0].status = OnchainReplenishmentTransferStatus::SubmissionClaimed;
    std::fs::write(config.storage.onchain_replenishment_ledger_path.as_ref().unwrap(),
        format!("{}\n", json!({"schemaVersion":1,"run":run}))).unwrap();
    let store = OnchainReplenishmentPlanStore::load(&config);
    let paused = record_withdrawal_submission(&store, &run.run_id, exchange::WithdrawalSubmission {
        venue:"kraken".into(), provider_withdrawal_id:"known-kraken-receipt".into(),
        client_withdrawal_id:run.transfers[0].client_transfer_id.clone(), submitted_at_ms:now,
        source_url:"Kraken Funding fixture".into(), problem:Some("回执金额不一致，保留编号".into()),
    }, now + 1).unwrap();
    assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
    assert!(paused.recheck_request().is_some());
    let restored = OnchainReplenishmentPlanStore::load(&config).run(&run.run_id, now + 2).unwrap();
    assert_eq!(restored.transfers[0].provider_transfer_id.as_deref(), Some("known-kraken-receipt"));
    assert!(restored.problem.as_deref().unwrap().contains("保留编号"));
    assert_eq!(restored.transfers.len(), 1);
}

#[tokio::test]
async fn withdrawal_cost_pending_is_not_charged_and_confirmed_receipt_cannot_regress() {
    let (_dir, config, run) = ledger("binance", common::time::now_ms());
    let (_server, mut evidence) = history("binance", "0.1", &run).await;
    let store = OnchainReplenishmentPlanStore::load(&config);
    let mut pending = evidence.clone();
    pending.status = exchange::WithdrawalStatus::Pending;
    pending.transaction_id = None;
    let waiting = store
        .record_withdrawal_status(&run.run_id, pending.clone(), pending.checked_at_ms)
        .unwrap();
    assert!(
        !waiting.transfers[0]
            .withdrawal_cost
            .as_ref()
            .unwrap()
            .confirmed
    );
    assert!(replenishment_message(&waiting).contains("提币费暂报"));
    assert!(validate_completed_withdrawal_fees(&waiting).is_err());
    evidence.checked_at_ms += 1;
    let confirmed = store
        .record_withdrawal_status(&run.run_id, evidence.clone(), evidence.checked_at_ms)
        .unwrap();
    let value = super::super::replenishment_costs::value_withdrawal_fee(
        confirmed.transfers[0].withdrawal_cost.as_ref().unwrap(),
        Some(super::super::usd_valuation::fixture("USDC", 0.98, evidence.checked_at_ms)),
        evidence.checked_at_ms,
    ).unwrap();
    let confirmed = store.record_withdrawal_cost_valuation(&run.run_id, 0, value, evidence.checked_at_ms).unwrap();
    assert!(replenishment_message(&confirmed).contains("提币费折算 $0.098"));
    let bytes = std::fs::read(
        config
            .storage
            .onchain_replenishment_ledger_path
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    for variant in [
        "pending", "fee", "coin", "client", "provider", "hash", "negative", "old",
    ] {
        let mut invalid = evidence.clone();
        invalid.checked_at_ms += 1;
        match variant {
            "pending" => invalid.status = exchange::WithdrawalStatus::Pending,
            "fee" => invalid.transaction_fee = Decimal::new(2, 1),
            "coin" => invalid.currency = "OTHER".into(),
            "client" => invalid.client_withdrawal_id = "other-transfer".into(),
            "provider" => invalid.provider_withdrawal_id = "other-withdrawal".into(),
            "hash" => invalid.transaction_id = Some("other-hash".into()),
            "negative" => invalid.transaction_fee = Decimal::NEGATIVE_ONE,
            "old" => invalid.checked_at_ms = 1,
            _ => unreachable!(),
        }
        assert!(
            store
                .record_withdrawal_status(&run.run_id, invalid.clone(), invalid.checked_at_ms)
                .is_err(),
            "{variant}"
        );
    }
    assert_eq!(
        std::fs::read(
            config
                .storage
                .onchain_replenishment_ledger_path
                .as_ref()
                .unwrap()
        )
        .unwrap(),
        bytes
    );
    assert_eq!(
        store
            .run(&run.run_id, evidence.checked_at_ms + 2)
            .unwrap()
            .transfers,
        confirmed.transfers
    );
    evidence.transaction_id = None;
    evidence.checked_at_ms += 2;
    let refreshed = store
        .record_withdrawal_status(&run.run_id, evidence.clone(), evidence.checked_at_ms)
        .unwrap();
    assert_eq!(
        refreshed.transfers[0].transaction_id.as_deref(),
        Some("tx-withdrawal")
    );
    assert_eq!(refreshed.transfers[0].withdrawal_cost, confirmed.transfers[0].withdrawal_cost,
        "repeated official receipt must retain its first confirmed fee and saved FX valuation");
    assert_eq!(OnchainReplenishmentPlanStore::load(&config).run(&run.run_id, evidence.checked_at_ms + 1).unwrap().transfers,
        refreshed.transfers);
}

#[test]
fn withdrawal_cost_legacy_missing_receipt_is_unknown_not_free() {
    let (_dir, _config, run) = ledger("binance", common::time::now_ms());
    assert!(run.transfers[0].withdrawal_cost.is_none());
    assert_eq!(
        validate_completed_withdrawal_fees(&run).unwrap_err().code(),
        "ONCHAIN_REPLENISHMENT_WITHDRAWAL_FEE_UNPROVEN"
    );
}
