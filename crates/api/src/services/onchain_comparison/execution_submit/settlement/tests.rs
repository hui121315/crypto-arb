use super::*;
use crate::services::onchain_execution_run_store::{test_checkpoint, OnchainExecutionRunStore};
use shared_types::{
    LiveOrderState, OnchainExecutionLegKind, OnchainExecutionRunStatus, OrderUpdateSource,
};
use trading::{ExecutionLedger, FillLedgerInput};

fn basis() -> OnchainCexSettlementBasis {
    let checkpoint = test_checkpoint();
    seed(
        checkpoint.primary_record.as_ref().unwrap(),
        &checkpoint.primary_plan.instrument_spec,
    )
    .unwrap()
    .basis
}

fn fill(
    id: &str,
    quantity: f64,
    price: f64,
    fee: Option<f64>,
    asset: Option<&str>,
) -> ExecutionLedgerEvent {
    ExecutionLedger::default()
        .record_fill_event(
            &test_checkpoint().primary_record.unwrap(),
            &FillLedgerInput {
                venue_event_id: id.into(),
                quantity,
                price,
                fee_amount: fee,
                fee_currency: asset.map(str::to_owned),
                occurred_at_ms: 2,
            },
            OrderUpdateSource::PrivateWs,
            3,
        )
        .unwrap()
}

#[test]
fn cex_settlement_sums_actual_fills_once_and_ignores_cumulative_snapshots() {
    let a = fill("a", 0.4, 100.0, Some(0.04), Some("USD"));
    let b = fill("b", 0.6, 120.0, Some(0.072), Some("USD"));
    let mut snapshot = fill("snapshot", 1.0, 112.0, Some(999.0), Some("USD"));
    snapshot.event_type = ExecutionLedgerEventType::FillSnapshot;
    let result = reconcile(&basis(), &[a.clone(), snapshot, b, a]);
    assert_eq!(result.status, OnchainCexSettlementStatus::Complete);
    assert_eq!(result.gross_quote_amount.as_deref(), Some("112"));
    assert_eq!(result.debit_amount.as_deref(), Some("1"));
    assert_eq!(result.credit_amount.as_deref(), Some("111.888"));
    assert_eq!(result.fees[0].amount, "0.112");
    assert_eq!(result.fill_event_ids.len(), 2);
}

#[test]
fn cex_settlement_buys_deduct_base_fees_and_add_quote_fees_to_cost() {
    let mut basis = basis();
    basis.side = OrderSide::Buy;
    for (asset, fee, debit, credit) in [
        ("SOL", 0.01, "100", "0.99"),
        ("USD", 0.1, "100.1", "1"),
        ("USD", -0.1, "99.9", "1"),
    ] {
        let mut event = fill("trade", 1.0, 100.0, Some(fee), Some(asset));
        event.order.side = OrderSide::Buy;
        let result = reconcile(&basis, &[event]);
        assert_eq!(result.status, OnchainCexSettlementStatus::Complete);
        assert_eq!(result.debit_amount.as_deref(), Some(debit));
        assert_eq!(result.credit_amount.as_deref(), Some(credit));
    }
}

#[test]
fn cex_settlement_third_asset_and_tiny_fees_are_not_implicitly_usd() {
    for asset in ["BNB", "USDT", "USDC"] {
        let result = reconcile(
            &basis(),
            &[fill("trade", 1.0, 100.0, Some(1e-20), Some(asset))],
        );
        assert_eq!(result.status, OnchainCexSettlementStatus::Complete);
        assert_eq!(result.credit_amount.as_deref(), Some("100"));
        assert_eq!(result.fees[0].asset, asset);
        assert_eq!(result.fees[0].amount, "0.00000000000000000001");
    }
}

#[test]
fn cex_settlement_missing_fee_is_pending_but_explicit_zero_is_known() {
    for (fee, asset) in [(None, Some("USD")), (Some(0.1), None)] {
        let result = reconcile(&basis(), &[fill("trade", 1.0, 100.0, fee, asset)]);
        assert_eq!(result.status, OnchainCexSettlementStatus::PendingFees);
        assert!(result.debit_amount.is_none() && result.credit_amount.is_none());
    }
    let zero = reconcile(&basis(), &[fill("trade", 1.0, 100.0, Some(0.0), None)]);
    assert_eq!(zero.status, OnchainCexSettlementStatus::Complete);
    assert_eq!(zero.credit_amount.as_deref(), Some("100"));
}

#[test]
fn cex_settlement_late_fill_completes_without_using_reference_price() {
    let a = fill("a", 0.4, 100.0, Some(0.04), Some("USD"));
    let partial = reconcile(&basis(), &[a.clone()]);
    assert_eq!(partial.status, OnchainCexSettlementStatus::PendingFills);
    assert!(partial.credit_amount.is_none());
    let b = fill("b", 0.6, 120.0, Some(0.072), Some("USD"));
    let complete = reconcile(&partial.basis, &[a, b]);
    assert_eq!(complete.credit_amount.as_deref(), Some("111.888"));
}

#[test]
fn cex_settlement_rejects_contradictory_identity_quantity_and_replay() {
    let a = fill("a", 1.0, 100.0, Some(0.1), Some("USD"));
    for mutate in [
        |event: &mut ExecutionLedgerEvent| event.order.side = OrderSide::Buy,
        |event: &mut ExecutionLedgerEvent| event.order.exchange = "other".into(),
        |event: &mut ExecutionLedgerEvent| event.order.symbol = "SOL/USDC".into(),
        |event: &mut ExecutionLedgerEvent| {
            if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut event.payload {
                fill.quote_value = 999.0;
            }
        },
    ] {
        let mut bad = a.clone();
        mutate(&mut bad);
        assert_eq!(
            reconcile(&basis(), &[bad]).status,
            OnchainCexSettlementStatus::Invalid
        );
    }
    let another = fill("b", 1.0, 100.0, Some(0.1), Some("USD"));
    assert_eq!(
        reconcile(&basis(), &[a.clone(), another]).status,
        OnchainCexSettlementStatus::Invalid
    );
    let mut conflict = a.clone();
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut conflict.payload {
        fill.fee.as_mut().unwrap().amount = 0.2;
    }
    assert_eq!(
        reconcile(&basis(), &[a, conflict]).status,
        OnchainCexSettlementStatus::Invalid
    );
}

#[test]
fn cex_settlement_seed_requires_terminal_spot_and_exact_market() {
    let checkpoint = test_checkpoint();
    let mut record = checkpoint.primary_record.unwrap();
    let mut spec = checkpoint.primary_plan.instrument_spec;
    assert!(seed(&record, &spec).is_some());
    record.state = LiveOrderState::PartiallyFilled;
    assert!(seed(&record, &spec).is_none());
    record.state = LiveOrderState::Cancelled;
    record.filled_quantity = Some(0.5);
    assert_eq!(seed(&record, &spec).unwrap().basis.confirmed_quantity, 0.5);
    spec.product_type = Some("perpetual".into());
    assert!(seed(&record, &spec).is_none());
    spec.product_type = Some("spot".into());
    spec.native_symbol = "SOL/USDC".into();
    assert!(seed(&record, &spec).is_none());
}

#[test]
fn cex_settlement_run_journal_restores_basis_and_frozen_actual_amounts() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_execution_run_ledger_path =
        Some(dir.path().join("runs.jsonl").to_string_lossy().into());
    let checkpoint = test_checkpoint();
    let mut run = checkpoint.response;
    run.status = OnchainExecutionRunStatus::Completed;
    let mut leg = super::super::order_leg_result(
        1,
        OnchainExecutionLegKind::PrimaryCex,
        checkpoint.primary_record.as_ref().unwrap(),
        &checkpoint.primary_plan.instrument_spec,
    );
    let replay = OnchainExecutionRunStore::load(&config);
    run.legs.push(leg.clone());
    replay.store.append_run(&run).unwrap();
    let restored = OnchainExecutionRunStore::load(&config);
    assert_eq!(restored.runs[0].legs[0].settlement, leg.settlement);
    let saved_basis = &restored.runs[0].legs[0].settlement.as_ref().unwrap().basis;
    leg.settlement = Some(reconcile(
        saved_basis,
        &[fill("trade", 1.0, 100.0, Some(0.1), Some("USD"))],
    ));
    run.legs[0] = leg;
    restored.store.append_run(&run).unwrap();
    let final_replay = OnchainExecutionRunStore::load(&config);
    assert_eq!(
        final_replay.runs[0].legs[0]
            .settlement
            .as_ref()
            .unwrap()
            .credit_amount
            .as_deref(),
        Some("99.9")
    );
    assert!(final_replay.pending.is_empty());
    // Legacy receipts remain readable, without synthesizing a fee of zero.
    let mut legacy = serde_json::to_value(&run.legs[0]).unwrap();
    legacy.as_object_mut().unwrap().remove("settlement");
    let decoded: shared_types::OnchainExecutionLegResult = serde_json::from_value(legacy).unwrap();
    assert!(decoded.settlement.is_none());
}

#[tokio::test]
async fn cex_settlement_recent_runs_rebuilds_late_fees_after_restart_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let ledger_path = dir.path().join("fills.jsonl");
    let run_path = dir.path().join("runs.jsonl");
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = dir.path().to_string_lossy().into();
    config.storage.execution_ledger_path = Some(ledger_path.to_string_lossy().into());
    config.storage.onchain_execution_run_ledger_path = Some(run_path.to_string_lossy().into());
    let checkpoint = test_checkpoint();
    let mut run = checkpoint.response;
    run.status = OnchainExecutionRunStatus::Completed;
    run.legs.push(super::super::order_leg_result(
        1,
        OnchainExecutionLegKind::PrimaryCex,
        checkpoint.primary_record.as_ref().unwrap(),
        &checkpoint.primary_plan.instrument_spec,
    ));
    OnchainExecutionRunStore::load(&config)
        .store
        .append_run(&run)
        .unwrap();
    let event = fill("trade", 1.0, 100.0, None, None);
    std::fs::write(
        &ledger_path,
        format!("{}\n", serde_json::to_string(&event).unwrap()),
    )
    .unwrap();
    let state = AppState::new(config.clone()).await.unwrap();
    let pending = super::super::recent_runs(&state, 20).rows.remove(0);
    assert_eq!(
        pending.legs[0].settlement.as_ref().unwrap().status,
        OnchainCexSettlementStatus::PendingFees
    );
    assert!(state.trading_service().list_orders().is_empty());
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
    drop(state);
    // Only the isolated test ledger is replaced, after its writer is shut down.
    let event = fill("trade", 1.0, 100.0, Some(0.1), Some("USD"));
    std::fs::write(
        &ledger_path,
        format!("{}\n", serde_json::to_string(&event).unwrap()),
    )
    .unwrap();
    let restored = AppState::new(config).await.unwrap();
    let before = std::fs::read(&run_path).unwrap();
    let first = super::super::recent_runs(&restored, 20).rows.remove(0);
    let second = super::super::recent_runs(&restored, 20).rows.remove(0);
    assert_eq!(first, second);
    assert_eq!(first.status, OnchainExecutionRunStatus::Completed);
    assert_eq!(
        first.legs[0]
            .settlement
            .as_ref()
            .unwrap()
            .credit_amount
            .as_deref(),
        Some("99.9")
    );
    assert_eq!(
        std::fs::read(&run_path).unwrap(),
        before,
        "reads must not fsync the run log each time"
    );
    assert!(restored.trading_service().list_orders().is_empty());
    restored
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}
