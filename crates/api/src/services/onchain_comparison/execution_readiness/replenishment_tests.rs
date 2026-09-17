use super::*;

#[test]
fn partial_inventory_only_replenishes_the_missing_amount() {
    let mut missing = OnchainInventoryEvidence {
        location: OnchainInventoryLocation::Onchain,
        scope: "base".to_owned(),
        asset: "USDC".to_owned(),
        required: 100.0,
        available: Some(80.0),
        status: OnchainInventoryStatus::Insufficient,
        source: "fixture".to_owned(),
        observed_at_ms: Some(1),
        problem: None,
    };
    for location in [
        OnchainInventoryLocation::Onchain,
        OnchainInventoryLocation::Cex,
    ] {
        missing.location = location;
        assert_eq!(inventory_shortfall(&missing), Some(Decimal::from(20)));
    }
    missing.required = 0.3;
    missing.available = Some(0.1);
    assert_eq!(inventory_shortfall(&missing), Some(Decimal::new(2, 1)));
    for available in [None, Some(-1.0), Some(f64::NAN), Some(0.3), Some(1.0)] {
        missing.available = available;
        assert_eq!(inventory_shortfall(&missing), None);
    }
}

#[test]
fn source_balance_covers_the_rounded_transfer_and_fee_not_just_the_gap() {
    let evidence = transfer();
    for (available, expected) in [
        (20.21, OnchainTransferStatus::Blocked),
        (20.2101, OnchainTransferStatus::Ready),
        (21.0, OnchainTransferStatus::Ready),
        (f64::NAN, OnchainTransferStatus::Unknown),
    ] {
        let mut row = evidence.clone();
        check_replenishment_source_balance(&mut row, available);
        assert_eq!(row.status, expected);
    }
    let mut deposit = evidence;
    deposit.direction = OnchainTransferDirection::DepositToCex;
    check_replenishment_source_balance(&mut deposit, 20.005);
    assert_eq!(deposit.status, OnchainTransferStatus::Blocked);
}

#[test]
fn stablecoin_base_fee_uses_its_market_price_and_unrelated_quote_is_not_one() {
    let mut snapshot = OnchainComparisonSnapshot::default();
    snapshot.quote_usd_valuation = Some(super::super::usd_valuation::fixture("USDC", 1.0, 0));
    snapshot.config.base_token = "USDT".to_owned();
    snapshot.config.quote_token = "USDC".to_owned();
    let comparison = OnchainCexComparison {
        direction: OnchainComparisonDirection::BuyOnchainSellCex,
        onchain_price: 1.03,
        cex_price: 1.05,
        gross_spread_bps: 100.0,
        cex_fee_bps: 0.0,
        quote_conversion_fee_bps: 0.0,
        slippage_bps: 0.0,
        gas_usd: 0.0,
        gas_bps: 0.0,
        total_cost_bps: 0.0,
        net_spread_bps: 100.0,
        observable_notional_usd: 100.0,
        executable: true,
    };
    let mut row = transfer();
    row.asset = "USDT".to_owned();
    row.fee = Some(1.0);
    row.fee_exact = Some("1".to_owned());
    let economics = transfer_economics(&snapshot, &comparison, &[row]);
    assert_eq!(economics.transfer_cost_usd, Some(1.05));
    assert!(economics.post_transfer_net_profit_usd.unwrap() < 0.0);
    assert_eq!(
        transfer_asset_quote_price(&snapshot, &comparison, "USDC"),
        Some(1.0)
    );
    assert_eq!(
        transfer_asset_quote_price(&snapshot, &comparison, "USD"),
        None
    );
}

fn transfer() -> OnchainTransferEvidence {
    OnchainTransferEvidence {
        direction: OnchainTransferDirection::WithdrawToChain,
        venue: "binance".to_owned(),
        asset: "USDC".to_owned(),
        chain: "base".to_owned(),
        network: Some("BASE".to_owned()),
        amount: 20.01,
        amount_exact: Some("20.01".to_owned()),
        status: OnchainTransferStatus::Ready,
        fee: Some(0.2001),
        fee_exact: Some("0.2001".to_owned()),
        minimum: Some(1.0),
        minimum_exact: Some("1".to_owned()),
        amount_step: Some("0.01".to_owned()),
        requires_tag: false,
        contract_verified: true,
        credit_confirmations: Some(1),
        unlock_confirmations: Some(2),
        network_status: None,
        source: Some("fixture".to_owned()),
        observed_at_ms: Some(1),
        problem: None,
    }
}
