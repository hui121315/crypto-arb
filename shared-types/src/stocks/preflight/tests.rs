use super::*;
fn account() -> StockAccountEvidence {
    StockAccountEvidence {
        fingerprint: "fixture".into(),
        spot_maker_fee_bps: "-0.5".into(),
        spot_taker_fee_bps: "10".into(),
        liquidating: false,
        fees_at_ms: 1000,
        balances_at_ms: 1000,
        balances: [
            (
                "MU.US".into(),
                StockAccountBalance {
                    available: "0".into(),
                    locked: "10".into(),
                    staked: "20".into(),
                    observed_at_ms: 1000,
                    source_at_us: None,
                },
            ),
            (
                "USDC".into(),
                StockAccountBalance {
                    available: "100".into(),
                    locked: "0".into(),
                    staked: "0".into(),
                    observed_at_ms: 1000,
                    source_at_us: None,
                },
            ),
        ]
        .into_iter()
        .collect(),
    }
}
#[test]
fn stock_preflight_bps_inventory_and_unknown_costs_are_not_treated_as_profit_or_free_funds() {
    let s = crate::stocks::comparison::tests::fixture();
    let a = account();
    let wallet = StockWalletEvidence {
        owner: "fixture".into(),
        mint: "MU".into(),
        stock_raw: Some("16000".into()),
        usdc_raw: Some("10000000".into()),
        sol_lamports: Some("1000000".into()),
        checked_at_ms: 1000,
        problems: vec![],
    };
    let rows = evaluate_preflight(&s, Some(&a), Some(&wallet), 1200);
    assert_eq!(rows[0].cex_fee_usdc.as_deref(), Some("0.01002"));
    assert_eq!(rows[0].after_known_costs_usdc.as_deref(), Some("0.00998"));
    assert_eq!(rows[0].inventory[0].available.as_deref(), Some("0"));
    assert_eq!(rows[0].inventory[0].sufficient, Some(false));
    assert_eq!(rows[1].inventory[1].available.as_deref(), Some("0.02"));
    assert_eq!(rows[1].inventory[1].sufficient, Some(true));
    assert!(rows
        .iter()
        .all(|r| !r.executable && r.blockers.iter().any(|b| b.contains("Gas"))));
    let mut missing = a.clone();
    missing.balances.remove("USDC");
    assert_eq!(
        evaluate_preflight(&s, Some(&missing), Some(&wallet), 1200)[1].inventory[0].available,
        None
    );
    missing.balances_at_ms = -100_000;
    assert!(
        evaluate_preflight(&s, Some(&missing), Some(&wallet), 1200)[0]
            .cex_fee_usdc
            .is_none()
    );
    let mut with_cost = s.clone();
    let c = s.comparison.as_ref().unwrap();
    let cost = StockChainCost {
        transaction: None,
        asset: c.asset.clone(),
        direction: StockChainDirection::Buy,
        wallet_address: wallet.owner.clone(),
        mint: c.mint.clone(),
        quote: c.buy.clone(),
        transaction_fingerprint: "fixture".into(),
        checked_at_ms: 1100,
        valid_until_ms: 1500,
        provider_fees: vec![],
        network_fee_lamports: Some("7000".into()),
        wallet_debit_lamports: Some("2046280".into()),
        wallet_budget_lamports: Some("2046280".into()),
        wallet_required_lamports: Some("2937160".into()),
        native_valuation: None,
        simulation_slot: Some(12),
        simulation_passed: true,
        problems: vec![],
    };
    assert!(cost.current(&with_cost, &wallet.owner, 1200));
    assert!(!cost.current(&with_cost, "different-wallet", 1200));
    with_cost.chain_costs.push(cost);
    let rows = evaluate_preflight(&with_cost, Some(&a), Some(&wallet), 1200);
    assert_eq!(rows[0].inventory[2].required.as_deref(), Some("0.00293716"));
    assert_eq!(rows[0].inventory[2].sufficient, Some(false));
    assert_eq!(rows[0].after_known_costs_usdc.as_deref(), Some("0.00998"));
    assert!(!rows[0].executable);
    assert!(rows[0].blockers.iter().any(|p| p.contains("USDC 置换成本")));
    assert!(
        evaluate_preflight(&with_cost, Some(&a), Some(&wallet), 1600)[0].inventory[2]
            .required
            .is_none()
    );
    with_cost.chain_costs[0].native_valuation = Some(StockNativeValuation {
        native_lamports: "2046280".into(),
        replenishment: None,
        quote: StockDexQuote {
            input_mint: comparison::SOLANA_USDC.into(),
            output_mint: STOCK_WRAPPED_SOL.into(),
            input_raw: "300000".into(),
            output_raw: "3000000".into(),
            minimum_output_raw: "2900000".into(),
            expires_at_ms: Some(1400),
            ..c.buy.clone()
        },
    });
    let rows = evaluate_preflight(&with_cost, Some(&a), Some(&wallet), 1200);
    assert_eq!(rows[0].native_fee_usdc.as_deref(), Some("0.3"));
    assert_eq!(rows[0].after_known_costs_usdc.as_deref(), Some("-0.29002"));
    assert!(!rows[0].executable);
    let report = StockPreflight {
        funding: vec![],
        asset: c.asset.clone(),
        wallet_address: Some(wallet.owner.clone()),
        checked_at_ms: 1200,
        valid_until_ms: 5000,
        price_basis: StockPriceBasis::from_snapshot(&with_cost),
        spot_taker_fee_pct: None,
        account_at_ms: None,
        wallet_at_ms: None,
        directions: rows,
        problems: vec![],
    };
    assert!(report.current(&with_cost, 1200));
    let mut missing_required = serde_json::to_value(&with_cost.chain_costs[0]).unwrap();
    missing_required
        .as_object_mut()
        .unwrap()
        .remove("walletRequiredLamports");
    let mut legacy = with_cost.clone();
    legacy.chain_costs[0] = serde_json::from_value(missing_required).unwrap();
    assert!(!report.current(&legacy, 1200));
    let rows = evaluate_preflight(&legacy, Some(&a), Some(&wallet), 1200);
    assert!(rows[0].inventory[2].required.is_none());
    assert_eq!(rows[0].native_fee_usdc.as_deref(), Some("0.3"));
    assert!(rows[0]
        .blockers
        .iter()
        .any(|p| p.contains("不使用 Provider 估算代替")));
    legacy.chain_costs[0].wallet_budget_lamports = Some("9000000".into());
    assert_eq!(
        legacy.chain_costs[0].native_usdc_budget(1200).as_deref(),
        Some("0.3")
    );
    assert!(!report.current(&with_cost, 1400));
    assert!(
        evaluate_preflight(&with_cost, Some(&a), Some(&wallet), 1400)[0]
            .native_fee_usdc
            .is_none()
    );
    let mut zero = with_cost.chain_costs[0].clone();
    zero.native_valuation = None;
    zero.wallet_debit_lamports = Some("0".into());
    assert!(zero.wallet_required_lamports.as_deref() != Some("0"));
    assert_eq!(zero.native_usdc_budget(1200).as_deref(), Some("0"));
    zero.simulation_passed = false;
    assert!(zero.native_usdc_budget(1200).is_none());
    zero.simulation_passed = true;
    zero.wallet_debit_lamports = None;
    assert!(zero.native_usdc_budget(1200).is_none());
    with_cost
        .comparison
        .as_mut()
        .unwrap()
        .buy
        .minimum_output_raw = "16999".into();
    assert!(
        evaluate_preflight(&with_cost, Some(&a), Some(&wallet), 1200)[0].inventory[2]
            .required
            .is_none()
    );
}
#[test]
fn stock_preflight_rfq_embedded_fee_is_not_charged_twice_and_transfer_limits_are_visible() {
    let mut s = crate::stocks::comparison::tests::fixture();
    let session = StockSession {
        name: "fixture".into(),
        min_quantity: "0.01".into(),
        max_quantity: None,
        step_size: "0.01".into(),
    };
    let route = s.trading_route.as_mut().unwrap();
    route.kind = StockRouteKind::Rfq;
    route.symbol = Some("RFQ".into());
    route.session = Some(session);
    s.rfq_connected = true;
    s.rfqs = vec![StockRfq {
        request: StockRfqRequest {
            request_id: "local-preflight-rfq".into(),
            asset: "MU.US".into(),
            side: StockRfqSide::Ask,
            quantity: "0.02".into(),
        },
        client_id: 1,
        account_fingerprint: "fixture".into(),
        symbol: "RFQ".into(),
        rfq_id: Some("1".into()),
        phase: StockRfqPhase::Candidate,
        candidate: Some(StockRfqCandidate {
            quote_id: "2".into(),
            taker_price: "501".into(),
            source_at_us: 1_100_000,
            received_at_ms: 1100,
        }),
        submission_time_ms: Some(1100),
        expiry_time_ms: Some(20_000),
        source_at_us: Some(1_100_000),
        fill_price: None,
        executed_quantity: None,
        executed_quote_quantity: None,
        fills: vec![],
        settlement: Default::default(),
        acceptance: None,
        needs_recheck: false,
        cancel_requested: false,
        created_at_ms: 1000,
        updated_at_ms: 1100,
        problem: None,
    }];
    let rows = evaluate_preflight(&s, Some(&account()), None, 1200);
    assert_eq!(rows[0].cex_fee_usdc.as_deref(), Some("0"));
    assert_eq!(rows[0].after_known_costs_usdc.as_deref(), Some("0.02"));
    assert!(rows[0].transfer_problem.is_some());
    let basis = StockPriceBasis::from_snapshot(&s);
    let mut changed_amount = s.clone();
    changed_amount.comparison.as_mut().unwrap().buy.input_raw = "10000001".into();
    assert_ne!(basis, StockPriceBasis::from_snapshot(&changed_amount));
    s.rfqs[0].cancel_requested = true;
    assert_ne!(basis, StockPriceBasis::from_snapshot(&s));
    assert!(evaluate_preflight(&s, Some(&account()), None, 1200)[0]
        .after_known_costs_usdc
        .is_none());
}
