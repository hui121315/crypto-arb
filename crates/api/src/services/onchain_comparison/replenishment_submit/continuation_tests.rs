use super::*;
use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;

fn plans() -> (OnchainReplenishmentRun, OnchainReplenishmentPlanResponse) {
    let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    run.status = OnchainReplenishmentRunStatus::ReadyForNextTransfer;
    run.plan.transfer_cost_usd = Some(0.3);
    run.plan.post_transfer_net_profit_usd = Some(1.0);
    let first = &mut run.plan.legs[0];
    first.asset_address = Some("usdc-mint".into());
    first.source_address = Some("wallet".into());
    first.economics.estimated_cost_usd = Some(0.1);
    first.economics.estimated_network_cost_usd = Some(0.05);
    first.economics.source_debit_upper_bound_exact = Some("12.5".into());
    let mut second = first.clone();
    second.direction = OnchainTransferDirection::WithdrawToChain;
    second.destination.address = Some("wallet".into());
    second.source_address = None;
    second.asset = "SOL".into();
    second.asset_address = Some("So11111111111111111111111111111111111111112".into());
    second.asset_decimals = Some(9);
    second.economics.estimated_cost_usd = Some(0.2);
    second.economics.source_debit_upper_bound_exact = Some("12.6".into());
    run.plan.legs.push(second);
    run.transfers[0].withdrawal_unlocked = Some(true);
    run.transfers[0].transaction_id = Some("signature".into());
    run.transfers[0].network_cost = Some(super::super::replenishment_costs::fixture(
        "0.0005", 100.0, 50,
    ));
    let mut remaining = run.plan.clone();
    remaining.plan_id = "fresh-remaining-preview".into();
    remaining.legs.remove(0);
    remaining.transfer_cost_usd = Some(0.2);
    remaining.post_transfer_net_profit_usd = Some(1.1);
    remaining.built_at_ms = 60;
    remaining.valid_until_ms = 200;
    (run, remaining)
}

#[test]
fn replenishment_continuation_keeps_completed_prefix_and_counts_its_cost() {
    let (run, remaining) = plans();
    let current = revalidated_remaining_plan(&run, remaining.clone()).unwrap();
    assert_eq!(current.legs.len(), 2);
    let mut expected = run.plan.legs[0].clone();
    expected.economics.reconciled_cost_usd = Some(0.1);
    assert_eq!(current.legs[0], expected);
    assert_eq!(current.legs[1], remaining.legs[0]);
    assert_eq!(current.plan_id, run.plan.plan_id);
    assert_eq!(current.built_at_ms, 60);
    assert!((current.transfer_cost_usd.unwrap() - 0.3).abs() < 1e-12);
    assert!((current.post_transfer_net_profit_usd.unwrap() - 1.0).abs() < 1e-12);

    let mut first_submission = run.clone();
    first_submission.status = OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit;
    first_submission.transfers.clear();
    assert!(revalidated_remaining_plan(&first_submission, run.plan.clone()).is_ok());
}

#[test]
fn replenishment_continuation_rejects_missing_cost_and_apparent_profit_without_sunk_fees() {
    let (mut run, remaining) = plans();
    let mut worsened = remaining.clone();
    worsened.post_transfer_net_profit_usd = Some(1.05);
    assert_eq!(
        revalidated_remaining_plan(&run, worsened)
            .unwrap_err()
            .code(),
        "ONCHAIN_REPLENISHMENT_ECONOMICS_WORSENED"
    );
    for cost in [None, Some(f64::NAN), Some(-0.1)] {
        run.plan.legs[0].economics.estimated_cost_usd = cost;
        assert!(revalidated_remaining_plan(&run, remaining.clone()).is_err());
    }
    let (run, mut remaining) = plans();
    remaining.transfer_cost_usd = Some(0.01);
    assert!(revalidated_remaining_plan(&run, remaining).is_err());
    let (mut run, remaining) = plans();
    run.plan.post_transfer_net_profit_usd = Some(f64::NAN);
    assert!(revalidated_remaining_plan(&run, remaining).is_err());
}

#[test]
fn replenishment_continuation_cannot_skip_short_locked_or_unconfirmed_credit() {
    let (run, remaining) = plans();
    for variant in ["short", "locked", "pending", "wrong_index", "extra"] {
        let mut invalid = run.clone();
        match variant {
            "short" => invalid.transfers[0].credited_amount_exact = Some("12.4".into()),
            "locked" => invalid.transfers[0].withdrawal_unlocked = Some(false),
            "pending" => {
                invalid.transfers[0].status = OnchainReplenishmentTransferStatus::SourceCompleted
            }
            "wrong_index" => invalid.transfers[0].leg_index = 1,
            "extra" => invalid.transfers.push(invalid.transfers[0].clone()),
            _ => unreachable!(),
        }
        assert!(
            revalidated_remaining_plan(&invalid, remaining.clone()).is_err(),
            "{variant}"
        );
    }
}

#[test]
fn replenishment_continuation_still_binds_remaining_amount_asset_network_and_destination() {
    let (run, remaining) = plans();
    for field in [
        "amount",
        "asset",
        "network",
        "destination",
        "fee",
        "duplicate",
    ] {
        let mut changed = remaining.clone();
        match field {
            "amount" => changed.legs[0].transfer_amount_exact = Some("13".into()),
            "asset" => changed.legs[0].asset_address = Some("other-mint".into()),
            "network" => changed.legs[0].network_evidence.network = Some("OTHER".into()),
            "destination" => changed.legs[0].destination.address = Some("other-wallet".into()),
            "fee" => changed.legs[0].economics.source_debit_upper_bound_exact = Some("13".into()),
            "duplicate" => changed.legs.insert(0, run.plan.legs[0].clone()),
            _ => unreachable!(),
        }
        assert!(
            revalidated_remaining_plan(&run, changed).is_err(),
            "{field}"
        );
    }
}

#[test]
fn replenishment_continuation_survives_restart_and_claims_only_the_unfinished_leg() {
    let (run, remaining) = plans();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replenishment.jsonl");
    std::fs::write(
        &path,
        format!("{}\n", serde_json::json!({"schemaVersion":1,"run":run})),
    )
    .unwrap();
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let store = OnchainReplenishmentPlanStore::load(&config);
    let restored = store.run(&run.run_id, 60).unwrap();
    let current = revalidated_remaining_plan(&restored, remaining).unwrap();
    let claim = store
        .claim_submission(&run.run_id, "test", current.clone(), 1, &store.submission_snapshot(), 70)
        .unwrap();
    assert!(!claim.replayed);
    assert_eq!(claim.run.transfers.len(), 2);
    assert_eq!(claim.run.transfers[0], run.transfers[0]);
    assert_eq!(claim.run.transfers[1].leg_index, 1);
    let mut expected = run.plan.legs[0].clone();
    expected.economics.reconciled_cost_usd = Some(0.1);
    assert_eq!(claim.run.plan.legs[0], expected);
    drop(store);
    let store = OnchainReplenishmentPlanStore::load(&config);
    let replay = store
        .claim_submission(&run.run_id, "test", current, 1, &store.submission_snapshot(), 90)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.run.transfers.len(), 2);
    assert_eq!(
        replay.run.transfers[1].client_transfer_id,
        claim.run.transfers[1].client_transfer_id
    );
    assert_eq!(
        replay.run.plan.legs[0].economics.estimated_cost_usd,
        Some(0.1)
    );
}

#[test]
fn replenishment_continuation_legacy_cost_remains_unknown_instead_of_zero() {
    let (run, remaining) = plans();
    let mut json = serde_json::to_value(run).unwrap();
    json["plan"]["legs"][0]["economics"]
        .as_object_mut()
        .unwrap()
        .remove("estimatedCostUsd");
    let legacy: OnchainReplenishmentRun = serde_json::from_value(json).unwrap();
    assert!(legacy.plan.legs[0].economics.estimated_cost_usd.is_none());
    assert!(revalidated_remaining_plan(&legacy, remaining).is_err());
}

#[test]
fn replenishment_continuation_cannot_borrow_another_transaction_network_cost() {
    let (run, remaining) = plans();
    for field in ["chain", "asset", "payer", "transaction"] {
        let mut changed = run.clone();
        let cost = changed.transfers[0].network_cost.as_mut().unwrap();
        match field {
            "chain" => cost.chain = "ethereum".into(),
            "asset" => cost.asset = "ETH".into(),
            "payer" => cost.payer = "other-wallet".into(),
            "transaction" => cost.transaction_id = "other-signature".into(),
            _ => unreachable!(),
        }
        assert_eq!(
            revalidated_remaining_plan(&changed, remaining.clone())
                .unwrap_err()
                .code(),
            "ONCHAIN_REPLENISHMENT_COMPLETED_COST_MISSING",
            "{field}"
        );
    }
}

#[test]
fn replenishment_continuation_replaces_gas_budget_once_and_rejects_unvalued_or_over_budget_spend() {
    let (mut run, remaining) = plans();
    run.transfers[0].network_cost = Some(super::super::replenishment_costs::fixture(
        "0.0002", 100.0, 50,
    ));
    let current = revalidated_remaining_plan(&run, remaining.clone()).unwrap();
    assert!((current.transfer_cost_usd.unwrap() - 0.27).abs() < 1e-12);
    assert!((current.post_transfer_net_profit_usd.unwrap() - 1.03).abs() < 1e-12);
    assert_eq!(current.legs[0].economics.estimated_cost_usd, Some(0.1));
    assert_eq!(
        current.legs[0].economics.estimated_network_cost_usd,
        Some(0.05)
    );
    assert!((current.legs[0].economics.reconciled_cost_usd.unwrap() - 0.07).abs() < 1e-12);
    let mut repeated = run.clone();
    repeated.plan = current.clone();
    assert_eq!(
        revalidated_remaining_plan(&repeated, remaining.clone())
            .unwrap()
            .transfer_cost_usd,
        current.transfer_cost_usd
    );
    run.transfers[0].network_cost = Some(super::super::replenishment_costs::fixture(
        "0.002", 100.0, 50,
    ));
    assert_eq!(
        revalidated_remaining_plan(&run, remaining.clone())
            .unwrap_err()
            .code(),
        "ONCHAIN_REPLENISHMENT_ECONOMICS_WORSENED"
    );
    run.transfers[0]
        .network_cost
        .as_mut()
        .unwrap()
        .usd_valuation = None;
    assert_eq!(
        revalidated_remaining_plan(&run, remaining.clone())
            .unwrap_err()
            .code(),
        "ONCHAIN_REPLENISHMENT_COMPLETED_COST_MISSING"
    );
    run.transfers[0].network_cost =
        Some(super::super::replenishment_costs::fixture("0", 100.0, 50));
    run.plan.legs[0].economics.estimated_network_cost_usd = None;
    assert!(revalidated_remaining_plan(&run, remaining).is_err());
}

#[test]
fn replenishment_continuation_uses_actual_withdrawal_fee_without_adding_the_estimate_again() {
    let (mut run, remaining) = plans();
    run.plan.legs[0].direction = OnchainTransferDirection::WithdrawToChain;
    run.plan.legs[0].economics.estimated_network_cost_usd = None;
    run.plan.legs[0].economics.fee_amount_exact = Some("0.1".into());
    run.transfers[0].network_cost = None;
    let mut cost = shared_types::OnchainReplenishmentWithdrawalCost {
        asset: "USDC".into(), reported_amount_exact: "12.5".into(), fee_exact: "0.05".into(),
        confirmed: true, source: "fixture withdrawal receipt".into(), observed_at_ms: 50, usd_valuation: None,
    };
    cost.usd_valuation = Some(super::super::replenishment_costs::value_withdrawal_fee(
        &cost, Some(super::super::usd_valuation::fixture("USDC", 0.98, 50)), 50,
    ).unwrap());
    run.transfers[0].withdrawal_cost = Some(cost.clone());
    let current = revalidated_remaining_plan(&run, remaining.clone()).unwrap();
    assert!((current.transfer_cost_usd.unwrap() - 0.249).abs() < 1e-12);
    assert!((current.post_transfer_net_profit_usd.unwrap() - 1.051).abs() < 1e-12);
    assert_eq!(current.legs[0].economics.reconciled_cost_usd, Some(0.049));
    assert_eq!(current.legs[0].economics.estimated_cost_usd, Some(0.1));
    let mut repeated = run.clone();
    repeated.plan = current.clone();
    assert_eq!(revalidated_remaining_plan(&repeated, remaining.clone()).unwrap().transfer_cost_usd,
        current.transfer_cost_usd);
    run.transfers[0].withdrawal_cost.as_mut().unwrap().usd_valuation = None;
    assert!(replenishment_message(&run).contains("暂不执行下一步"));
    assert_eq!(revalidated_remaining_plan(&run, remaining.clone()).unwrap_err().code(),
        "ONCHAIN_REPLENISHMENT_COMPLETED_COST_MISSING");
    cost.usd_valuation = Some(super::super::replenishment_costs::value_withdrawal_fee(
        &cost, Some(super::super::usd_valuation::fixture("USDC", 3.0, 50)), 50,
    ).unwrap());
    run.transfers[0].withdrawal_cost = Some(cost.clone());
    assert_eq!(revalidated_remaining_plan(&run, remaining.clone()).unwrap_err().code(),
        "ONCHAIN_REPLENISHMENT_ECONOMICS_WORSENED");
    cost.fee_exact = "0".into();
    cost.usd_valuation = Some(super::super::replenishment_costs::value_withdrawal_fee(&cost, None, 50).unwrap());
    run.transfers[0].withdrawal_cost = Some(cost);
    assert_eq!(revalidated_remaining_plan(&run, remaining).unwrap().transfer_cost_usd, Some(0.2));
}
