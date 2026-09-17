use super::*;
use crate::services::onchain_comparison::replenishment_credit::{
    rpc_tests::{client, fixture},
    swap::{
        self,
        tests::{solana_basis, solana_tx},
    },
};
use crate::services::onchain_execution_run_store::{test_checkpoint, OnchainExecutionRunStore};
use serde_json::json;
use shared_types::{OnchainExecutionLegKind as Kind, OnchainExecutionLegResult, OrderSide};

fn cex(id: &str, base: &str, side: OrderSide, kind: Kind, fee: f64) -> OnchainExecutionLegResult {
    let mut checkpoint = test_checkpoint();
    let mut record = checkpoint.primary_record.take().unwrap();
    record.intent.id = id.into();
    record.intent.exchange = "kraken".into();
    record.intent.symbol = format!("{base}/USD");
    record.intent.side = side;
    record.intent.quantity = 100.0;
    record.identity = shared_types::VenueOrderIdentity::from_intent(&record.intent);
    record.filled_quantity = Some(100.0);
    record.state = shared_types::LiveOrderState::Filled;
    let event = trading::ExecutionLedger::default()
        .record_fill_event(
            &record,
            &trading::FillLedgerInput {
                venue_event_id: format!("fill-{id}"),
                quantity: 100.0,
                price: 1.0,
                fee_amount: Some(fee),
                fee_currency: Some("USD".into()),
                occurred_at_ms: 1,
            },
            shared_types::OrderUpdateSource::PrivateWs,
            2,
        )
        .unwrap();
    let receipt = super::super::settlement::reconcile(
        &shared_types::OnchainCexSettlementBasis {
            order_id: id.into(),
            venue: "kraken".into(),
            symbol: record.intent.symbol.clone(),
            side,
            base_asset: base.into(),
            quote_asset: "USD".into(),
            confirmed_quantity: 100.0,
        },
        &[event],
    );
    assert_eq!(
        receipt.status,
        shared_types::OnchainCexSettlementStatus::Complete
    );
    let mut leg: OnchainExecutionLegResult = serde_json::from_value(json!({
        "position":1, "kind":"primary_cex", "status":"filled", "venue":"kraken",
        "symbol":record.intent.symbol, "orderId":id, "filledQuantity":100, "message":"filled"
    }))
    .unwrap();
    leg.kind = kind;
    leg.settlement = Some(receipt);
    leg
}

fn run() -> OnchainExecutionSubmitResponse {
    serde_json::from_value(json!({"runId":"accounting-full", "buildId":"build", "status":"completed",
        "cexOrderId":"primary", "chainTransactionId":"signature", "estimatedNetProfitUsd":9,
        "remainingExposureUsd":0, "quantityReconciled":false, "message":"confirmed", "startedAtMs":1,"updatedAtMs":2})).unwrap()
}

#[test]
fn approval_allocation_compensated_execution_keeps_actual_approval_fee_and_native_currency() {
    let mut run = run();
    run.status = shared_types::OnchainExecutionRunStatus::Compensated;
    run.chain_transaction_id = None;
    run.compensation_order_id = Some("undo".into());
    run.legs = vec![cex("primary", "USDT", OrderSide::Buy, Kind::PrimaryCex, 0.25),
        cex("undo", "USDT", OrderSide::Sell, Kind::Compensation, 0.1)];
    run.approval_costs = vec![super::super::super::approval_allocation::tests::cost()];
    let flows = flows::collect(&run).unwrap();
    assert_eq!(flows.iter().filter(|f| f.kind == shared_types::OnchainExecutionCashFlowKind::ApprovalFee).count(), 1);
    let net = flows::net(&flows).unwrap();
    assert_eq!(net.iter().find(|a| a.asset == "ETH").unwrap().amount_exact, "-0.000021");
    let now = common::time::now_ms();
    let mut fx = rates(&net, now);
    let eth = fx.iter_mut().find(|r| r.asset == "ETH").unwrap();
    eth.usd_bid = 2000.0;
    eth.usd_ask = 2100.0;
    let value = value_at(&net, fx, now).unwrap();
    assert_eq!(value.net_usd_exact, "-0.3941");
    run.accounting = Some(Accounting { status: Status::Valued, flows, net_assets: net, usd_value: Some(value), problems:vec![] });
    if let Ok(path) = std::env::var("CROSSLINE_APPROVAL_ACCOUNTING_FIXTURE") {
        std::fs::write(path, serde_json::to_vec_pretty(&run).unwrap()).unwrap();
    }
    run.approval_costs.push(run.approval_costs[0].clone());
    assert!(flows::collect(&run).is_err());
}

async fn chain() -> OnchainExecutionLegResult {
    let rpc = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", solana_tx()),
    ])
    .await;
    let receipt = swap::read_on_rpc(&client(), &rpc.url, &solana_basis())
        .await
        .unwrap();
    assert_eq!(
        receipt.status,
        shared_types::OnchainChainSettlementStatus::Complete
    );
    let mut leg: OnchainExecutionLegResult = serde_json::from_value(json!({
        "position":2,"kind":"chain","status":"confirmed","venue":"solana","transactionId":"signature","message":"confirmed"
    })).unwrap();
    leg.chain_settlement = Some(receipt);
    assert!(rpc.replies.lock().unwrap().is_empty());
    leg
}

fn rates(assets: &[OnchainExecutionAssetChange], now: i64) -> Vec<OnchainUsdValuation> {
    assets
        .iter()
        .map(|a| {
            let (bid, ask) = match a.asset.as_str() {
                "USDC" => (0.9, 1.1),
                "SOL" => (90.0, 100.0),
                _ => (1.0, 1.0),
            };
            let mut quote = usd_valuation::fixture(&a.asset, bid, now);
            quote.usd_ask = ask;
            if a.asset == "USD" {
                quote.source = "same_currency".into();
                quote.venue.clear();
            }
            quote
        })
        .collect()
}

#[tokio::test]
async fn execution_accounting_combines_three_legs_native_fees_and_refunds_without_double_charging()
{
    let mut run = run();
    run.legs = vec![
        cex(
            "conversion",
            "USDC",
            OrderSide::Sell,
            Kind::QuoteConversion,
            0.1,
        ),
        cex("primary", "USDT", OrderSide::Buy, Kind::PrimaryCex, 0.25),
        chain().await,
    ];
    let flows = flows::collect(&run).unwrap();
    let net = flows::net(&flows).unwrap();
    let amount = |asset: &str| {
        net.iter()
            .find(|n| n.asset == asset)
            .map(|n| n.amount_exact.as_str())
    };
    assert_eq!(amount("USD"), Some("-0.35"));
    assert_eq!(amount("USDT"), Some("1"));
    assert_eq!(amount("USDC"), Some("5"));
    assert_eq!(amount("SOL"), Some("-0.000014"));
    let value = value_at(&net, rates(&net, 1_000), 1_000).unwrap();
    assert_eq!(value.net_usd_exact, "5.1486");
    run.accounting = Some(Accounting {
        status: Status::Valued,
        flows,
        net_assets: net,
        usd_value: Some(value),
        problems: vec![],
    });
    if let Ok(path) = std::env::var("CROSSLINE_EXECUTION_ACCOUNTING_FIXTURE") {
        std::fs::write(path, serde_json::to_vec_pretty(&run).unwrap()).unwrap();
    }
    let original = run.clone();
    for fault in 0..9 {
        let mut bad = original.clone();
        match fault {
            0 => bad.legs.push(bad.legs[0].clone()),
            1 => {
                bad.legs[1].settlement.as_mut().unwrap().status =
                    shared_types::OnchainCexSettlementStatus::PendingFees
            }
            2 => bad.legs[1].settlement.as_mut().unwrap().debit_amount = Some("100".into()),
            3 => bad.legs[2].chain_settlement.as_mut().unwrap().network_cost = None,
            4 => bad.legs[2].transaction_id = Some("other-hash".into()),
            5 => bad.legs[1].settlement.as_mut().unwrap().basis.venue = "other".into(),
            6 => {
                bad.legs.pop();
            }
            7 => {
                bad.legs[2]
                    .chain_settlement
                    .as_mut()
                    .unwrap()
                    .network_cost
                    .as_mut()
                    .unwrap()
                    .total_fee_exact = Some("9".into())
            }
            _ => {
                bad.legs[2]
                    .chain_settlement
                    .as_mut()
                    .unwrap()
                    .basis
                    .assets
                    .output
                    .symbol = "USD".into()
            }
        }
        assert!(
            flows::collect(&bad).is_err(),
            "fault {fault} must not produce an actual total"
        );
    }
}

#[tokio::test]
async fn replenishment_allocation_execution_receipt_includes_native_transfer_fee_once() {
    let mut run = run();
    run.legs = vec![
        cex(
            "conversion",
            "USDC",
            OrderSide::Sell,
            Kind::QuoteConversion,
            0.1,
        ),
        cex("primary", "USDT", OrderSide::Buy, Kind::PrimaryCex, 0.25),
        chain().await,
    ];
    run.replenishment_costs = vec![super::super::super::replenishment_allocation::test_cost()];
    let flows = flows::collect(&run).unwrap();
    let net = flows::net(&flows).unwrap();
    assert_eq!(
        net.iter().find(|a| a.asset == "USDC").unwrap().amount_exact,
        "4.9"
    );
    assert_eq!(
        flows
            .iter()
            .filter(|f| f.kind == shared_types::OnchainExecutionCashFlowKind::ReplenishmentFee)
            .count(),
        1
    );
    let value = value_at(&net, rates(&net, 1000), 1000).unwrap();
    assert_eq!(value.net_usd_exact, "5.0586");
    run.accounting = Some(Accounting {
        status: Status::Valued,
        flows,
        net_assets: net,
        usd_value: Some(value),
        problems: vec![],
    });
    if let Ok(path) = std::env::var("CROSSLINE_REPLENISHMENT_ACCOUNTING_FIXTURE") {
        std::fs::write(path, serde_json::to_vec_pretty(&run).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn execution_accounting_third_asset_rebate_and_sponsored_gas_are_not_user_debits_twice() {
    let mut run = run();
    let mut primary = cex("primary", "USDT", OrderSide::Buy, Kind::PrimaryCex, -0.25);
    primary
        .settlement
        .as_mut()
        .unwrap()
        .fees
        .push(shared_types::OnchainCexSettlementFee {
            asset: "BNB".into(),
            amount: "0.01".into(),
        });
    let mut chain = chain().await;
    chain
        .chain_settlement
        .as_mut()
        .unwrap()
        .network_cost
        .as_mut()
        .unwrap()
        .payer = "sponsor".into();
    run.legs = vec![primary, chain];
    let net = flows::net(&flows::collect(&run).unwrap()).unwrap();
    assert!(net
        .iter()
        .any(|a| a.asset == "USD" && a.amount_exact == "-99.75"));
    assert!(net
        .iter()
        .any(|a| a.asset == "BNB" && a.amount_exact == "-0.01"));
    assert!(net
        .iter()
        .any(|a| a.asset == "SOL" && a.amount_exact == "-0.000009"));
}

#[test]
fn execution_accounting_valuation_rejects_stale_future_wrong_pair_and_implicit_pegs() {
    let assets = vec![OnchainExecutionAssetChange {
        asset: "USDC".into(),
        amount_exact: "-10".into(),
    }];
    let quote = rates(&assets, 1_000)[0].clone();
    assert_eq!(
        value_at(&assets, vec![quote.clone()], 1_000)
            .unwrap()
            .net_usd_exact,
        "-11"
    );
    assert!(value_at(&assets, vec![], 1_000).is_err());
    for fault in 0..5 {
        let mut bad = quote.clone();
        match fault {
            0 => bad.source = "same_currency".into(),
            1 => bad.symbol = "USDC/USDT".into(),
            2 => bad.observed_at_ms = 1_001,
            3 => bad.observed_at_ms = -30_000,
            _ => bad.asset = "USDT".into(),
        }
        assert!(value_at(&assets, vec![bad], 1_000).is_err());
    }
}

#[tokio::test]
async fn execution_accounting_persists_changed_results_once_and_replays_without_repricing_or_orders(
) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = dir.path().to_string_lossy().into();
    let path = dir.path().join("runs.jsonl");
    config.storage.onchain_execution_run_ledger_path = Some(path.to_string_lossy().into());
    let state = AppState::new(config.clone()).await.unwrap();
    let mut row = run();
    row.status = shared_types::OnchainExecutionRunStatus::Compensated;
    row.chain_transaction_id = None;
    row.compensation_order_id = Some("reverse".into());
    row.legs = vec![
        cex("primary", "USDT", OrderSide::Buy, Kind::PrimaryCex, 0.25),
        cex("reverse", "USDT", OrderSide::Sell, Kind::Compensation, 0.25),
    ];
    state
        .onchain_execution_run_store()
        .append_run(&row)
        .unwrap();
    state
        .onchain_execution_runs()
        .insert(row.run_id.clone(), row.clone());
    let updated = refresh_one(&state, &row.run_id).unwrap();
    let accounting = updated.accounting.as_ref().unwrap();
    assert_eq!(accounting.status, Status::Valued);
    assert_eq!(accounting.usd_value.as_ref().unwrap().net_usd_exact, "-0.5");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(refresh_one(&state, &row.run_id).unwrap(), updated);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "unchanged polling must not keep appending the journal"
    );
    let replay = OnchainExecutionRunStore::load(&config);
    assert_eq!(replay.runs[0].accounting, updated.accounting);
    let mut historical = updated.clone();
    historical.legs[1].settlement.as_mut().unwrap().fees.push(
        shared_types::OnchainCexSettlementFee {
            asset: "USDC".into(),
            amount: "1".into(),
        },
    );
    let flows = flows::collect(&historical).unwrap();
    let net = flows::net(&flows).unwrap();
    let value = value_at(&net, rates(&net, 1_000), 1_000).unwrap();
    assert_eq!(value.net_usd_exact, "-1.6");
    historical.accounting = Some(Accounting {
        status: Status::Valued,
        flows,
        net_assets: net,
        usd_value: Some(value.clone()),
        problems: vec![],
    });
    state
        .onchain_execution_run_store()
        .append_run(&historical)
        .unwrap();
    state
        .onchain_execution_runs()
        .insert(historical.run_id.clone(), historical.clone());
    let unchanged = refresh_one(&state, &historical.run_id).unwrap();
    assert_eq!(
        unchanged.accounting.as_ref().unwrap().usd_value.as_ref(),
        Some(&value),
        "restart must preserve the saved historical quote without requiring a current quote"
    );
    let interrupted = super::super::interrupted_response(unchanged, "write failed".into());
    assert!(!interrupted.quantity_reconciled);
    assert_eq!(
        interrupted.accounting.as_ref().unwrap().status,
        Status::PendingReceipts
    );
    assert!(interrupted.accounting.unwrap().usd_value.is_none());
    historical.chain_transaction_id = Some("missing-chain-receipt".into());
    state
        .onchain_execution_runs()
        .insert(historical.run_id.clone(), historical);
    let before_read = std::fs::read(&path).unwrap();
    let projected = super::super::recent_runs(&state, 20).rows.remove(0);
    assert_eq!(
        projected.accounting.as_ref().unwrap().status,
        Status::PendingReceipts
    );
    assert!(projected.accounting.unwrap().usd_value.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), before_read);
    assert!(state.trading_service().list_orders().is_empty());
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}
