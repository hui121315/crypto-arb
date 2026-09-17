use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims};
use std::{path::Path, sync::atomic::AtomicUsize};

pub(in crate::services::backpack_stocks) fn fixture(
    path: &Path,
    direction: StockChainDirection,
) -> (
    Arc<BackpackStocks>,
    StockPeerPlanRequest,
    StockPeerAccount,
    StockWalletEvidence,
    StockChainCost,
) {
    let now = common::time::now_ms();
    let service = alerts::tests::peer_service(now);
    let peer = service.snapshot().peer;
    let (mut snapshot, _, old) = plans::tests::fixture(now);
    snapshot.peer = peer;
    let mut cost = snapshot
        .chain_costs
        .iter()
        .find(|c| c.direction == direction)
        .unwrap()
        .clone();
    stock_costs::execution::tests::attach(&mut cost, true);
    // Preserve the original taker quote, then discard previous account/cost UI samples.
    *snapshot.comparison.as_mut().unwrap() = {
        let mut c = snapshot.comparison.clone().unwrap();
        match direction {
            StockChainDirection::Buy => c.buy = cost.quote.clone(),
            StockChainDirection::Sell => c.sell = Some(cost.quote.clone()),
        };
        c
    };
    snapshot.preflight = None;
    snapshot.chain_costs.clear();
    *service.snapshot.write() = snapshot;
    let aggregator = Arc::new(exchange::Aggregator::new());
    aggregator.register(Arc::new(alerts::tests::PeerWsFixture(Arc::new(
        AtomicUsize::new(0),
    ))));
    let service = Arc::new(
        service
            .with_peer_feed(
                aggregator,
                Arc::new(crate::services::market_subscriptions::MarketSubscriptions::load(None)),
            )
            .with_peer_plan_store(path.into()),
    );
    let request = StockPeerPlanRequest {
        request_id: "local-stock-peer-plan-0001".into(),
        asset: old.asset,
        selection: service.snapshot().peer.unwrap().selection,
        direction,
        wallet_address: old.wallet_address,
        input_raw: cost.quote.input_raw.clone(),
        keyed: service.snapshot().comparison.unwrap().keyed,
    };
    let account = StockPeerAccount {
        venue: "kraken".into(),
        native_symbol: "MUx/USD".into(),
        stock_asset: "MUx".into(),
        quote_asset: "USD".into(),
        stock_available: Some("10".into()),
        quote_available: Some("100".into()),
        usdc_available: Some("0".into()),
        stock_taker_pct: Some("0.1".into()),
        fx_taker_pct: Some("0.2".into()),
        observed_at_ms: now,
        sources: vec!["local fixture".into()],
        problems: vec![],
    };
    let wallet = StockWalletEvidence {
        owner: request.wallet_address.clone(),
        mint: cost.mint.address.clone(),
        stock_raw: Some("16000".into()),
        usdc_raw: Some("25000000".into()),
        sol_lamports: Some("1000000000".into()),
        checked_at_ms: now,
        problems: vec![],
    };
    (service, request, account, wallet, cost)
}
async fn no_inputs(
    _: Arc<BackpackStocks>,
    _: StockPeerPlanRequest,
    _: u64,
    _: Arc<dyn exchange::ExchangeAdapter>,
) -> Result<(StockPeerAccount, StockWalletEvidence), String> {
    panic!("durable retry must not reread funds")
}
async fn no_cost(_: StockChainCostRequest, _: StockComparison) -> Result<StockChainCost, String> {
    panic!("durable retry must not request new quote")
}

#[tokio::test]
async fn stock_peer_plan_both_directions_reserve_native_cash_cancel_and_restore_without_resubmit() {
    for direction in [StockChainDirection::Buy, StockChainDirection::Sell] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, r, a, w, c) = fixture(&path, direction);
        let original_tx = c.transaction.clone();
        let hub = realtime::WsHub::default();
        let snapshot = s
            .build_peer_plan_with(
                r.clone(),
                &hub,
                |_, _, _, _| async { Ok((a, w)) },
                |_, _| async { Ok(c) },
            )
            .await
            .unwrap();
        assert!(snapshot.plans.is_empty() && snapshot.peer_order_checks.is_empty());
        let p = &snapshot.peer_plans[0];
        assert_eq!(p.terms.basis.chain_cost.transaction, original_tx);
        assert_eq!(
            p.terms.allocations[0].asset,
            if direction == StockChainDirection::Buy {
                "MUx"
            } else {
                "USD"
            }
        );
        assert_eq!(p.terms.draft.quote_asset, "USD");
        let mut older = p.terms.basis.clone();
        older.peer.quote.as_mut().unwrap().received_at_ms = p.terms.created_at_ms - 2500;
        let limited = prepare_peer_plan_terms(
            &r,
            older,
            p.terms.account_fingerprint.clone(),
            p.terms.created_at_ms,
        )
        .unwrap();
        assert_eq!(
            limited.market_valid_until_ms,
            p.terms.created_at_ms + 500,
            "original receive freshness also bounds plan validity"
        );
        assert_eq!(p.terms.basis.account.usdc_available.as_deref(), Some("0"));
        assert!(s
            .wallet_claims
            .check("solana", &r.wallet_address, common::time::now_ms())
            .is_err());
        if direction == StockChainDirection::Sell {
            if let Ok(path) = std::env::var("STOCK_PEER_PLAN_CAPTURE_PATH") {
                std::fs::write(path, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();
            }
        }
        let original = std::fs::read(&path).unwrap();
        let repeated = s
            .build_peer_plan_with(r.clone(), &hub, no_inputs, no_cost)
            .await
            .unwrap();
        assert_eq!(&repeated.peer_plans[0], p);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        drop(s);
        let s = Arc::new(
            BackpackStocks::new()
                .unwrap()
                .with_peer_plan_store(path.clone()),
        );
        assert_eq!(&s.snapshot().peer_plans[0], p);
        assert!(s
            .wallet_claims
            .check("solana", &r.wallet_address, common::time::now_ms())
            .is_err());
        // No selected market or credentials after restart are needed to retrieve or cancel the original.
        assert_eq!(
            &s.build_peer_plan_with(r.clone(), &hub, no_inputs, no_cost)
                .await
                .unwrap()
                .peer_plans[0],
            p
        );
        assert!(s
            .cancel_peer_plan(
                StockPlanRevisionRequest {
                    plan_id: p.plan_id.clone(),
                    revision: 99
                },
                &hub
            )
            .is_err());
        let cancelled = s
            .cancel_peer_plan(
                StockPlanRevisionRequest {
                    plan_id: p.plan_id.clone(),
                    revision: 1,
                },
                &hub,
            )
            .unwrap();
        assert_eq!(cancelled.peer_plans[0].phase, StockPeerPlanPhase::Cancelled);
        assert!(s
            .wallet_claims
            .check("solana", &r.wallet_address, common::time::now_ms())
            .is_ok());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            s.build_peer_plan_with(r.clone(), &hub, no_inputs, no_cost)
                .await
                .unwrap()
                .peer_plans,
            cancelled.peer_plans
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let mut changed = r.clone();
        changed.input_raw = "10000001".into();
        assert!(s
            .build_peer_plan_with(changed, &hub, no_inputs, no_cost)
            .await
            .is_err());
        drop(s);
        let s = BackpackStocks::new().unwrap().with_peer_plan_store(path);
        assert_eq!(s.snapshot().peer_plans, cancelled.peer_plans);
        assert!(s.snapshot().peer_plan_problem.is_none());
    }
}

#[tokio::test]
async fn stock_peer_plan_guards_original_input_fee_freshness_identity_and_native_balance() {
    for case in 0..8 {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, r, mut a, mut w, mut c) = fixture(&path, StockChainDirection::Sell);
        match case {
            0 => a.quote_available = Some("0".into()),
            1 => a.quote_asset = "USDC".into(),
            2 => a.stock_taker_pct = None,
            3 => w.sol_lamports = None,
            4 => c.quote.input_raw = "15999".into(),
            5 => c.wallet_address = "11111111111111111111111111111111".into(),
            6 => a.observed_at_ms -= 16_000,
            _ => c.transaction = None,
        }
        let result = s
            .build_peer_plan_with(
                r,
                &realtime::WsHub::default(),
                |_, _, _, _| async { Ok((a, w)) },
                |_, _| async { Ok(c) },
            )
            .await;
        assert!(result.is_err(), "case {case}");
        assert!(!path.exists(), "case {case} persisted an invalid plan");
    }
    let temp = tempfile::tempdir().unwrap();
    let (s, r, a, w, _) = fixture(&temp.path().join("peer.jsonl"), StockChainDirection::Buy);
    let changed = s.clone();
    let result = s
        .build_peer_plan_with(
            r,
            &realtime::WsHub::default(),
            move |_, _, _, _| async move {
                changed.peer_feed.as_ref().unwrap().0.register(Arc::new(
                    alerts::tests::PeerWsFixture(Arc::new(AtomicUsize::new(0))),
                ));
                Ok((a, w))
            },
            no_cost,
        )
        .await;
    assert!(result.unwrap_err().contains("账户配置"));
}

#[tokio::test]
async fn stock_peer_plan_shared_claims_expiry_and_corrupt_restart_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("peer.jsonl");
    let (s, r, a, w, c) = fixture(&path, StockChainDirection::Buy);
    let now = common::time::now_ms();
    let owner = Owner::new(Module::CrossChain, "other-module");
    s.wallet_claims
        .commit(
            owner.clone(),
            Some(Hold::wallet("solana", &r.wallet_address, None).unwrap()),
            now,
            || Ok(()),
        )
        .unwrap();
    assert!(s
        .build_peer_plan_with(r.clone(), &realtime::WsHub::default(), no_inputs, no_cost)
        .await
        .is_err());
    s.wallet_claims.commit(owner, None, now, || Ok(())).unwrap();
    let snap = s
        .build_peer_plan_with(
            r.clone(),
            &realtime::WsHub::default(),
            |_, _, _, _| async { Ok((a, w)) },
            |_, _| async { Ok(c) },
        )
        .await
        .unwrap();
    let p = snap.peer_plans[0].clone();
    // Different wallet, same configured venue account still conflicts.
    let hold = Hold::wallet("solana", "11111111111111111111111111111111", None)
        .unwrap()
        .with_account("kraken_stocks", "configured-account")
        .unwrap();
    assert!(s
        .wallet_claims
        .commit(
            Owner::new(Module::Execution, "other-wallet"),
            Some(hold),
            now,
            || panic!("must not persist competing account claim")
        )
        .is_err());
    assert!(s
        .wallet_claims
        .check("solana", &r.wallet_address, p.terms.reserved_until_ms)
        .is_ok());
    assert_eq!(s.peer_plan_store.previous(&r).unwrap(), Some(p.clone()));
    drop(s);
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    f.write_all(b"{\"version\":1").unwrap();
    f.sync_all().unwrap();
    drop(f);
    let before = std::fs::read(&path).unwrap();
    let claims = Arc::new(WalletClaims::default());
    let s = BackpackStocks::new()
        .unwrap()
        .with_wallet_claims(claims.clone())
        .with_peer_plan_store(path.clone());
    assert_eq!(s.snapshot().peer_plans, vec![p]);
    assert!(s
        .snapshot()
        .peer_plan_problem
        .unwrap()
        .contains("尾部不完整"));
    assert!(claims
        .check("solana", &r.wallet_address, now + 60_000)
        .is_err());
    assert!(s.peer_plan_store.previous(&r).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
