use super::*;
use crate::services::onchain_execution_run_store::{test_checkpoint, OnchainExecutionRunStore};
use shared_types::{OnchainCexSettlementFee, OnchainExecutionRunStatus, OnchainUsdValuation};

fn sample() -> (
    ClaimedOnchainBuild,
    OnchainCexOrderPlan,
    OrderRecord,
    OnchainCexSettlement,
) {
    let saved = test_checkpoint();
    let mut build = saved.build;
    let mut config = saved.config;
    config.base_token = "SOL".into();
    config.base_decimals = 9;
    config.quote_token = "USD".into();
    config.quote_decimals = 6;
    config.gas_usd = 0.0;
    config.spread_alert.min_net_spread_bps = 0.0;
    build.direction = OnchainComparisonDirection::BuyCexSellOnchain;
    build.cex_order.side = OrderSide::Buy;
    build.input_amount_raw = "1000000000".into();
    build.output_amount_raw = "105000000".into();
    build.minimum_output_amount_raw = Some("104000000".into());
    build.valid_until_ms = common::time::now_ms() + 60_000;
    let plan = build.cex_order.clone();
    let mut record = saved.primary_record.unwrap();
    record.intent.side = OrderSide::Buy;
    let mut receipt = settlement::seed(&record, &plan.instrument_spec).unwrap();
    receipt.status = OnchainCexSettlementStatus::Complete;
    receipt.debit_amount = Some("100".into());
    receipt.credit_amount = Some("0.999".into());
    receipt.fees = vec![OnchainCexSettlementFee {
        asset: "SOL".into(),
        amount: "0.001".into(),
    }];
    (
        ClaimedOnchainBuild {
            response: build,
            config,
        },
        plan,
        record,
        receipt,
    )
}

fn valuation(rate: f64) -> OnchainUsdValuation {
    OnchainUsdValuation {
        asset: "USD".into(),
        venue: "kraken".into(),
        symbol: "USD/USD".into(),
        source: "same_currency".into(),
        usd_bid: rate,
        usd_ask: rate,
        observed_at_ms: common::time::now_ms(),
    }
}

fn chain(
    claimed: &ClaimedOnchainBuild,
    input: &str,
    minimum: &str,
) -> execution_build::FirmChainContract {
    execution_build::FirmChainContract {
        quote: onchain_monitor::ProviderQuote {
            input_address: claimed.response.input_token.clone(),
            output_address: claimed.response.output_token.clone(),
            input_amount_raw: input.into(),
            output_amount_raw: "105000000".into(),
            router: None,
        },
        minimum_output_amount_raw: minimum.into(),
        transaction: claimed.response.chain_transaction.clone(),
        official_docs_url: "https://developers.jup.ag/docs/swap/order-and-execute".into(),
        quote_observed_at_ms: common::time::now_ms(),
        valid_until_ms: common::time::now_ms() + 60_000,
    }
}

#[test]
fn primary_alignment_only_reduces_input_and_floors_exact_native_units() {
    let (claimed, _, _, mut receipt) = sample();
    assert_eq!(
        aligned_input(&claimed.response, &claimed.config, &receipt).unwrap(),
        "999000000"
    );
    receipt.credit_amount = Some("1.002".into());
    assert_eq!(
        aligned_input(&claimed.response, &claimed.config, &receipt).unwrap(),
        "1000000000"
    );
    receipt.credit_amount = Some("0.9999999999".into());
    assert_eq!(
        aligned_input(&claimed.response, &claimed.config, &receipt).unwrap(),
        "999999999"
    );
    receipt.credit_amount = Some("0.00000000001".into());
    assert!(aligned_input(&claimed.response, &claimed.config, &receipt).is_err());
    receipt.status = OnchainCexSettlementStatus::PendingFees;
    assert!(aligned_input(&claimed.response, &claimed.config, &receipt).is_err());
}

#[test]
fn primary_alignment_profit_uses_paid_cost_and_minimum_output_not_expected_price() {
    let (claimed, _, _, receipt) = sample();
    let costs = paid_assets(&claimed.response, &claimed.config, &receipt, &[]).unwrap();
    assert_eq!(costs["USD"], Decimal::from(100));
    assert!(
        !costs.contains_key("SOL"),
        "base fee is already reflected in the reduced net input"
    );
    let updated = revised_build(
        &claimed,
        &receipt,
        "999000000",
        chain(&claimed, "999000000", "104000000"),
        100.0,
        valuation(1.0),
    )
    .unwrap();
    assert_eq!(updated.response.estimated_net_profit_usd, 4.0);
    assert_eq!(updated.response.input_amount_raw, "999000000");
    assert_eq!(
        updated
            .response
            .chain_input_adjustment
            .as_ref()
            .unwrap()
            .cex_order_id,
        receipt.basis.order_id
    );
    assert!(revised_build(
        &claimed,
        &receipt,
        "999000000",
        chain(&claimed, "999000000", "99000000"),
        100.0,
        valuation(1.0)
    )
    .is_err());
    assert!(revised_build(
        &claimed,
        &receipt,
        "999000000",
        chain(&claimed, "999000000", "104000000"),
        100.0,
        valuation(0.9)
    )
    .is_err());
    assert!(revised_build(
        &claimed,
        &receipt,
        "1000000000",
        chain(&claimed, "1000000000", "104000000"),
        100.0,
        valuation(1.0)
    )
    .is_err());
}

#[test]
fn replenishment_allocation_is_retained_in_requote_profit_gate() {
    let (mut claimed, _, _, receipt) = sample();
    claimed.response.replenishment_costs =
        vec![super::super::super::replenishment_allocation::test_cost()];
    let updated = revised_build(
        &claimed,
        &receipt,
        "999000000",
        chain(&claimed, "999000000", "104000000"),
        100.0,
        valuation(1.0),
    )
    .unwrap();
    assert!((updated.response.estimated_net_profit_usd - 3.92).abs() < 1e-10);
    assert_eq!(
        updated.response.replenishment_costs,
        claimed.response.replenishment_costs
    );
    assert!(revised_build(
        &claimed,
        &receipt,
        "999000000",
        chain(&claimed, "999000000", "100050000"),
        100.0,
        valuation(1.0)
    )
    .is_err());
}

#[test]
fn primary_alignment_counts_conversion_cash_once_and_keeps_third_asset_fees() {
    let (mut claimed, _, _, mut primary) = sample();
    claimed.config.quote_token = "USDC".into();
    let mut conversion = primary.clone();
    conversion.basis.order_id = "conversion".into();
    conversion.basis.side = OrderSide::Sell;
    conversion.basis.base_asset = "USDC".into();
    conversion.basis.symbol = "USDC/USD".into();
    conversion.debit_amount = Some("101".into());
    conversion.credit_amount = Some("100.2".into());
    conversion.fees = vec![OnchainCexSettlementFee {
        asset: "BNB".into(),
        amount: "0.002".into(),
    }];
    let mut order = claimed.response.cex_order.clone();
    order.native_symbol = conversion.basis.symbol.clone();
    order.side = OrderSide::Sell;
    claimed.response.quote_conversion_order = Some(shared_types::OnchainQuoteConversionOrderPlan {
        sequence: OnchainQuoteConversionSequence::BeforePrimaryCex,
        from_asset: "USDC".into(),
        to_asset: "USD".into(),
        planned_from_amount: 101.0,
        planned_to_amount: 100.2,
        order,
    });
    primary.fees.push(OnchainCexSettlementFee {
        asset: "BNB".into(),
        amount: "0.001".into(),
    });
    let costs = paid_assets(
        &claimed.response,
        &claimed.config,
        &primary,
        &[conversion.clone()],
    )
    .unwrap();
    assert_eq!(costs["USDC"], Decimal::from(101));
    assert_eq!(costs["BNB"], Decimal::from_str_exact("0.003").unwrap());
    assert_eq!(
        costs["USD"],
        Decimal::from_str_exact("-0.2").unwrap(),
        "only the retained conversion credit is an asset; the primary payment is not charged twice"
    );
    assert!(paid_assets(
        &claimed.response,
        &claimed.config,
        &primary,
        &[conversion.clone(), conversion.clone()]
    )
    .is_err());
    conversion.credit_amount = Some("99".into());
    assert!(paid_assets(&claimed.response, &claimed.config, &primary, &[conversion]).is_err());
}

#[test]
fn primary_alignment_keeps_sub_unit_residual_visible_after_confirmed_execution() {
    let (claimed, _, record, mut receipt) = sample();
    receipt.credit_amount = Some("0.9999999999".into());
    let updated = revised_build(
        &claimed,
        &receipt,
        "999999999",
        chain(&claimed, "999999999", "104000000"),
        100.0,
        valuation(1.0),
    )
    .unwrap();
    let adjustment = updated.response.chain_input_adjustment.as_ref().unwrap();
    assert_eq!(adjustment.residual_base_amount, "0.0000000009");
    let result = super::super::response(
        super::super::ResponseContext {
            run_id: "dust",
            build: &updated.response,
            started_at_ms: 1,
        },
        Some(&record),
        super::super::RunOutcome {
            status: OnchainExecutionRunStatus::Completed,
            chain_transaction_id: Some("confirmed-tx".into()),
            remaining_exposure_usd: 0.0,
            message: "completed".into(),
            problem: None,
        },
    );
    assert_eq!(result.status, OnchainExecutionRunStatus::Exposed);
    assert!(result.remaining_exposure_usd > 0.0);
    let problem = result.problem.unwrap();
    assert!(problem.contains("预计双端净余量 0.0000000009 SOL"));
    assert!(problem.contains("仍需核对链上实际扣款"));
    assert_eq!(
        result.legs.last().unwrap().status,
        shared_types::OnchainExecutionLegStatus::Confirmed
    );
}

#[test]
fn primary_alignment_resized_build_and_visible_receipt_survive_journal_replay() {
    let (claimed, plan, record, receipt) = sample();
    let updated = revised_build(
        &claimed,
        &receipt,
        "999000000",
        chain(&claimed, "999000000", "104000000"),
        100.0,
        valuation(1.0),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_execution_run_ledger_path =
        Some(dir.path().join("runs.jsonl").to_string_lossy().into());
    let mut checkpoint = test_checkpoint();
    checkpoint.build = updated.response.clone();
    checkpoint.config = updated.config;
    checkpoint.primary_plan = plan;
    checkpoint.primary_record = Some(record);
    checkpoint.response.status = OnchainExecutionRunStatus::AwaitingChainFinality;
    checkpoint.stage =
        crate::services::onchain_execution_run_store::OnchainExecutionStage::ChainBroadcasting;
    checkpoint.transaction_id = Some("adjusted-tx".into());
    checkpoint.response.chain_transaction_id = checkpoint.transaction_id.clone();
    checkpoint.response.legs = vec![super::super::chain_leg_result(
        &checkpoint.build,
        "adjusted-tx",
        shared_types::OnchainExecutionLegStatus::Pending,
        "已重新询价",
    )];
    OnchainExecutionRunStore::load(&config)
        .store
        .append_pending(&checkpoint)
        .unwrap();
    let restored = OnchainExecutionRunStore::load(&config);
    assert_eq!(restored.pending[0].build, checkpoint.build);
    assert_eq!(
        restored.pending[0].response.legs[0].chain_input_adjustment,
        checkpoint.build.chain_input_adjustment
    );
    let mut legacy = serde_json::to_value(&checkpoint.build).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("chainInputAdjustment");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("minimumOutputAmountRaw");
    let legacy: OnchainExecutionBuildResponse = serde_json::from_value(legacy).unwrap();
    assert!(legacy.chain_input_adjustment.is_none() && legacy.minimum_output_amount_raw.is_none());
}

#[tokio::test]
async fn primary_alignment_uses_local_fill_ledger_and_reuses_valid_transaction() {
    let (claimed, plan, record, _) = sample();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fills.jsonl");
    let event = trading::ExecutionLedger::default()
        .record_fill_event(
            &record,
            &trading::FillLedgerInput {
                venue_event_id: "trade".into(),
                quantity: 1.0,
                price: 100.0,
                fee_amount: Some(0.1),
                fee_currency: Some("USD".into()),
                occurred_at_ms: 2,
            },
            shared_types::OrderUpdateSource::PrivateWs,
            3,
        )
        .unwrap();
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string(&event).unwrap()),
    )
    .unwrap();
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = dir.path().to_string_lossy().into();
    config.storage.execution_ledger_path = Some(path.to_string_lossy().into());
    let state = AppState::new(config).await.unwrap();
    let prepared = providers::PreparedChainSubmission::SolanaRpc {
        signed_transaction: "unchanged".into(),
        rpc_url: "https://invalid.test".into(),
        local_transaction_id: "local".into(),
    };
    let aligned = align(&state, &claimed, &plan, &record, &[], prepared)
        .await
        .unwrap();
    assert_eq!(aligned.prepared.transaction_id(), "local");
    assert_eq!(aligned.claimed.response.input_amount_raw, "1000000000");
    assert!((aligned.claimed.response.estimated_net_profit_usd - 3.9).abs() < 1e-9);
    assert!(state.trading_service().list_orders().is_empty());
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .unwrap();
}
