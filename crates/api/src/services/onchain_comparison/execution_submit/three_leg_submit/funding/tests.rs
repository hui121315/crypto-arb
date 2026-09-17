use super::super::{empty_conversion, restored_conversion_run, SubmitContext};
use super::*;
use crate::services::onchain_execution_run_store::{
    test_checkpoint, OnchainQuoteConversionAttempt,
};
use shared_types::{ExecutionLedgerEvent, LiveOrderState, OrderUpdateSource};
use trading::{ExecutionLedger, FillLedgerInput};

fn config() -> OnchainComparisonConfig {
    OnchainComparisonConfig {
        cex_taker_fee_bps: 10.0,
        ..Default::default()
    }
}

fn conversion() -> OnchainQuoteConversionOrderPlan {
    let mut order = test_checkpoint().primary_plan;
    order.native_symbol = "USDC/USD".into();
    order.instrument_spec.native_symbol = order.native_symbol.clone();
    order.instrument_spec.display_symbol = order.native_symbol.clone();
    order.base_quantity = 100.3;
    order.reference_price = 1.0;
    order.estimated_quote_amount = 100.3;
    order.sizing_plan.rounded_contracts = 100.3;
    order.sizing_plan.rounded_base_qty = 100.3;
    order.sizing_plan.contract_size = 1.0;
    OnchainQuoteConversionOrderPlan {
        sequence: OnchainQuoteConversionSequence::BeforePrimaryCex,
        from_asset: "USDC".into(),
        to_asset: "USD".into(),
        planned_from_amount: 100.3,
        planned_to_amount: 100.3,
        order,
    }
}

fn limits(plan: &OnchainQuoteConversionOrderPlan) -> Limits {
    let mut primary = test_checkpoint().primary_plan;
    primary.side = OrderSide::Buy;
    before_primary(&config(), &primary, plan).unwrap()
}

fn attempt(
    plan: &OnchainQuoteConversionOrderPlan,
    id: &str,
    quantity: f64,
    fee: Option<f64>,
    asset: Option<&str>,
) -> (OnchainQuoteConversionAttempt, ExecutionLedgerEvent) {
    let mut order = plan.order.clone();
    order.client_order_id = format!("client-{id}");
    order.base_quantity = quantity;
    order.sizing_plan.rounded_contracts = quantity;
    order.sizing_plan.rounded_base_qty = quantity;
    let mut record = test_checkpoint().primary_record.unwrap();
    record.intent.id = id.into();
    record.intent.client_order_id = order.client_order_id.clone();
    record.intent.symbol = order.native_symbol.clone();
    record.intent.side = order.side;
    record.intent.quantity = quantity;
    record.identity = shared_types::VenueOrderIdentity::from_intent(&record.intent);
    record.filled_quantity = Some(quantity);
    record.filled_price = Some(1.0);
    record.state = LiveOrderState::Filled;
    let event = ExecutionLedger::default()
        .record_fill_event(
            &record,
            &FillLedgerInput {
                venue_event_id: format!("fill-{id}"),
                quantity,
                price: 1.0,
                fee_amount: fee,
                fee_currency: asset.map(str::to_owned),
                occurred_at_ms: 2,
            },
            OrderUpdateSource::PrivateWs,
            3,
        )
        .unwrap();
    (
        OnchainQuoteConversionAttempt {
            plan: order,
            record,
        },
        event,
    )
}

async fn state(events: &[ExecutionLedgerEvent]) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fills.jsonl");
    let rows = events
        .iter()
        .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
        .collect::<String>();
    std::fs::write(&path, rows).unwrap();
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = dir.path().to_string_lossy().into();
    config.storage.execution_ledger_path = Some(path.to_string_lossy().into());
    (dir, AppState::new(config).await.unwrap())
}

#[tokio::test]
async fn conversion_funding_gross_fill_is_not_net_funding_and_retries_reserve_fees() {
    let plan = conversion();
    let (record, event) = attempt(&plan, "one", 100.3, Some(0.3), Some("USD"));
    let (_dir, state) = state(&[event]).await;
    let mut run = restored_conversion_run(&plan, vec![record]);
    assert!(
        run.complete,
        "legacy gross completion would have incorrectly released the next leg"
    );
    assert!(
        refresh(&state, &plan, &mut run, limits(&plan)).await,
        "{:?}",
        run.problem
    );
    assert!(!run.complete);
    assert_eq!(run.accounted_to, Decimal::from(100));
    assert!(remaining(&plan, &run, limits(&plan))
        .unwrap_err()
        .contains("不能动用账户其他余额"));
    let mut run = empty_conversion(None);
    run.filled_from = 50.0;
    run.accounted_from = Decimal::from_str_exact("50.2").unwrap();
    run.accounted_to = Decimal::from_str_exact("49.8").unwrap();
    let (input, output) = remaining(&plan, &run, limits(&plan)).unwrap();
    assert!(
        decimal(input).unwrap() * Decimal::from_str_exact("1.001").unwrap()
            <= limits(&plan).max_debit - run.accounted_from
    );
    assert!(
        decimal(output).unwrap() * Decimal::from_str_exact("0.999").unwrap()
            >= limits(&plan).required_credit - run.accounted_to
    );
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn conversion_funding_reloads_net_receipts_and_never_resubmits_completed_conversion() {
    let plan = conversion();
    let (one, first) = attempt(&plan, "one", 50.0, Some(0.05), Some("USD"));
    let (two, second) = attempt(&plan, "two", 50.2, Some(0.05), Some("USD"));
    let (_dir, state) = state(&[first.clone(), second, first]).await;
    let encoded = serde_json::to_vec(&vec![one, two]).unwrap();
    let restored = restored_conversion_run(&plan, serde_json::from_slice(&encoded).unwrap());
    let saved = test_checkpoint();
    let claimed = crate::services::onchain_execution_build_store::ClaimedOnchainBuild {
        response: saved.build,
        config: config(),
    };
    let context = SubmitContext {
        state: &state,
        claimed: &claimed,
        response: super::super::super::ResponseContext {
            run_id: "funded",
            build: &claimed.response,
            started_at_ms: 1,
        },
    };
    let result = super::super::execute_conversion(context, &plan, restored, limits(&plan)).await;
    assert!(result.complete, "{:?}", result.problem);
    assert_eq!(
        result.accounted_from,
        Decimal::from_str_exact("100.2").unwrap()
    );
    assert_eq!(
        result.accounted_to,
        Decimal::from_str_exact("100.1").unwrap()
    );
    assert_eq!(result.attempts.len(), 2);
    assert!(state.trading_service().list_orders().is_empty());
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn conversion_funding_missing_fee_stops_before_any_followup_order() {
    let plan = conversion();
    let (record, event) = attempt(&plan, "one", 100.3, None, None);
    let (_dir, state) = state(&[event]).await;
    let run = restored_conversion_run(&plan, vec![record]);
    let saved = test_checkpoint();
    let claimed = crate::services::onchain_execution_build_store::ClaimedOnchainBuild {
        response: saved.build,
        config: config(),
    };
    let context = SubmitContext {
        state: &state,
        claimed: &claimed,
        response: super::super::super::ResponseContext {
            run_id: "missing-fee",
            build: &claimed.response,
            started_at_ms: 1,
        },
    };
    let result = super::super::execute_conversion(context, &plan, run, limits(&plan)).await;
    assert!(!result.complete && result.receipt_unresolved);
    assert!(result.problem.as_ref().unwrap().contains("手续费"));
    assert!(state.trading_service().list_orders().is_empty());
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}

#[test]
fn conversion_funding_after_sell_uses_only_actual_primary_credit() {
    let mut plan = conversion();
    plan.sequence = OnchainQuoteConversionSequence::AfterPrimaryCex;
    plan.from_asset = "USD".into();
    plan.to_asset = "USDC".into();
    plan.order.side = OrderSide::Buy;
    let saved = test_checkpoint();
    let mut receipt = super::super::super::settlement::seed(
        saved.primary_record.as_ref().unwrap(),
        &saved.primary_plan.instrument_spec,
    )
    .unwrap();
    receipt.status = OnchainCexSettlementStatus::Complete;
    receipt.credit_amount = Some("99.5".into());
    let limits = limits_from_primary(&config(), &plan, &receipt).unwrap();
    assert_eq!(limits.max_debit, Decimal::from_str_exact("99.5").unwrap());
    let run = empty_conversion(None);
    assert!(!initial_plan_fits(&plan, &run, limits).unwrap());
    assert!(validate_next(&plan, &plan, &run, limits).is_err());
    let (input, _) = remaining(&plan, &run, limits).unwrap();
    assert!(input < 99.5);
    receipt.basis.quote_asset = "USDT".into();
    assert!(limits_from_primary(&config(), &plan, &receipt).is_err());
}

#[tokio::test]
async fn conversion_funding_input_fee_overrun_stops_but_third_asset_fee_stays_separate() {
    let plan = conversion();
    for asset in ["USDC", "BNB"] {
        let (record, event) = attempt(&plan, "fee-currency", 100.3, Some(0.3), Some(asset));
        let (_dir, state) = state(&[event]).await;
        let mut run = restored_conversion_run(&plan, vec![record]);
        let ok = refresh(&state, &plan, &mut run, limits(&plan)).await;
        if asset == "USDC" {
            assert!(!ok && run.receipt_unresolved && !run.complete);
            assert_eq!(
                run.accounted_from,
                Decimal::from_str_exact("100.6").unwrap()
            );
            assert!(run.problem.as_ref().unwrap().contains("超过本次资金上限"));
        } else {
            assert!(ok && run.complete);
            assert_eq!(
                run.accounted_from,
                Decimal::from_str_exact("100.3").unwrap()
            );
        }
        assert_eq!(
            run.attempts[0].settlement.as_ref().unwrap().fees[0].asset,
            asset
        );
        assert!(state.trading_service().list_orders().is_empty());
        state
            .trading_service()
            .drain_sql_ledger_and_shutdown()
            .await
            .unwrap();
    }
}

#[test]
fn conversion_funding_rejects_cross_market_and_tiny_shortfall_is_not_complete() {
    let plan = conversion();
    let limits = limits(&plan);
    let mut run = empty_conversion(None);
    run.accounted_to =
        limits.required_credit - Decimal::from_str_exact("0.00000000000000000001").unwrap();
    let (_, output) = remaining(&plan, &run, limits).unwrap();
    assert!(output > 0.0);
    let mut primary = test_checkpoint().primary_plan;
    primary.side = OrderSide::Buy;
    primary.instrument_spec.quote_asset = Some("USDT".into());
    assert!(before_primary(&config(), &primary, &plan).is_err());
    let (one, _) = attempt(&plan, "same", 1.0, Some(0.0), Some("USD"));
    let mut other = one.clone();
    other.plan.client_order_id = "different-client".into();
    other.record.intent.client_order_id = other.plan.client_order_id.clone();
    let run = restored_conversion_run(&plan, vec![one, other]);
    assert!(run.receipt_unresolved);
    assert_eq!(run.filled_from, 1.0);
}
