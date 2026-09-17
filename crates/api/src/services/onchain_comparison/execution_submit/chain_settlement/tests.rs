use super::*;
use crate::services::onchain_comparison::replenishment_credit::{
    rpc_tests::{client, fixture},
    swap::tests::{solana_basis, solana_tx},
};
use crate::services::onchain_execution_run_store::{test_checkpoint, OnchainExecutionRunStore};
use serde_json::json;

fn run() -> OnchainExecutionSubmitResponse {
    let mut checkpoint = test_checkpoint();
    let basis = solana_basis();
    checkpoint.build.wallet_address = basis.wallet;
    checkpoint.build.chain = basis.chain;
    checkpoint.build.input_token = basis.assets.input.address.clone();
    checkpoint.build.output_token = basis.assets.output.address.clone();
    checkpoint.build.input_amount_raw = basis.maximum_input_raw;
    checkpoint.build.minimum_output_amount_raw = basis.minimum_output_raw;
    checkpoint.build.settlement_assets = Some(basis.assets);
    super::super::response(
        super::super::ResponseContext {
            run_id: "accounting",
            build: &checkpoint.build,
            started_at_ms: 1,
        },
        None,
        super::super::RunOutcome {
            status: shared_types::OnchainExecutionRunStatus::Completed,
            chain_transaction_id: Some("signature".into()),
            remaining_exposure_usd: 0.0,
            message: "confirmed".into(),
            problem: None,
        },
    )
}

#[tokio::test]
async fn chain_settlement_rpc_to_visible_run_journal_and_replay_preserves_receipt_without_orders() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = dir.path().to_string_lossy().into();
    config.storage.onchain_execution_run_ledger_path =
        Some(dir.path().join("runs.jsonl").to_string_lossy().into());
    let state = AppState::new(config.clone()).await.unwrap();
    let original = run();
    state
        .onchain_execution_run_store()
        .append_run(&original)
        .unwrap();
    state
        .onchain_execution_runs()
        .insert(original.run_id.clone(), original.clone());
    let previous = pending(&original).unwrap().clone();
    let fixture = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", solana_tx()),
    ])
    .await;
    let mut next = swap::read_on_rpc(&client(), &fixture.url, &previous.basis)
        .await
        .unwrap();
    next.attempts = 1;
    assert!(save(&state, &original.run_id, &previous, next.clone()));
    assert!(
        !save(&state, &original.run_id, &previous, next.clone()),
        "stale response must not overwrite a newer receipt"
    );
    let updated = state
        .onchain_execution_runs()
        .get(&original.run_id)
        .unwrap()
        .value()
        .clone();
    assert!(pending(&updated).is_none());
    assert_eq!(updated.legs[0].chain_settlement.as_ref(), Some(&next));
    let mut rebuilt = original.clone();
    preserve_known(&updated, &mut rebuilt);
    assert_eq!(rebuilt.legs[0].chain_settlement.as_ref(), Some(&next));
    let replay = OnchainExecutionRunStore::load(&config);
    assert_eq!(
        replay.runs[0].legs[0].chain_settlement.as_ref(),
        Some(&next)
    );
    assert!(state.trading_service().list_orders().is_empty());
    assert!(fixture.replies.lock().unwrap().is_empty());
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}

#[test]
fn chain_settlement_worker_is_bounded_and_never_starts_for_unconfirmed_or_wrong_hash() {
    let mut row = run();
    assert!(pending(&row).is_some());
    row.legs[0].status = OnchainExecutionLegStatus::Pending;
    assert!(pending(&row).is_none());
    row.legs[0].status = OnchainExecutionLegStatus::Confirmed;
    row.legs[0].chain_settlement.as_mut().unwrap().attempts = MAX_ATTEMPTS;
    assert!(pending(&row).is_none());
    row.legs[0].chain_settlement.as_mut().unwrap().attempts = 0;
    row.legs[0].transaction_id = Some("other".into());
    assert!(pending(&row).is_none());
    let mut checkpoint = test_checkpoint();
    checkpoint.build.settlement_assets = None;
    assert!(
        seed(&checkpoint.build, "signature").is_none(),
        "legacy rows must not invent token precision"
    );
}

#[test]
fn chain_settlement_actual_refund_and_shortfall_cannot_be_reported_as_flat() {
    let mut row = run();
    let receipt = row.legs[0].chain_settlement.as_mut().unwrap();
    receipt.status = Status::Complete;
    receipt.input_amount_raw = Some("99000000".into());
    receipt.output_amount_raw = Some("105000000".into());
    let cex: shared_types::OnchainExecutionLegResult = serde_json::from_value(json!({"position":1,"kind":"primary_cex",
        "status":"filled","venue":"kraken","symbol":"USDT/USD","orderId":"cex","transactionId":null,
        "filledQuantity":100,"message":"filled","settlement":{"basis":{"orderId":"cex","venue":"kraken","symbol":"USDT/USD",
        "side":"buy","baseAsset":"USDT","quoteAsset":"USD","confirmedQuantity":100},"status":"complete",
        "grossBaseAmount":"100","grossQuoteAmount":"100","debitAmount":"100","creditAmount":"100",
        "fees":[],"fillEventIds":["trade"],"observedAtMs":1,"problem":null}})).unwrap();
    row.legs.insert(0, cex);
    row.cex_order_id = Some("cex".into());
    reconcile_quantity(&mut row, true);
    assert_eq!(row.status, shared_types::OnchainExecutionRunStatus::Exposed);
    assert!(!row.quantity_reconciled);
    assert!(row.problem.as_deref().unwrap().contains("剩余 1 USDT"));
    row.legs[1]
        .chain_settlement
        .as_mut()
        .unwrap()
        .input_amount_raw = Some("100000000".into());
    reconcile_quantity(&mut row, true);
    assert!(row.quantity_reconciled);
    assert_eq!(
        row.status,
        shared_types::OnchainExecutionRunStatus::Completed
    );
    let cex = row.legs[0].settlement.as_mut().unwrap();
    cex.basis.side = shared_types::OrderSide::Sell;
    cex.basis.base_asset = "USDC".into();
    cex.basis.symbol = "USDC/USD".into();
    cex.debit_amount = Some("106".into());
    row.legs[0].symbol = Some("USDC/USD".into());
    reconcile_quantity(&mut row, true);
    assert!(!row.quantity_reconciled);
    assert!(row.problem.as_deref().unwrap().contains("还缺 1 USDC"));
    row.legs[0].settlement.as_mut().unwrap().debit_amount = Some("105".into());
    reconcile_quantity(&mut row, true);
    assert!(row.quantity_reconciled);
    let matched = row;
    for defect in 0..7 {
        let mut row = matched.clone();
        match defect {
            0 => row.cex_order_id = Some("other-order".into()),
            1 => row.legs[0].venue = "other-venue".into(),
            2 => row.chain_transaction_id = Some("other-hash".into()),
            3 => row.legs.push(row.legs[0].clone()),
            4 => row.legs.push(row.legs[1].clone()),
            5 => row.legs[0].settlement.as_mut().unwrap().debit_amount = Some("0".into()),
            _ => {
                row.legs[1]
                    .chain_settlement
                    .as_mut()
                    .unwrap()
                    .output_amount_raw = Some("105.0".into())
            }
        }
        reconcile_quantity(&mut row, true);
        assert!(
            !row.quantity_reconciled,
            "defect {defect} must invalidate prior quantity proof"
        );
    }
}
