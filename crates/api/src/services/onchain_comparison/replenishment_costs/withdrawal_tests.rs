use super::*;
use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;

fn receipt(fee: &str) -> OnchainReplenishmentWithdrawalCost {
    OnchainReplenishmentWithdrawalCost {
        asset: "USDC".into(),
        reported_amount_exact: "12.5".into(),
        fee_exact: fee.into(),
        confirmed: true,
        source: "fixture withdrawal receipt".into(),
        observed_at_ms: 1000,
        usd_valuation: None,
    }
}

#[test]
fn withdrawal_fee_valuation_uses_actual_amount_and_usdc_ask_not_a_dollar_peg() {
    let mut cost = receipt("0.1");
    assert!(value_withdrawal_fee(&cost, None, 1000).is_err());
    let mut quote = super::super::usd_valuation::fixture("USDC", 0.98, 1000);
    quote.usd_ask = 0.99;
    let value = value_withdrawal_fee(&cost, Some(quote), 1000).unwrap();
    assert_eq!(value.usd_amount_exact, "0.099");
    cost.usd_valuation = Some(value);
    assert!((withdrawal_fee_usd(&cost).unwrap() - 0.099).abs() < 1e-12);
    cost.fee_exact = "0.2".into();
    assert!(withdrawal_fee_usd(&cost).is_err());
    cost.fee_exact = "0".into();
    cost.usd_valuation = Some(value_withdrawal_fee(&cost, None, 1000).unwrap());
    assert_eq!(withdrawal_fee_usd(&cost).unwrap(), 0.0);
    assert!(cost.usd_valuation.as_ref().unwrap().quote.is_none());
}

#[test]
fn withdrawal_fee_valuation_rejects_unconfirmed_invalid_precision_and_wrong_evidence() {
    for case in [
        "pending",
        "source",
        "future_receipt",
        "negative",
        "precision",
        "asset",
        "pair",
        "rest",
        "stale",
        "crossed",
    ] {
        let mut cost = receipt("0.1");
        let mut quote = super::super::usd_valuation::fixture("USDC", 1.0, 1000);
        match case {
            "pending" => cost.confirmed = false,
            "source" => cost.source.clear(),
            "future_receipt" => cost.observed_at_ms = 1001,
            "negative" => cost.fee_exact = "-0.1".into(),
            "precision" => cost.fee_exact = "0.00000000000000000000000000001".into(),
            "asset" => quote.asset = "USDT".into(),
            "pair" => quote.symbol = "USDC/USDT".into(),
            "rest" => quote.source = "rest_baseline".into(),
            "stale" => quote.observed_at_ms = -30_000,
            "crossed" => quote.usd_ask = 0.5,
            _ => unreachable!(),
        }
        assert!(
            value_withdrawal_fee(&cost, Some(quote), 1000).is_err(),
            "{case}"
        );
    }
}

#[test]
fn withdrawal_fee_valuation_is_durable_without_repricing_or_resuming_money_actions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.jsonl");
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    run.status = shared_types::OnchainReplenishmentRunStatus::Completed;
    run.plan.legs[0].direction = shared_types::OnchainTransferDirection::WithdrawToChain;
    run.plan.legs[0].economics.estimated_cost_usd = Some(0.2);
    run.transfers[0].withdrawal_cost = Some(receipt("0.1"));
    std::fs::write(
        &path,
        format!("{}\n", serde_json::json!({"schemaVersion":1,"run":run})),
    )
    .unwrap();
    let store = OnchainReplenishmentPlanStore::load(&config);
    let before = store.run(&run.run_id, 1000).unwrap();
    assert!(waiting_for_valuation(&before));
    assert_eq!(missing_assets(&[before.clone()]), vec!["USDC"]);
    let value = value_withdrawal_fee(
        &receipt("0.1"),
        Some(super::super::usd_valuation::fixture("USDC", 0.98, 1000)),
        1000,
    )
    .unwrap();
    let valued = store
        .record_withdrawal_cost_valuation(&run.run_id, 0, value.clone(), 1000)
        .unwrap();
    assert!(!waiting_for_valuation(&valued));
    assert!(missing_assets(&[valued.clone()]).is_empty());
    assert_eq!(valued.status, before.status);
    assert_eq!(valued.transfers[0].status, before.transfers[0].status);
    assert_eq!(
        valued.transfers[0].last_checked_at_ms,
        before.transfers[0].last_checked_at_ms
    );
    assert_eq!(
        valued.transfers[0].transaction_id,
        before.transfers[0].transaction_id
    );
    assert_eq!(valued.plan.legs[0].economics.estimated_cost_usd, Some(0.2));
    assert_eq!(
        valued.plan.legs[0].economics.reconciled_cost_usd,
        Some(0.098)
    );
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        store
            .record_withdrawal_cost_valuation(&run.run_id, 0, value, 1001)
            .unwrap(),
        valued
    );
    let repriced = value_withdrawal_fee(
        &receipt("0.1"),
        Some(super::super::usd_valuation::fixture("USDC", 1.01, 1001)),
        1001,
    )
    .unwrap();
    assert!(store
        .record_withdrawal_cost_valuation(&run.run_id, 0, repriced, 1001)
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let restored = OnchainReplenishmentPlanStore::load(&config)
        .run(&run.run_id, 1002)
        .unwrap();
    assert_eq!(restored.transfers, valued.transfers);
    assert_eq!(restored.plan, valued.plan);
    let mut legacy = serde_json::to_value(&restored).unwrap();
    legacy["transfers"][0]["withdrawalCost"]
        .as_object_mut()
        .unwrap()
        .remove("usdValuation");
    let mut legacy: OnchainReplenishmentRun = serde_json::from_value(legacy).unwrap();
    assert!(waiting_for_valuation(&legacy));
    for amount in ["0", "0.000"] {
        legacy.transfers[0]
            .withdrawal_cost
            .as_mut()
            .unwrap()
            .fee_exact = amount.into();
        assert!(
            missing_assets(&[legacy.clone()]).is_empty(),
            "zero fees need no FX subscription"
        );
    }
    legacy.transfers[0]
        .withdrawal_cost
        .as_mut()
        .unwrap()
        .confirmed = false;
    assert!(
        !waiting_for_valuation(&legacy),
        "a pending fee is not a proven cost"
    );
}
