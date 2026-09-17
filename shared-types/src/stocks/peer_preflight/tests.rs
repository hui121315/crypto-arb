use super::*;
fn fixture() -> StockMarketSnapshot {
    let mut s = crate::stocks::peers::tests::fixture();
    s.peer_preflight = Some(StockPeerPreflight {
        asset: "MU.US".into(),
        selection: s.peer.as_ref().unwrap().selection.clone(),
        checked_at_ms: 1000,
        account: Some(StockPeerAccount {
            venue: "kraken".into(),
            native_symbol: "MUx/USD".into(),
            stock_asset: "MUx".into(),
            quote_asset: "USD".into(),
            stock_available: Some("1".into()),
            quote_available: Some("9999".into()),
            usdc_available: Some("10".into()),
            stock_taker_pct: Some("0.1".into()),
            fx_taker_pct: Some("0.2".into()),
            observed_at_ms: 1000,
            sources: vec![],
            problems: vec![],
        }),
        wallet: Some(StockWalletEvidence {
            owner: "wallet".into(),
            mint: "verified-stock".into(),
            stock_raw: Some("1000000".into()),
            usdc_raw: Some("100000000".into()),
            sol_lamports: Some("1000000000".into()),
            checked_at_ms: 1000,
            problems: vec![],
        }),
        problems: vec![],
    });
    s
}
#[test]
fn stock_peer_preflight_budgets_two_fee_directions_without_borrowing_or_implicit_usd() {
    let s = fixture();
    let rows = evaluate_peer_preflight(&s, 1100);
    let sell = rows[0]
        .trading_cost_usdc
        .as_deref()
        .unwrap()
        .parse::<Decimal>()
        .unwrap();
    assert_eq!(
        sell,
        Decimal::from(66)
            - Decimal::from(66) * "0.999".parse::<Decimal>().unwrap()
                / "1.002".parse::<Decimal>().unwrap()
    );
    let buy = rows[1].inventory[0]
        .required
        .as_deref()
        .unwrap()
        .parse::<Decimal>()
        .unwrap();
    assert_eq!(
        buy,
        "133.2".parse::<Decimal>().unwrap() * "1.001".parse::<Decimal>().unwrap()
    );
    assert_eq!(rows[0].inventory[0].sufficient, Some(false));
    assert_eq!(rows[1].inventory[0].asset, "USD");
    assert_eq!(rows[1].inventory[0].available.as_deref(), Some("9999"));
    assert!(rows
        .iter()
        .all(|r| r.after_known_costs_usdc.is_none() && r.native_cost_usdc.is_none()));
    assert_eq!(
        rows[1].inventory[1].required.as_deref(),
        Some("1"),
        "chain inventory is tokens, not rebased shares"
    );
    assert_eq!(
        rows[0].inventory[0].required.as_deref(),
        Some("1.2"),
        "exchange inventory is shares"
    );
    for case in 0..7 {
        let mut bad = s.clone();
        let a = bad
            .peer_preflight
            .as_mut()
            .unwrap()
            .account
            .as_mut()
            .unwrap();
        match case {
            0 => a.stock_taker_pct = None,
            1 => a.fx_taker_pct = None,
            2 => a.native_symbol = "MUx/USDT".into(),
            3 => a.observed_at_ms = -15000,
            4 => a.stock_asset = "other".into(),
            5 => a.stock_taker_pct = Some("100".into()),
            _ => a.venue = "backpack".into(),
        }
        assert!(
            evaluate_peer_preflight(&bad, 1100)
                .iter()
                .all(|r| r.trading_cost_usdc.is_none()),
            "case {case}"
        );
    }
    let mut thin = s;
    thin.peer
        .as_mut()
        .unwrap()
        .quote_conversion
        .as_mut()
        .unwrap()
        .bid_quantity = Some("133.3".into());
    assert!(
        evaluate_peer_preflight(&thin, 1100)[1]
            .trading_cost_usdc
            .is_none(),
        "fees cannot exceed quoted FX size"
    );
}

#[test]
fn stock_peer_known_costs_require_exact_current_simulation_and_explicit_native_cost() {
    let mut s = fixture();
    let comparison = s.comparison.as_ref().unwrap();
    s.chain_costs = [StockChainDirection::Buy, StockChainDirection::Sell]
        .into_iter()
        .map(|direction| StockChainCost {
            transaction: None,
            asset: comparison.asset.clone(),
            direction,
            wallet_address: "wallet".into(),
            mint: comparison.mint.clone(),
            quote: direction.quote(comparison).unwrap().clone(),
            transaction_fingerprint: "local-sponsored-fixture".into(),
            checked_at_ms: 1000,
            valid_until_ms: 2000,
            provider_fees: vec![],
            network_fee_lamports: Some("5000".into()),
            // Fixture sponsor pays the network fee; explicit zero is not missing.
            wallet_debit_lamports: Some("0".into()),
            wallet_budget_lamports: Some("0".into()),
            wallet_required_lamports: Some("0".into()),
            native_valuation: None,
            simulation_slot: Some(1),
            simulation_passed: true,
            problems: vec![],
        })
        .collect();
    let rows = evaluate_peer_preflight(&s, 1100);
    for row in rows {
        assert_eq!(row.native_cost_usdc.as_deref(), Some("0"));
        assert_eq!(
            number(row.after_known_costs_usdc.as_deref().unwrap()),
            number(row.gross_usdc.as_deref().unwrap())
                .zip(number(row.trading_cost_usdc.as_deref().unwrap()))
                .map(|(gross, fee)| gross - fee)
        );
        assert_eq!(row.inventory[2].required.as_deref(), Some("0"));
    }
    for case in 0..6 {
        let mut bad = s.clone();
        for cost in &mut bad.chain_costs {
            match case {
                0 => cost.wallet_debit_lamports = None,
                1 => cost.wallet_debit_lamports = Some("5000".into()),
                2 => cost.wallet_address = "another-wallet".into(),
                3 => cost.quote.input_raw = "1".into(),
                4 => cost.simulation_passed = false,
                _ => cost.valid_until_ms = 1100,
            }
        }
        assert!(
            evaluate_peer_preflight(&bad, 1100)
                .iter()
                .all(|r| r.native_cost_usdc.is_none() && r.after_known_costs_usdc.is_none()),
            "case {case}"
        );
    }
}
