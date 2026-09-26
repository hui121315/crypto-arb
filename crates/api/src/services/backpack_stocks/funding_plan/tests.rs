use super::*;
use axum::{extract::Query, http::HeaderMap, routing::get, Json, Router};
use serde_json::json;
use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

pub(in crate::services::backpack_stocks) fn inputs(
    now: i64,
) -> (
    StockFundingPlanRequest,
    StockMarketSnapshot,
    StockAccountEvidence,
    StockWalletEvidence,
) {
    let (mut s, a, r) = plans::tests::fixture(now);
    s.funding_assets =
        protocol::asset_context(include_bytes!("../funding/fixtures/assets.json"), "MU.US")
            .unwrap()
            .1;
    let w = StockWalletEvidence {
        owner: r.wallet_address.clone(),
        mint: s.comparison.as_ref().unwrap().mint.address.clone(),
        stock_raw: Some("0".into()),
        usdc_raw: Some("0".into()),
        sol_lamports: Some("100000000".into()),
        checked_at_ms: now,
        problems: vec![],
    };
    let r = StockFundingPlanRequest {
        source_plan: None,
        request_id: "local-funding-plan-0001".into(),
        security_asset: "MU.US".into(),
        funding_asset: "USDC".into(),
        direction: StockChainDirection::Buy,
        target: StockFundingTarget::Solana,
        wallet_address: r.wallet_address,
        preflight_at_ms: now,
    };
    (r, s, a, w)
}

pub(in crate::services::backpack_stocks) fn fixture(now: i64) -> StockFundingPlan {
    let (r, s, a, w) = inputs(now);
    let cap = StockWithdrawalCapacity {
        asset: r.funding_asset.clone(),
        quantity: "25".into(),
        checked_at_ms: now,
    };
    prepare(
        r,
        &s,
        &a,
        &w,
        &s.preflight.as_ref().unwrap().directions,
        Some(cap),
        None,
        now,
    )
    .unwrap()
}

#[test]
fn stock_funding_plan_exact_units_capacity_and_corporate_action_guards() {
    let now = 10_000;
    let (r, s, a, w) = inputs(now);
    let rows = &s.preflight.as_ref().unwrap().directions;
    let cap = StockWithdrawalCapacity {
        asset: "USDC".into(),
        quantity: "25".into(),
        checked_at_ms: now,
    };
    let p = prepare(r.clone(), &s, &a, &w, rows, Some(cap.clone()), None, now).unwrap();
    assert_eq!(p.terms.quantity, "10.5");
    assert_eq!(p.terms.source_budget, "11");
    assert_eq!(p.terms.minimum_credit_raw, "10000000");
    assert_eq!(p.terms.valid_until_ms, now + 30_000);
    for case in 0..8 {
        let mut s = s.clone();
        let mut cap = cap.clone();
        let mut a = a.clone();
        let mut w = w.clone();
        match case {
            0 => cap.quantity = "10.99".into(),
            1 => cap.asset = "USDT".into(),
            2 => {
                s.funding_assets
                    .iter_mut()
                    .find(|t| t.asset == "USDC")
                    .unwrap()
                    .tokens[0]
                    .withdrawal_fee = None
            }
            3 => {
                s.funding_assets
                    .iter_mut()
                    .find(|t| t.asset == "USDC")
                    .unwrap()
                    .tokens[0]
                    .withdraw_enabled = Some(false)
            }
            4 => w.usdc_raw = None,
            5 => a.balances.get_mut("USDC").unwrap().available = "10.99".into(),
            6 => s.comparison.as_mut().unwrap().mint.next_change_at_ms = Some(now),
            _ => {
                s.funding_assets
                    .iter_mut()
                    .find(|t| t.asset == "USDC")
                    .unwrap()
                    .tokens[0]
                    .contract_address = Some("wrong".into())
            }
        }
        assert!(
            prepare(r.clone(), &s, &a, &w, rows, Some(cap), None, now).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn stock_funding_plan_stock_deposit_rounds_raw_units_and_does_not_depend_on_withdrawal_flag() {
    let now = 10_000;
    let (mut r, mut s, mut a, mut w) = inputs(now);
    r.target = StockFundingTarget::Backpack;
    r.funding_asset = "MU.US".into();
    a.balances.get_mut("MU.US").unwrap().available = "0".into();
    w.stock_raw = Some("1000000".into());
    s.tokens[0].withdraw_enabled = Some(false);
    s.tokens[0].minimum_deposit = Some("0.0006".into());
    let a0 = StockDepositAddress {
        asset: "MU.US".into(),
        address: bs58::encode([9; 32]).into_string(),
        blockchain: "Solana".into(),
        account_fingerprint: a.fingerprint.clone(),
        checked_at_ms: now,
    };
    let p = prepare(
        r.clone(),
        &s,
        &a,
        &w,
        &s.preflight.as_ref().unwrap().directions,
        None,
        Some(a0.clone()),
        now,
    )
    .unwrap();
    assert_eq!(p.terms.quantity, "0.02");
    assert_eq!(p.terms.minimum_credit_raw, "16000");
    assert_eq!(p.terms.source_budget, "0.02");
    for case in 0..4 {
        let mut address = a0.clone();
        match case {
            0 => address.account_fingerprint = "other".into(),
            1 => address.address = w.owner.clone(),
            2 => address.blockchain = "Ethereum".into(),
            _ => address.checked_at_ms = now + 1,
        }
        assert!(prepare(
            r.clone(),
            &s,
            &a,
            &w,
            &s.preflight.as_ref().unwrap().directions,
            None,
            Some(address),
            now
        )
        .is_err());
    }
}

#[test]
fn stock_funding_plan_restore_recomputes_amounts_and_rejects_rehashed_bad_terms() {
    let plan = fixture(10_000);
    for case in 0..12 {
        let mut p = plan.clone();
        match case {
            0 => p.terms.quantity = "9.5".into(),
            1 => p.terms.source_budget = "10.5".into(),
            2 => p.terms.minimum_credit_raw = "9999999".into(),
            3 => p.terms.destination = bs58::encode([9; 32]).into_string(),
            4 => p.terms.withdrawal_capacity = None,
            5 => p.terms.withdrawal_capacity.as_mut().unwrap().quantity = "10".into(),
            6 => p.terms.need.source_spare = Some("26".into()),
            7 => p.terms.need.required = Some("12".into()),
            8 => p.terms.mint.next_change_at_ms = Some(10_001),
            9 => p.terms.security.cusip = None,
            10 => {
                p.terms.token.contract_address = Some("wrong".into());
                p.terms.need.token = Some(p.terms.token.clone());
            }
            _ => p.terms.need.target = "Backpack".into(),
        }
        p.plan_id = plan_id(&p.request, &p.terms).unwrap();
        assert!(validate(&p).is_err(), "case {case}");
    }
    let (mut r, mut s, _, _) = inputs(10_000);
    r.funding_asset = "MU.US".into();
    s.comparison.as_mut().unwrap().mint.ui_multiplier = "1.3333".into();
    let mut need = plan.terms.need;
    need.asset = r.funding_asset.clone();
    need.shortfall = Some("0.000001".into());
    let mut token = s.tokens[0].clone();
    token.minimum_withdrawal = Some("0".into());
    token.withdrawal_fee = Some("0.0006".into());
    need.token = Some(token);
    let (quantity, budget, raw) =
        quantities(&r, &need, &s.comparison.as_ref().unwrap().mint).unwrap();
    assert_eq!(raw, 1);
    assert_eq!(
        exact(quantity),
        "0.000602",
        "fractional share amount must first cover an entire raw unit"
    );
    assert_eq!(exact(budget), "0.001202");
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn stock_funding_readonly_official_capacity_to_durable_plan_and_ws_cancel_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("funding.jsonl");
    let now = common::time::now_ms();
    let (request, mut snapshot, account, wallet) = inputs(now);
    let mut service = BackpackStocks::stock_plan_fixture(tmp.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    let body = Arc::new(Mutex::new(
        json!({"symbol":"USDC","autoBorrow":false,"autoLendRedeem":false,"maxWithdrawalQuantity":"25"}),
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let reply = body.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    service.root = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route(
        "/api/v1/account/limits/withdrawal",
        get(
            move |headers: HeaderMap, Query(q): Query<BTreeMap<String, String>>| {
                let reply = reply.clone();
                let calls = calls.clone();
                async move {
                    assert_eq!(
                        q,
                        BTreeMap::from([
                            ("autoBorrow".into(), "false".into()),
                            ("autoLendRedeem".into(), "false".into()),
                            ("symbol".into(), "USDC".into())
                        ])
                    );
                    rfq_tests::signed(&headers, "maxWithdrawalQuantity", q);
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(reply.lock().clone())
                }
            },
        ),
    );
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap()
    }));
    let funding = shared_types::stocks::funding::evaluate_funding(
        &snapshot,
        &snapshot.preflight.as_ref().unwrap().directions,
        Some(&account),
        Some(&wallet),
        now,
    );
    snapshot.preflight.as_mut().unwrap().funding = funding;
    snapshot.preflight.as_mut().unwrap().wallet_address = Some(wallet.owner.clone());
    *service.snapshot.write() = snapshot;
    service.account.write().evidence = Some(account.clone());
    let service = Arc::new(service);
    let hub = realtime::WsHub::new(8);
    let mut frames = hub.subscribe(realtime::channels::STOCKS);
    let saved = service
        .build_funding_plan_with(request.clone(), &hub, move |_, read, _| async move {
            assert_eq!(read.asset, "MU.US");
            assert_eq!(read.wallet_address.as_deref(), Some(wallet.owner.as_str()));
            Ok(preflight::Inputs {
                fingerprint: Some(account.fingerprint),
                wallet: Some(wallet),
                problems: vec![],
            })
        })
        .await
        .unwrap();
    let plan = saved.funding_plans[0].clone();
    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        frame.payload_json().unwrap()["fundingPlans"][0]["planId"],
        plan.plan_id
    );
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        service
            .build_funding_plan(request, &hub)
            .await
            .unwrap()
            .funding_plans[0],
        plan
    );
    assert_eq!(observed.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    for reply in [
        json!({"symbol":"USDC","autoBorrow":true,"autoLendRedeem":false,"maxWithdrawalQuantity":"25"}),
        json!({"symbol":"USDT","autoBorrow":false,"autoLendRedeem":false,"maxWithdrawalQuantity":"25"}),
        json!({"symbol":"USDC","autoBorrow":false,"autoLendRedeem":false}),
    ] {
        *body.lock() = reply;
        assert!(service
            .funding_endpoints(&plan.request, &rfq_tests::keys().unwrap())
            .await
            .is_err());
    }
    let result = service
        .cancel_funding_plan(
            StockPlanRevisionRequest {
                plan_id: plan.plan_id.clone(),
                revision: 1,
            },
            &hub,
        )
        .unwrap();
    assert_eq!(
        result.funding_plans[0].phase,
        StockFundingPlanPhase::Cancelled
    );
    assert!(result.plans.is_empty());
    assert!(service.rfq_worker.lock().is_none());
    assert_eq!(observed.load(Ordering::SeqCst), 4);
}
