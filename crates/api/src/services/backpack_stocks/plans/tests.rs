use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};

mod submission;

fn keys() -> Result<credentials::Credentials, String> {
    credentials::Credentials::parse(
        &STANDARD.encode(common::signing::ed25519_public_key(&[7; 32]).unwrap()),
        &STANDARD.encode([7; 32]),
    )
}

pub(in crate::services::backpack_stocks) fn fixture(
    now: i64,
) -> (StockMarketSnapshot, StockAccountEvidence, StockPlanRequest) {
    let mut s = comparison::tests::snapshot();
    s.connected = true;
    s.observed_at_ms = now;
    s.token_metadata_at_ms = Some(now);
    s.books = vec![StockBookQuote {
        symbol: "MU.US_USDC".into(),
        bid: Some("600".into()),
        bid_quantity: Some("10".into()),
        ask: Some("601".into()),
        ask_quantity: Some("10".into()),
        update_id: 1,
        source_at_ms: now,
        received_at_ms: now,
    }];
    let mut c = comparison::tests::comparison();
    c.mint.checked_at_ms = now;
    c.mint.chain_time_ms = now;
    c.buy.requested_at_ms = now;
    c.buy.received_at_ms = now;
    c.buy.fee_bps = Some(10);
    c.buy.fee_mint = Some(shared_types::stocks::comparison::SOLANA_USDC.into());
    c.sell = Some(StockDexQuote {
        input_mint: c.mint.address.clone(),
        output_mint: c.buy.input_mint.clone(),
        input_raw: "16000".into(),
        output_raw: "14000000".into(),
        minimum_output_raw: "13000000".into(),
        ..c.buy.clone()
    });
    let wallet = bs58::encode(common::signing::ed25519_public_key(&[7; 32]).unwrap()).into_string();
    s.chain_costs = [StockChainDirection::Buy, StockChainDirection::Sell]
        .into_iter()
        .map(|direction| StockChainCost {
            transaction: None,
            asset: c.asset.clone(),
            direction,
            wallet_address: wallet.clone(),
            mint: c.mint.clone(),
            quote: direction.quote(&c).unwrap().clone(),
            transaction_fingerprint: "local-sponsored-transaction".into(),
            checked_at_ms: now,
            valid_until_ms: now + 5000,
            provider_fees: vec![],
            network_fee_lamports: Some("0".into()),
            wallet_debit_lamports: Some("0".into()),
            wallet_budget_lamports: Some("0".into()),
            wallet_required_lamports: Some("890880".into()),
            native_valuation: None,
            simulation_slot: Some(1),
            simulation_passed: true,
            problems: vec![],
        })
        .collect();
    s.comparison = Some(c);
    let balance = |n: &str| StockAccountBalance {
        available: n.into(),
        locked: "0".into(),
        staked: "0".into(),
        observed_at_ms: now,
        source_at_us: None,
    };
    let a = StockAccountEvidence {
        fingerprint: keys().unwrap().fingerprint(),
        spot_maker_fee_bps: "8".into(),
        spot_taker_fee_bps: "10".into(),
        liquidating: false,
        fees_at_ms: now,
        balances_at_ms: now,
        balances: [
            ("MU.US".into(), balance("2")),
            ("USDC".into(), balance("25")),
        ]
        .into_iter()
        .collect(),
    };
    refresh_report(&mut s, &a, now);
    let request = StockPlanRequest {
        request_id: "local-stock-plan-0001".into(),
        asset: "MU.US".into(),
        direction: StockChainDirection::Buy,
        wallet_address: wallet,
        preflight_at_ms: now,
        build: None,
    };
    (s, a, request)
}

pub(in crate::services::backpack_stocks) fn refresh_report(s: &mut StockMarketSnapshot, a: &StockAccountEvidence, now: i64) {
    let c = s.comparison.as_ref().unwrap();
    let wallet = StockWalletEvidence {
        owner: s.chain_costs[0].wallet_address.clone(),
        mint: c.mint.address.clone(),
        stock_raw: Some("16000".into()),
        usdc_raw: Some("25000000".into()),
        sol_lamports: Some("1000000000".into()),
        checked_at_ms: now,
        problems: vec![],
    };
    s.preflight = Some(StockPreflight {
        funding: vec![],
        asset: c.asset.clone(),
        wallet_address: Some(wallet.owner.clone()),
        checked_at_ms: now,
        valid_until_ms: now + 5000,
        price_basis: StockPriceBasis::from_snapshot(s),
        spot_taker_fee_pct: Some("0.1".into()),
        account_at_ms: Some(a.balances_at_ms),
        wallet_at_ms: Some(now),
        directions: evaluate_preflight(s, Some(a), Some(&wallet), now),
        problems: vec![],
    });
}

pub(in crate::services::backpack_stocks) fn fixture_plan(now: i64) -> StockExecutionPlan {
    let (s, a, r) = fixture(now);
    prepare(r, &s, &a, now).unwrap()
}

impl BackpackStocks {
    pub(crate) fn stock_plan_fixture(
        path: std::path::PathBuf,
        now: i64,
    ) -> (Self, StockPlanRequest) {
        let (snapshot, account, request) = fixture(now);
        let mut service = Self::new().unwrap().with_plan_store(path);
        service.credential_loader = keys;
        service.root = "http://127.0.0.1:9".into();
        service.ws_url = "ws://127.0.0.1:9".into();
        *service.snapshot.write() = snapshot;
        service.account.write().fingerprint = account.fingerprint.clone();
        service.account.write().evidence = Some(account);
        (service, request)
    }
}

#[test]
fn stock_plan_both_directions_bind_cost_inventory_and_earliest_evidence_expiry() {
    let (mut s, a, r) = fixture(10_000);
    let buy = prepare(r.clone(), &s, &a, 10_000).unwrap();
    assert_eq!(buy.terms.cex_shares, "0.02");
    assert_eq!(buy.terms.after_known_costs_usdc, "1.988");
    assert_eq!(buy.terms.allocations[0].asset, "MU.US");
    assert_eq!(buy.terms.allocations[1].quantity, "10");
    assert_eq!(buy.terms.allocations[2].quantity, "0.00089088");
    assert_eq!(buy.terms.market_valid_until_ms, 13_001);
    let sell = prepare(
        StockPlanRequest {
            direction: StockChainDirection::Sell,
            ..r.clone()
        },
        &s,
        &a,
        10_000,
    )
    .unwrap();
    assert_eq!(sell.terms.allocations[0].asset, "USDC");
    assert_eq!(sell.terms.allocations[0].quantity, "12.03202");
    assert_ne!(sell.plan_id, buy.plan_id);
    let c = &mut s.chain_costs[0];
    c.wallet_debit_lamports = Some("2000000".into());
    c.wallet_required_lamports = Some("2890880".into());
    c.native_valuation = Some(StockNativeValuation {
        native_lamports: "2000000".into(),
        replenishment: Some(StockNativeReplenishment {
            wallet_address: c.wallet_address.clone(),
            transaction: shared_types::OnchainUnsignedTransaction::SolanaVersioned {
                transaction_base64: "local-unsigned-fixture".into(),
                request_id: "local-replenishment".into(),
                router: c.quote.router.clone(),
                mode: "manual".into(),
                last_valid_block_height: None,
                expire_at_ms: Some(10_500),
            },
            transaction_fingerprint: "local-transaction".into(),
            network_fee_lamports: "7000".into(),
            wallet_outflow_lamports: "100000".into(),
            wallet_required_lamports: "990880".into(),
            minimum_credit_lamports: "2000000".into(),
            simulation_slot: 12,
            checked_at_ms: 10_000,
            valid_until_ms: 10_500,
        }),
        quote: StockDexQuote {
            output_mint: STOCK_WRAPPED_SOL.into(),
            input_raw: "215000".into(),
            output_raw: "2200000".into(),
            minimum_output_raw: "2100000".into(),
            expires_at_ms: Some(10_500),
            ..c.quote.clone()
        },
    });
    refresh_report(&mut s, &a, 10_000);
    let funded = prepare(r.clone(), &s, &a, 10_000).unwrap();
    assert_eq!(funded.terms.allocations[1].quantity, "10.215");
    assert_eq!(funded.terms.allocations[2].quantity, "0.00299088");
    assert_eq!(
        prepare(r.clone(), &s, &a, 10_000)
            .unwrap()
            .terms
            .market_valid_until_ms,
        10_500
    );
    assert!(prepare(r.clone(), &s, &a, 10_500).is_err());
    s.chain_costs[1].wallet_debit_lamports = Some("2000000".into());
    s.chain_costs[1].native_valuation = s.chain_costs[0].native_valuation.clone();
    refresh_report(&mut s, &a, 10_000);
    let sell = prepare(
        StockPlanRequest {
            direction: StockChainDirection::Sell,
            ..r.clone()
        },
        &s,
        &a,
        10_000,
    )
    .unwrap();
    assert_eq!(sell.terms.allocations.len(), 4);
    assert_eq!(sell.terms.allocations[3].asset, "USDC / SOL 补仓");
    assert_eq!(sell.terms.allocations[3].quantity, "0.215");
    assert_eq!(sell.terms.allocations[2].quantity, "0.00299088");
    let encoded = serde_json::to_string(&sell).unwrap();
    let restored: StockExecutionPlan = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        plan_store::plan_id(&restored.request, &restored.terms).unwrap(),
        sell.plan_id
    );
    for plan in [funded, sell] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("native-budget-plans.jsonl");
        let store = plan_store::PlanStore::load(Some(path.clone()), Default::default());
        let mut bad = plan.clone();
        let index = if bad.request.direction == StockChainDirection::Buy {
            1
        } else {
            3
        };
        bad.terms.allocations[index].quantity = "0".into();
        bad.plan_id = plan_store::plan_id(&bad.request, &bad.terms).unwrap();
        assert!(store.reserve(bad, 10_000).is_err());
        store.reserve(plan.clone(), 10_000).unwrap();
        drop(store);
        let store = plan_store::PlanStore::load(Some(path), Default::default());
        assert!(store.problem().is_none(), "{:?}", store.problem());
        assert_eq!(store.get(&plan.plan_id).unwrap(), plan);
    }
    s.chain_costs[0]
        .native_valuation
        .as_mut()
        .unwrap()
        .replenishment = None;
    refresh_report(&mut s, &a, 10_000);
    assert!(
        prepare(r, &s, &a, 10_000).is_err(),
        "quote-only budget must not reserve new funds"
    );
}

#[test]
fn stock_plan_changed_balance_fee_mapping_and_unknown_cost_cannot_reuse_preflight() {
    let (s, a, r) = fixture(10_000);
    for change in 0..8 {
        let (mut s, mut a) = (s.clone(), a.clone());
        match change {
            0 => a.balances.get_mut("MU.US").unwrap().available = "0".into(),
            1 => a.spot_taker_fee_bps = "20".into(),
            2 => a.liquidating = true,
            3 => s.tokens[0].contract_address = Some("different-mint".into()),
            4 => s.chain_costs[0].wallet_required_lamports = None,
            5 => s.chain_costs[0].wallet_debit_lamports = None,
            6 => s.preflight.as_mut().unwrap().directions[0].inventory[1].sufficient = Some(false),
            _ => s.connected = false,
        }
        assert!(
            prepare(r.clone(), &s, &a, 10_000).is_err(),
            "accepted changed evidence {change}"
        );
    }
    assert!(prepare(r.clone(), &s, &a, 13_001).is_err());
    let mut s = s;
    s.books[0].bid = Some("490".into());
    refresh_report(&mut s, &a, 10_000);
    assert!(prepare(r, &s, &a, 10_000)
        .unwrap_err()
        .contains("没有正差额"));
}

pub(in crate::services::backpack_stocks) fn rfq_fixture(
    now: i64,
) -> (StockMarketSnapshot, StockAccountEvidence, StockPlanRequest) {
    let (mut s, a, r) = fixture(now);
    let route = s.trading_route.as_mut().unwrap();
    route.kind = StockRouteKind::Rfq;
    route.symbol = Some("MU.US_USDC_RFQ".into());
    route.session = Some(StockSession {
        name: "local-session".into(),
        min_quantity: "0.01".into(),
        max_quantity: None,
        step_size: "0.01".into(),
    });
    s.rfq_connected = true;
    s.rfqs = vec![StockRfq {
        request: StockRfqRequest {
            request_id: "local-stock-rfq-0001".into(),
            asset: "MU.US".into(),
            side: StockRfqSide::Ask,
            quantity: "0.02".into(),
        },
        client_id: 1,
        account_fingerprint: a.fingerprint.clone(),
        symbol: "MU.US_USDC_RFQ".into(),
        rfq_id: Some("9007199254740993".into()),
        phase: StockRfqPhase::Candidate,
        candidate: Some(StockRfqCandidate {
            quote_id: "9007199254740997".into(),
            taker_price: "600".into(),
            source_at_us: now * 1000,
            received_at_ms: now,
        }),
        submission_time_ms: Some(now),
        expiry_time_ms: Some(now + 1000),
        source_at_us: Some(now * 1000),
        fill_price: None,
        executed_quantity: None,
        executed_quote_quantity: None,
        fills: vec![],
        settlement: Default::default(),
        acceptance: None,
        needs_recheck: false,
        cancel_requested: false,
        created_at_ms: now,
        updated_at_ms: now,
        problem: None,
    }];
    refresh_report(&mut s, &a, now);
    (s, a, r)
}

#[test]
fn stock_plan_rfq_binds_exact_account_candidate_and_expiry_without_accepting() {
    let (mut s, a, r) = rfq_fixture(10_000);
    let plan = prepare(r.clone(), &s, &a, 10_000).unwrap();
    assert_eq!(
        plan.terms.rfq.as_ref().unwrap().candidate.quote_id,
        "9007199254740997"
    );
    assert_eq!(plan.terms.market_valid_until_ms, 11_000);
    s.rfqs[0].account_fingerprint = "different-account".into();
    assert!(prepare(r.clone(), &s, &a, 10_000)
        .unwrap_err()
        .contains("账户已变化"));
    s.rfqs[0].cancel_requested = true;
    assert!(prepare(r, &s, &a, 10_000).is_err());
}
