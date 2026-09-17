use super::*;

#[test]
fn stock_saved_plan_keeps_original_terms_through_ws_and_draft_refreshes() {
    for direction in [StockChainDirection::Buy, StockChainDirection::Sell] {
        let (mut s, mut a, mut r) = fixture(10_000);
        r.direction = direction;
        let plan = prepare(r, &s, &a, 10_000).unwrap();
        let original = plan.clone();
        s.books[0].update_id += 100;
        s.books[0].source_at_ms = 10_500;
        s.books[0].received_at_ms = 10_500;
        // Favorable prices and an unrelated opposite-side change cannot rewrite the FOK order.
        if direction == StockChainDirection::Buy {
            s.books[0].bid = Some("602".into());
            s.books[0].ask_quantity = None;
        } else {
            s.books[0].ask = Some("600".into());
            s.books[0].bid_quantity = None;
        }
        let c = s.comparison.as_mut().unwrap();
        c.budget_usdc = "50".into();
        c.buy.input_raw = "50000000".into();
        c.buy.requested_at_ms = 10_500;
        c.sell = None;
        c.mint.slot += 1;
        c.mint.chain_time_ms = 10_500;
        c.mint.checked_at_ms = 10_500;
        s.chain_costs.clear();
        s.preflight = None;
        a.balances_at_ms = 10_500;
        a.fees_at_ms = 10_500;
        a.spot_taker_fee_bps = "10.00".into();
        for b in a.balances.values_mut() {
            b.observed_at_ms = 10_500;
        }
        validate_for_submission(&plan, &s, &a, 10_500).unwrap();
        assert_eq!(plan, original);
        assert!(validate_for_submission(&plan, &s, &a, plan.terms.market_valid_until_ms).is_err());
    }
}

#[test]
fn stock_saved_plan_rejects_economic_changes_and_unknown_evidence() {
    for direction in [StockChainDirection::Buy, StockChainDirection::Sell] {
        for change in 0..18 {
            let (mut s, mut a, mut r) = fixture(100_000);
            r.direction = direction;
            let plan = prepare(r, &s, &a, 100_000).unwrap();
            match change {
                0 => {
                    if direction == StockChainDirection::Buy {
                        s.books[0].bid = Some("599".into())
                    } else {
                        s.books[0].ask = Some("602".into())
                    }
                }
                1 => {
                    if direction == StockChainDirection::Buy {
                        s.books[0].bid_quantity = Some("0.001".into())
                    } else {
                        s.books[0].ask_quantity = Some("0.001".into())
                    }
                }
                2 => s.books[0].source_at_ms = 90_000,
                3 => s.connected = false,
                4 => s.comparison.as_mut().unwrap().mint.ui_multiplier = "2".into(),
                5 => s.comparison.as_mut().unwrap().mint.next_change_at_ms = Some(100_500),
                6 => s.tokens[0].contract_address = Some("other-mint".into()),
                7 => s.trading_route.as_mut().unwrap().kind = StockRouteKind::Closed,
                8 => a.spot_taker_fee_bps = "11".into(),
                9 => {
                    a.balances
                        .get_mut(&plan.terms.allocations[0].asset)
                        .unwrap()
                        .available = "0".into()
                }
                10 => {
                    a.balances
                        .get_mut(&plan.terms.allocations[0].asset)
                        .unwrap()
                        .observed_at_ms = 1
                }
                11 => a.liquidating = true,
                12 => a.fingerprint = "different-key".into(),
                13 => {
                    refresh_report(&mut s, &a, 100_500);
                    s.preflight.as_mut().unwrap().directions
                        [usize::from(direction == StockChainDirection::Sell)]
                    .inventory[1]
                        .available = None;
                }
                14 => {
                    let p = s.preflight.as_mut().unwrap();
                    p.checked_at_ms = 100_500;
                    p.wallet_at_ms = None;
                    p.problems.push("local wallet read failed".into());
                }
                15 => {
                    s.preflight.as_mut().unwrap().wallet_at_ms = None;
                }
                16 => {
                    s.preflight.as_mut().unwrap().directions
                        [usize::from(direction == StockChainDirection::Sell)]
                        .inventory[1].available = Some("0".into());
                }
                _ => s.security.as_mut().unwrap().order_books[0].tick_size = "0.1".into(),
            }
            assert!(
                validate_for_submission(&plan, &s, &a, 100_500).is_err(),
                "accepted {direction:?} change {change}"
            );
        }
    }
}

#[test]
fn stock_saved_rfq_ignores_other_candidates_but_never_switches_bound_quote() {
    let (mut s, a, r) = rfq_fixture(10_000);
    let plan = prepare(r, &s, &a, 10_000).unwrap();
    let mut unrelated = s.rfqs[0].clone();
    unrelated.request.request_id = "other-stock-candidate".into();
    unrelated.candidate.as_mut().unwrap().quote_id = "99887766".into();
    s.rfqs.insert(0, unrelated);
    s.preflight = None;
    s.comparison.as_mut().unwrap().buy.input_raw = "20000000".into();
    validate_for_submission(&plan, &s, &a, 10_100).unwrap();
    for change in 0..4 {
        let mut changed = s.clone();
        let r = &mut changed.rfqs[1];
        match change {
            0 => r.candidate.as_mut().unwrap().quote_id = "11223344".into(),
            1 => r.candidate.as_mut().unwrap().taker_price = "601".into(),
            2 => r.cancel_requested = true,
            _ => r.expiry_time_ms = Some(12_000),
        }
        assert!(validate_for_submission(&plan, &changed, &a, 10_100).is_err());
    }
}

#[test]
fn stock_saved_preflight_is_durable_and_legacy_history_remains_readable() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("proof.jsonl");
    let (s, a, r) = fixture(10_000);
    let plan = prepare(r, &s, &a, 10_000).unwrap();
    {
        let store = plan_store::PlanStore::load(Some(path.clone()), Default::default());
        store.reserve(plan.clone(), 10_000).unwrap();
    }
    let store = plan_store::PlanStore::load(Some(path), Default::default());
    let restored = store.get(&plan.plan_id).unwrap();
    assert_eq!(restored, plan);
    validate_for_submission(&restored, &s, &a, 10_100).unwrap();
    for change in 0..3 {
        let mut bad = plan.clone();
        let p = bad.terms.preflight_evidence.as_mut().unwrap();
        match change {
            0 => p.wallet_address = Some("other-wallet".into()),
            1 => p.wallet_at_ms = None,
            _ => p.directions[0].inventory[1].required = Some("0".into()),
        }
        assert!(bad.validate_preflight_evidence().is_err());
    }
    let mut legacy = plan;
    legacy.terms.preflight_evidence = None;
    legacy.plan_id = plan_store::plan_id(&legacy.request, &legacy.terms).unwrap();
    let path = temp.path().join("legacy.jsonl");
    {
        let store = plan_store::PlanStore::load(Some(path.clone()), Default::default());
        store.reserve(legacy.clone(), 10_000).unwrap();
    }
    let store = plan_store::PlanStore::load(Some(path), Default::default());
    let restored = store.get(&legacy.plan_id).unwrap();
    assert_eq!(restored, legacy);
    assert!(!serde_json::to_string(&restored)
        .unwrap()
        .contains("preflightEvidence"));
    assert!(validate_for_submission(&restored, &s, &a, 10_100)
        .unwrap_err()
        .contains("旧计划"));
}
