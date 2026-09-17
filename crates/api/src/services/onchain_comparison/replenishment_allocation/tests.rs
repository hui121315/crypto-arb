use super::*;

fn sample() -> (Run, OnchainComparisonConfig) {
    let mut run: Run = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    let config = OnchainComparisonConfig {
        cex_venue: "binance".into(),
        chain: "solana".into(),
        wallet_address: "wallet".into(),
        ..Default::default()
    };
    run.status = Status::Completed;
    run.updated_at_ms = 1000;
    let leg = &mut run.plan.legs[0];
    leg.direction = Direction::WithdrawToChain;
    leg.destination.address = Some(config.wallet_address.clone());
    leg.source_address = Some(config.wallet_address.clone());
    leg.asset_address = Some(config.quote_mint.clone());
    leg.asset = config.quote_token.clone();
    leg.asset_decimals = Some(config.quote_decimals);
    let transfer = &mut run.transfers[0];
    transfer.withdrawal_unlocked = None;
    transfer.withdrawal_cost = Some(shared_types::OnchainReplenishmentWithdrawalCost {
        asset: config.quote_token.clone(),
        reported_amount_exact: "12.5".into(),
        fee_exact: "0.1".into(),
        confirmed: true,
        source: "Binance withdrawal history".into(),
        observed_at_ms: 999,
        usd_valuation: None,
    });
    (run, config)
}

fn valued(run: &Run, config: &OnchainComparisonConfig) -> Cost {
    let mut cost = from_run(run, config, run.plan.direction, 1000).unwrap();
    let assets = cost
        .fees
        .iter()
        .filter(|f| f.amount_exact != "0")
        .map(|f| &f.asset)
        .collect::<BTreeSet<_>>();
    let rates = assets
        .into_iter()
        .map(|asset| usd_valuation::fixture(asset, if asset == "SOL" { 100.0 } else { 0.8 }, 1000))
        .collect();
    cost.build_valuation = accounting::value_flows(&cost.fees, rates, 1000).unwrap();
    cost
}

pub(super) fn cost() -> Cost {
    let (run, config) = sample();
    valued(&run, &config)
}

#[test]
fn replenishment_allocation_charges_only_actual_fee_once_not_principal_or_venue_gas() {
    let (mut run, config) = sample();
    run.plan.legs[0].economics.estimated_cost_usd = Some(99.0);
    run.transfers[0].network_cost = Some(super::super::replenishment_costs::fixture(
        "0.001", 100.0, 1000,
    ));
    let cost = valued(&run, &config);
    assert_eq!(cost.fees.len(), 1);
    assert_eq!(cost.fees[0].amount_exact, "-0.1");
    assert_eq!(total_usd(&[cost.clone()]).unwrap(), 0.08);
    assert_eq!(fees(&[cost.clone()]).unwrap().len(), 1);
    assert!(fees(&[cost.clone(), cost.clone()]).is_err());
    let mut changed = cost.clone();
    changed.run_id = "another-run-same-withdrawal".into();
    assert!(fees(&[cost, changed]).is_err());
}

#[test]
fn replenishment_allocation_rejects_incomplete_or_wrong_wallet_contract_venue_and_direction() {
    let (run, config) = sample();
    for case in [
        "pending",
        "missing_fee",
        "unconfirmed_fee",
        "fee_asset",
        "contract",
        "precision",
        "wallet",
        "venue",
        "chain",
        "direction",
        "credit",
        "transfer_index",
        "time",
    ] {
        let mut bad = run.clone();
        match case {
            "pending" => bad.status = Status::AwaitingDestinationCredit,
            "missing_fee" => bad.transfers[0].withdrawal_cost = None,
            "unconfirmed_fee" => {
                bad.transfers[0].withdrawal_cost.as_mut().unwrap().confirmed = false
            }
            "fee_asset" => {
                bad.transfers[0].withdrawal_cost.as_mut().unwrap().asset = "OTHER".into()
            }
            "contract" => bad.plan.legs[0].asset_address = Some("other-mint".into()),
            "precision" => bad.plan.legs[0].asset_decimals = Some(1),
            "wallet" => bad.plan.legs[0].destination.address = Some("Wallet".into()),
            "venue" => bad.plan.legs[0].venue = "other".into(),
            "chain" => bad.plan.legs[0].chain = "ethereum".into(),
            "direction" => bad.plan.direction = OnchainComparisonDirection::BuyOnchainSellCex,
            "credit" => bad.transfers[0].credited_amount_exact = Some("12.4".into()),
            "transfer_index" => bad.transfers[0].leg_index = 1,
            "time" => bad.updated_at_ms = 1001,
            _ => unreachable!(),
        }
        assert!(
            from_run(&bad, &config, run.plan.direction, 1000).is_err(),
            "{case}"
        );
    }
}

#[test]
fn replenishment_allocation_deposit_preserves_zero_vs_unknown_and_validates_network_receipt() {
    let (mut run, config) = sample();
    run.plan.legs[0].direction = Direction::DepositToCex;
    run.transfers[0].withdrawal_cost = None;
    run.transfers[0].reported_deposit_amount_exact = Some("12.5".into());
    run.transfers[0].deposit_fee_exact = Some("0".into());
    let mut network = super::super::replenishment_costs::fixture("0.000005", 100.0, 1000);
    network.transaction_id = run.transfers[0].transaction_id.clone().unwrap();
    run.transfers[0].network_cost = Some(network);
    let cost = valued(&run, &config);
    assert_eq!(total_usd(&[cost.clone()]).unwrap(), 0.0005);
    assert_eq!(fees(&[cost]).unwrap().len(), 1);
    for case in [
        "missing_deposit_fee",
        "positive_deposit_fee",
        "network_total",
        "network_payer",
        "network_transaction",
        "reported_amount",
    ] {
        let mut bad = run.clone();
        let transfer = &mut bad.transfers[0];
        match case {
            "missing_deposit_fee" => transfer.deposit_fee_exact = None,
            "positive_deposit_fee" => transfer.deposit_fee_exact = Some("0.1".into()),
            "network_total" => {
                transfer.network_cost.as_mut().unwrap().total_fee_exact = Some("0".into())
            }
            "network_payer" => {
                transfer.network_cost.as_mut().unwrap().payer = "someone-else".into()
            }
            "network_transaction" => {
                transfer.network_cost.as_mut().unwrap().transaction_id = "other-tx".into()
            }
            "reported_amount" => transfer.reported_deposit_amount_exact = None,
            _ => unreachable!(),
        }
        assert!(
            from_run(&bad, &config, run.plan.direction, 1000).is_err(),
            "{case}"
        );
    }
}

#[test]
fn replenishment_allocation_frozen_valuation_rejects_repricing_and_missing_rates() {
    let cost = cost();
    for case in ["total", "rate", "missing", "positive_fee", "kind"] {
        let mut bad = cost.clone();
        match case {
            "total" => bad.build_valuation.net_usd_exact = "0".into(),
            "rate" => bad.build_valuation.rates[0].usd_ask = 1.0,
            "missing" => bad.build_valuation.rates.clear(),
            "positive_fee" => bad.fees[0].amount_exact = "0.1".into(),
            "kind" => bad.fees[0].kind = Kind::Trade,
            _ => unreachable!(),
        }
        assert!(total_usd(&[bad]).is_err(), "{case}");
    }
}
