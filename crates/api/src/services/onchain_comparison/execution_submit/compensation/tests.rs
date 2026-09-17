use super::*;
use crate::services::onchain_execution_run_store::test_checkpoint;
use shared_types::{LiveOrderState, OnchainCexSettlementBasis, OnchainCexSettlementFee};

fn receipt(side: OrderSide, quantity: &str) -> OnchainCexSettlement {
    OnchainCexSettlement {
        basis: OnchainCexSettlementBasis {
            order_id: "order-1".into(),
            venue: "kraken".into(),
            symbol: "SOL/USD".into(),
            side,
            base_asset: "SOL".into(),
            quote_asset: "USD".into(),
            confirmed_quantity: 1.0,
        },
        status: OnchainCexSettlementStatus::Complete,
        gross_base_amount: Some("1".into()),
        gross_quote_amount: Some("100".into()),
        debit_amount: Some(if side == OrderSide::Buy {
            "100.1".into()
        } else {
            quantity.into()
        }),
        credit_amount: Some(if side == OrderSide::Buy {
            quantity.into()
        } else {
            "99.9".into()
        }),
        fees: vec![],
        fill_event_ids: vec!["fill-1".into()],
        observed_at_ms: Some(3),
        problem: None,
    }
}

fn fixture(side: OrderSide) -> (OnchainCexOrderPlan, OrderRecord) {
    let checkpoint = test_checkpoint();
    let mut plan = checkpoint.primary_plan;
    let mut record = checkpoint.primary_record.unwrap();
    plan.side = side;
    plan.sizing_plan.rounded_contracts = 1.0;
    plan.sizing_plan.contract_size = 1.0;
    plan.instrument_spec.source_url =
        Some("https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument".into());
    record.intent.side = side;
    (plan, record)
}

#[test]
fn cex_compensation_sells_only_net_base_received_after_fees() {
    let (original, record) = fixture(OrderSide::Buy);
    let reverse = plan(&original, &record, &receipt(OrderSide::Buy, "0.999")).unwrap();
    assert_eq!(reverse.side, OrderSide::Sell);
    assert_eq!(reverse.base_quantity, 0.999);
    assert!(reverse.base_quantity < record.filled_quantity.unwrap());
    assert_ne!(reverse.client_order_id, original.client_order_id);
}

#[test]
fn cex_compensation_exact_lot_does_not_oversell_or_fail_on_float_tail() {
    let (original, record) = fixture(OrderSide::Buy);
    let reverse = plan(&original, &record, &receipt(OrderSide::Buy, "1.001")).unwrap();
    assert_eq!(reverse.base_quantity.to_string(), "1.001");
    assert_eq!(reverse.sizing_plan.rounded_contracts, reverse.base_quantity);
    assert_eq!(reverse.sizing_plan.rounded_base_qty, reverse.base_quantity);
    assert_eq!(
        reverse.sizing_plan.actual_notional_usd,
        reverse.estimated_quote_amount
    );
    let reverse = plan(
        &original,
        &record,
        &receipt(OrderSide::Buy, "1.00099999999999999999"),
    )
    .unwrap();
    assert_eq!(reverse.base_quantity, 1.0);
}

#[test]
fn cex_compensation_keeps_step_dust_as_residual_instead_of_claiming_flat() {
    let (original, record) = fixture(OrderSide::Buy);
    let original_receipt = receipt(OrderSide::Buy, "0.9996");
    let reverse_plan = plan(&original, &record, &original_receipt).unwrap();
    assert_eq!(reverse_plan.base_quantity, 0.999);
    let mut reverse_receipt = receipt(OrderSide::Sell, "0.999");
    reverse_receipt.basis.order_id = "reverse-1".into();
    let remaining = residual(&original_receipt, &reverse_receipt).unwrap();
    assert_eq!(remaining.to_string(), "0.0006");
    let reversed = super::super::ReversedCexOrder {
        plan: reverse_plan,
        record,
        base_asset: "SOL".into(),
        original_order_id: "order-1".into(),
        remaining_base: Some(remaining),
        recovery_problem: None,
    };
    assert!(
        !reversed.recovered(),
        "a filled reverse order is not necessarily flat inventory"
    );
    assert_eq!(
        reversed.residual_evidence().unwrap().amount.as_deref(),
        Some("0.0006")
    );
}

#[test]
fn cex_compensation_restores_actual_base_debit_and_rechecks_reverse_fee() {
    let (original, record) = fixture(OrderSide::Sell);
    let original_receipt = receipt(OrderSide::Sell, "1.0004");
    let reverse = plan(&original, &record, &original_receipt).unwrap();
    assert_eq!(reverse.side, OrderSide::Buy);
    assert_eq!(reverse.base_quantity, 1.001);
    let mut reverse_receipt = receipt(OrderSide::Buy, "1.000");
    reverse_receipt.basis.order_id = "reverse-1".into();
    reverse_receipt.basis.confirmed_quantity = 1.001;
    assert_eq!(
        residual(&original_receipt, &reverse_receipt)
            .unwrap()
            .to_string(),
        "-0.0004"
    );
}

#[test]
fn cex_compensation_third_asset_cost_does_not_change_base_inventory() {
    let (original, record) = fixture(OrderSide::Buy);
    let mut original_receipt = receipt(OrderSide::Buy, "1");
    original_receipt.fees.push(OnchainCexSettlementFee {
        asset: "BNB".into(),
        amount: "0.01".into(),
    });
    assert_eq!(
        plan(&original, &record, &original_receipt)
            .unwrap()
            .base_quantity,
        1.0
    );
    let mut reverse_receipt = receipt(OrderSide::Sell, "1");
    reverse_receipt.basis.order_id = "reverse-1".into();
    assert_eq!(
        residual(&original_receipt, &reverse_receipt).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn cex_compensation_does_not_erase_tiny_exact_residuals_with_float_tolerance() {
    let original = receipt(OrderSide::Buy, "1");
    let mut reverse = receipt(OrderSide::Sell, "0.99999999999999999999");
    reverse.basis.order_id = "reverse-1".into();
    assert_eq!(
        residual(&original, &reverse)
            .unwrap()
            .normalize()
            .to_string(),
        "0.00000000000000000001"
    );
}

#[test]
fn cex_compensation_never_uses_missing_fees_price_or_mismatched_receipt() {
    let (original, record) = fixture(OrderSide::Buy);
    let mut proof = receipt(OrderSide::Buy, "1");
    proof.status = OnchainCexSettlementStatus::PendingFees;
    assert!(plan(&original, &record, &proof).is_err());
    proof.status = OnchainCexSettlementStatus::Complete;
    proof.basis.order_id = "other".into();
    assert!(plan(&original, &record, &proof).is_err());
    proof.basis.order_id = record.intent.id.clone();
    let mut no_price = record.clone();
    no_price.filled_price = None;
    assert!(plan(&original, &no_price, &proof).is_err());
    proof.credit_amount = Some("0.00001".into());
    assert!(plan(&original, &record, &proof)
        .unwrap_err()
        .contains("下单步长"));
    let mut wrong = proof.clone();
    wrong.basis.order_id = "reverse-1".into();
    wrong.basis.side = OrderSide::Sell;
    wrong.basis.quote_asset = "USDC".into();
    assert!(residual(&proof, &wrong).is_err());
}

#[tokio::test]
async fn cex_compensation_missing_fee_stops_before_any_order_submission() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = dir.path().to_string_lossy().into();
    let state = crate::state::AppState::new(config).await.unwrap();
    let saved = test_checkpoint();
    let claimed = crate::services::onchain_execution_build_store::ClaimedOnchainBuild {
        response: saved.build,
        config: saved.config,
    };
    let (original, record) = fixture(OrderSide::Buy);
    assert_eq!(record.state, LiveOrderState::Filled);
    let context = super::super::CompensationContext {
        state: &state,
        response: super::super::ResponseContext {
            run_id: "fee-wait",
            build: &claimed.response,
            started_at_ms: 1,
        },
        claimed: &claimed,
        plan: &original,
        reason: "local test",
    };
    let result = super::super::reverse_filled_order(context, &original, &record, "primary").await;
    assert!(result.is_err());
    assert!(state.trading_service().list_orders().is_empty());
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}

fn recovery_run() -> shared_types::OnchainExecutionSubmitResponse {
    use shared_types::{OnchainCexRecoveryResidual, OnchainExecutionLegKind as Kind};
    let (plan, record) = fixture(OrderSide::Buy);
    let mut run = test_checkpoint().response;
    run.status = shared_types::OnchainExecutionRunStatus::Exposed;
    let mut original =
        super::super::order_leg_result(1, Kind::PrimaryCex, &record, &plan.instrument_spec);
    original.settlement = Some(receipt(OrderSide::Buy, "1"));
    let mut reverse_record = record;
    reverse_record.intent.id = "reverse-1".into();
    reverse_record.intent.side = OrderSide::Sell;
    let mut reverse = super::super::order_leg_result(
        2,
        Kind::Compensation,
        &reverse_record,
        &plan.instrument_spec,
    );
    let mut proof = receipt(OrderSide::Sell, "1");
    proof.basis.order_id = "reverse-1".into();
    reverse.settlement = Some(proof);
    reverse.recovery_residual = Some(OnchainCexRecoveryResidual {
        original_order_id: "order-1".into(),
        asset: "SOL".into(),
        amount: None,
    });
    run.legs = vec![original, reverse];
    run.chain_transaction_id = None;
    run
}

#[test]
fn cex_compensation_late_fee_reconciliation_restores_link_and_final_status() {
    let mut run = recovery_run();
    run.legs[1].settlement.as_mut().unwrap().status = OnchainCexSettlementStatus::PendingFees;
    refresh_residuals(&mut run, true);
    assert_eq!(run.status, shared_types::OnchainExecutionRunStatus::Exposed);
    assert!(run.legs[1]
        .recovery_residual
        .as_ref()
        .unwrap()
        .amount
        .is_none());
    let encoded = serde_json::to_string(&run).unwrap();
    let mut restored: shared_types::OnchainExecutionSubmitResponse =
        serde_json::from_str(&encoded).unwrap();
    restored.legs[1].settlement.as_mut().unwrap().status = OnchainCexSettlementStatus::Complete;
    refresh_residuals(&mut restored, true);
    assert_eq!(
        restored.status,
        shared_types::OnchainExecutionRunStatus::Compensated
    );
    assert_eq!(restored.remaining_exposure_usd, 0.0);
    assert_eq!(
        restored.legs[1]
            .recovery_residual
            .as_ref()
            .unwrap()
            .amount
            .as_deref(),
        Some("0")
    );
    let once = restored.clone();
    refresh_residuals(&mut restored, true);
    assert_eq!(restored, once);
}

#[test]
fn cex_compensation_reconciliation_does_not_clear_chain_or_duplicate_order_exposure() {
    let mut run = recovery_run();
    refresh_residuals(&mut run, false);
    assert_eq!(run.status, shared_types::OnchainExecutionRunStatus::Exposed);
    let mut duplicate = run.legs[1].clone();
    duplicate.order_id = Some("reverse-2".into());
    run.legs.push(duplicate);
    refresh_residuals(&mut run, true);
    assert_eq!(run.status, shared_types::OnchainExecutionRunStatus::Exposed);
    assert!(run.legs[1]
        .recovery_residual
        .as_ref()
        .unwrap()
        .amount
        .is_none());
    let mut run = recovery_run();
    let build = test_checkpoint().build;
    run.legs.push(super::super::chain_leg_result(
        &build,
        "tx",
        shared_types::OnchainExecutionLegStatus::Confirmed,
        "confirmed",
    ));
    refresh_residuals(&mut run, true);
    assert_eq!(run.status, shared_types::OnchainExecutionRunStatus::Exposed);
}

#[test]
fn cex_compensation_zero_fill_cancel_preserves_original_net_change() {
    let mut run = recovery_run();
    run.legs[1].status = shared_types::OnchainExecutionLegStatus::Cancelled;
    run.legs[1].filled_quantity = Some(0.0);
    run.legs[1].settlement = None;
    refresh_residuals(&mut run, true);
    assert_eq!(run.status, shared_types::OnchainExecutionRunStatus::Exposed);
    assert_eq!(
        run.legs[1]
            .recovery_residual
            .as_ref()
            .unwrap()
            .amount
            .as_deref(),
        Some("1")
    );
}
