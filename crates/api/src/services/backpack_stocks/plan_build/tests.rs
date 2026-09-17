use super::*;
use axum::{extract::State, routing::get, Json, Router};
use std::sync::atomic::AtomicUsize;

fn fixture(
    path: std::path::PathBuf,
) -> (
    Arc<BackpackStocks>,
    StockPlanBuildRequest,
    preflight::Inputs,
    StockChainCost,
) {
    let now = common::time::now_ms();
    let (service, old) = BackpackStocks::stock_plan_fixture(path, now);
    let service = Arc::new(service);
    let mut cost = service.snapshot().chain_costs[0].clone();
    crate::services::onchain_comparison::stock_costs::execution::tests::attach(&mut cost, true);
    let request = StockPlanBuildRequest {
        request_id: old.request_id,
        asset: old.asset,
        direction: old.direction,
        wallet_address: old.wallet_address,
        input_raw: cost.quote.input_raw.clone(),
        keyed: service.snapshot().comparison.unwrap().keyed,
    };
    let inputs = preflight::Inputs {
        fingerprint: service
            .account
            .read()
            .evidence
            .as_ref()
            .map(|e| e.fingerprint.clone()),
        wallet: Some(StockWalletEvidence {
            owner: request.wallet_address.clone(),
            mint: cost.mint.address.clone(),
            stock_raw: Some("16000".into()),
            usdc_raw: Some("25000000".into()),
            sol_lamports: Some("1000000000".into()),
            checked_at_ms: now,
            problems: vec![],
        }),
        problems: vec![],
    };
    {
        let mut snapshot = service.snapshot.write();
        snapshot.preflight = None;
        snapshot.chain_costs.clear();
        snapshot.comparison.as_mut().unwrap().buy.requested_at_ms -= 20_000;
    }
    (service, request, inputs, cost)
}

async fn no_inputs(
    _: Arc<BackpackStocks>,
    _: StockPreflightRequest,
    _: u64,
) -> Result<preflight::Inputs, String> {
    panic!("a durable retry must not reread accounts or wallets")
}
async fn no_cost(_: StockChainCostRequest, _: StockComparison) -> Result<StockChainCost, String> {
    panic!("a durable retry must not fetch or simulate a new transaction")
}

#[tokio::test]
async fn stock_plan_build_reverse_and_two_rfq_candidates_use_the_same_published_basis() {
    for (rfq, direction) in [
        (false, StockChainDirection::Sell),
        (true, StockChainDirection::Buy),
        (true, StockChainDirection::Sell),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (service, mut request, inputs, mut cost) = fixture(temp.path().join("plans.jsonl"));
        let mut service = Arc::try_unwrap(service).ok().unwrap();
        request.direction = direction;
        cost.direction = direction;
        cost.quote = direction
            .quote(service.snapshot().comparison.as_ref().unwrap())
            .unwrap()
            .clone();
        cost.quote.requested_at_ms = common::time::now_ms();
        cost.quote.received_at_ms = cost.quote.requested_at_ms;
        request.input_raw = cost.quote.input_raw.clone();
        crate::services::onchain_comparison::stock_costs::execution::tests::attach(&mut cost, true);
        if rfq {
            let now = common::time::now_ms() - 2;
            let (template, _, _) = plans::tests::rfq_fixture(now);
            service.snapshot.write().trading_route = template.trading_route;
            service.rfq_store = rfq_store::RfqStore::load(Some(temp.path().join("rfq.jsonl")));
            let base = template.rfqs[0].clone();
            service
                .rfq_subscription
                .send_replace(Some(base.account_fingerprint.clone()));
            for (index, side) in [StockRfqSide::Ask, StockRfqSide::Bid]
                .into_iter()
                .enumerate()
            {
                let mut record = base.clone();
                record.request.request_id =
                    format!("local-candidate-{}", if index == 0 { "a" } else { "z" });
                record.request.side = side;
                record.rfq_id = Some(format!("900719925474099{}", index + 3));
                record.created_at_ms += index as i64;
                record.updated_at_ms = record.created_at_ms;
                record.source_at_us = Some(record.created_at_ms * 1000);
                record.candidate.as_mut().unwrap().taker_price =
                    if index == 0 { "600" } else { "601" }.into();
                let (claimed, _) = service
                    .rfq_store
                    .claim(
                        record.request.clone(),
                        &record.account_fingerprint,
                        record.symbol.clone(),
                        record.created_at_ms,
                    )
                    .unwrap();
                record.client_id = claimed.client_id;
                service
                    .rfq_store
                    .change(&claimed.request.request_id, true, move |r| {
                        *r = record;
                        Ok(true)
                    })
                    .unwrap();
            }
            assert!(service.snapshot().rfqs[0].request.request_id.ends_with('z'));
        }
        let service = Arc::new(service);
        let snapshot = service
            .build_plan_with(
                request,
                &realtime::WsHub::default(),
                move |_, _, _| async move { Ok(inputs) },
                move |_, _| async move { Ok(cost) },
            )
            .await
            .unwrap();
        assert!(snapshot
            .preflight
            .as_ref()
            .unwrap()
            .current(&snapshot, common::time::now_ms()));
        let plan = &snapshot.plans[0];
        assert_eq!(plan.request.direction, direction);
        assert_eq!(plan.terms.rfq.is_some(), rfq);
        assert!(
            plan.rfq_acceptance.is_none()
                && plan.cex_order.is_none()
                && plan.chain_submission.is_none()
        );
        let a = service.account.read();
        let prepared = plans::prepare(
            plan.request.clone(),
            &snapshot,
            a.evidence.as_ref().unwrap(),
            common::time::now_ms(),
        )
        .unwrap();
        assert_eq!(prepared.terms.cex_instruction, plan.terms.cex_instruction);
    }
}

#[derive(Clone)]
struct Mock {
    wallet: StockWalletEvidence,
    cost: StockChainCost,
    reads: Arc<AtomicUsize>,
}

#[tokio::test]
async fn stock_plan_build_one_pass_uses_latest_market_and_durable_retry_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let (service, request, inputs, cost) = fixture(path.clone());
    let reads = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route(
            "/inventory",
            get(|State(s): State<Mock>| async move {
                assert_eq!(s.reads.fetch_add(1, Ordering::SeqCst), 0);
                Json(s.wallet)
            }),
        )
        .route(
            "/unsigned-cost",
            get(|State(s): State<Mock>| async move {
                assert_eq!(s.reads.fetch_add(1, Ordering::SeqCst), 1);
                Json(s.cost)
            }),
        )
        .with_state(Mock {
            wallet: inputs.wallet.unwrap(),
            cost,
            reads: reads.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let input_client = client.clone();
    let input_root = root.clone();
    let hub = realtime::WsHub::default();
    let mut frames = hub.subscribe(realtime::channels::STOCKS);
    let updated = service.clone();
    let snapshot = service
        .build_plan_with(
            request.clone(),
            &hub,
            move |_, _, _| async move {
                let wallet = input_client
                    .get(format!("{input_root}/inventory"))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                Ok(preflight::Inputs {
                    fingerprint: inputs.fingerprint,
                    wallet: Some(wallet),
                    problems: vec![],
                })
            },
            move |r, baseline| async move {
                assert_eq!(r.wallet_address, request.wallet_address);
                assert!(common::time::now_ms() - baseline.buy.requested_at_ms >= 20_000);
                let mut cost: StockChainCost = client
                    .get(format!("{root}/unsigned-cost"))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                let now = common::time::now_ms();
                cost.quote.requested_at_ms = now;
                cost.quote.received_at_ms = now;
                cost.checked_at_ms = now;
                cost.valid_until_ms = now + 5000;
                let mut snapshot = updated.snapshot.write();
                snapshot.books[0].bid = Some("605".into());
                snapshot.books[0].ask = Some("606".into());
                snapshot.books[0].update_id += 1;
                snapshot.books[0].source_at_ms = now;
                Ok(cost)
            },
        )
        .await;
    server.abort();
    let _ = server.await;
    let snapshot = snapshot.unwrap();
    if let Ok(output) = std::env::var("STOCK_PLAN_BUILD_CAPTURE_PATH") {
        std::fs::write(output, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();
    }
    let plan = snapshot.plans[0].clone();
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert_eq!(
        plan.request.build.as_ref().unwrap().input_raw,
        plan.terms.chain_cost.quote.input_raw
    );
    assert!(
        matches!(plan.terms.cex_instruction, Some(StockCexInstruction::OrderBook {ref limit_price, ..}) if limit_price == "605")
    );
    assert!(plan.cex_order.is_none() && plan.chain_submission.is_none());
    assert_eq!(snapshot.preflight.unwrap().price_basis.books[0].1, 2);
    let frame = frames.recv().await.unwrap();
    assert_eq!(
        frame.payload_json().unwrap()["plans"][0]["planId"],
        plan.plan_id
    );
    let bytes = std::fs::read(&path).unwrap();
    let request = plan.request.build.clone().unwrap();
    service
        .build_plan_with(request.clone(), &hub, no_inputs, no_cost)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    drop(service);
    let (restarted, _) = BackpackStocks::stock_plan_fixture(path.clone(), common::time::now_ms());
    let restarted = Arc::new(restarted);
    restarted.snapshot.write().security = None;
    let recovered = restarted
        .build_plan_with(request.clone(), &hub, no_inputs, no_cost)
        .await
        .unwrap();
    assert_eq!(recovered.plans[0].plan_id, plan.plan_id);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut changed = request.clone();
    changed.input_raw = "11000000".into();
    assert!(restarted
        .build_plan_with(changed, &hub, no_inputs, no_cost)
        .await
        .unwrap_err()
        .contains("不同参数"));
    restarted.cancel_plan(&plan.plan_id, &hub).unwrap();
    let cancelled = restarted
        .build_plan_with(request, &hub, no_inputs, no_cost)
        .await
        .unwrap();
    assert_eq!(cancelled.plans[0].phase, StockPlanPhase::Cancelled);
    assert_eq!(cancelled.plans.len(), 1);
}

#[tokio::test]
async fn stock_plan_build_rejects_expiry_inventory_changed_selection_and_mismatched_cost() {
    for case in 0..9 {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plans.jsonl");
        let (service, request, mut inputs, mut cost) = fixture(path.clone());
        let mutate = service.clone();
        match case {
            0 => cost.valid_until_ms = common::time::now_ms() - 1,
            1 => inputs.wallet.as_mut().unwrap().usdc_raw = Some("0".into()),
            2 => cost.quote.input_raw = "10000001".into(),
            3 => cost.wallet_address = "11111111111111111111111111111111".into(),
            4 => inputs.wallet.as_mut().unwrap().checked_at_ms -= 31_000,
            5 => cost.wallet_debit_lamports = None,
            6 => cost.mint.ui_multiplier = "2".into(),
            _ => {}
        }
        let result = service
            .build_plan_with(
                request,
                &realtime::WsHub::default(),
                move |_, _, _| async move { Ok(inputs) },
                move |_, _| async move {
                    if case == 7 {
                        mutate.generation.fetch_add(1, Ordering::SeqCst);
                    }
                    if case == 8 {
                        mutate
                            .account
                            .write()
                            .evidence
                            .as_mut()
                            .unwrap()
                            .balances
                            .get_mut("MU.US")
                            .unwrap()
                            .available = "0".into();
                    }
                    Ok(cost)
                },
            )
            .await;
        assert!(result.is_err(), "case {case} must not reserve");
        assert!(!path.exists(), "case {case} left a durable reservation");
        assert!(service.snapshot().plans.is_empty());
    }
}

#[tokio::test]
async fn stock_plan_build_cancelled_wait_releases_locks_and_never_reaches_cost() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let (service, request, inputs, cost) = fixture(path.clone());
    let entered = Arc::new(tokio::sync::Notify::new());
    let waiting = entered.clone();
    let first = service.clone();
    let first_request = request.clone();
    let work = tokio::spawn(async move {
        first
            .build_plan_with(
                first_request,
                &realtime::WsHub::default(),
                move |_, _, _| async move {
                    waiting.notify_one();
                    std::future::pending::<Result<preflight::Inputs, String>>().await
                },
                no_cost,
            )
            .await
    });
    entered.notified().await;
    assert!(service
        .build_plan_with(
            request.clone(),
            &realtime::WsHub::default(),
            no_inputs,
            no_cost
        )
        .await
        .unwrap_err()
        .contains("正在构建"));
    work.abort();
    assert!(work.await.unwrap_err().is_cancelled());
    assert!(!path.exists());
    service
        .build_plan_with(
            request,
            &realtime::WsHub::default(),
            move |_, _, _| async move { Ok(inputs) },
            move |_, _| async move { Ok(cost) },
        )
        .await
        .unwrap();
    assert_eq!(service.snapshot().plans.len(), 1);
}
