use super::*;
use crate::services::onchain_comparison::stock_costs::execution::tests as chain_fixture;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;

fn risk() -> trading::RiskConfig {
    trading::RiskConfig {
        live_trading_enabled: true,
        ..Default::default()
    }
}

fn reserved(path: std::path::PathBuf) -> (Arc<BackpackStocks>, StockStablecoinSubmitRequest) {
    let now = common::time::now_ms();
    let service = Arc::new(BackpackStocks::new().unwrap().with_stablecoin_store(path));
    let mut p = stablecoin_store::tests::fixture(now);
    let cost = p.cost.as_mut().unwrap();
    chain_fixture::attach(cost, true);
    p.request.wallet_address = cost.wallet_address.clone();
    p.wallet.owner = cost.wallet_address.clone();
    p.quote = cost.quote.clone();
    let p = stablecoin_preview(p.request, p.wallet, p.quote, p.cost, vec![], now).unwrap();
    assert!(p.can_reserve(now), "{:?}", p.blockers);
    let plan = service
        .stablecoin_store
        .reserve(stablecoin_store::tests::request(&p), p, now)
        .unwrap();
    (
        service,
        StockStablecoinSubmitRequest {
            plan_id: plan.plan_id,
            revision: 1,
            confirm_live: true,
        },
    )
}

struct Remote {
    cost: StockChainCost,
    mode: AtomicUsize,
    submits: AtomicUsize,
    reads: AtomicUsize,
}

async fn remote(
    cost: StockChainCost,
) -> (
    Arc<Remote>,
    String,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
) {
    let state = Arc::new(Remote {
        cost,
        mode: AtomicUsize::new(0),
        submits: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
    });
    let router = Router::new()
        .route(
            "/execute",
            post(
                |State(s): State<Arc<Remote>>, Json(body): Json<Value>| async move {
                    assert_eq!(body["requestId"], "original-stock-request");
                    assert_eq!(body["lastValidBlockHeight"], "1000");
                    assert_eq!(
                        body["signedTransaction"],
                        chain_fixture::signed(&s.cost).unwrap()
                    );
                    assert_eq!(body.as_object().unwrap().len(), 3);
                    s.submits.fetch_add(1, Ordering::SeqCst);
                    // Transport failed after the provider could have accepted the transaction.
                    (
                        StatusCode::GATEWAY_TIMEOUT,
                        Json(json!({"error":"local timeout fixture"})),
                    )
                },
            ),
        )
        .route(
            "/rpc",
            post(
                |State(s): State<Arc<Remote>>, Json(body): Json<Value>| async move {
                    s.reads.fetch_add(1, Ordering::SeqCst);
                    let mode = s.mode.load(Ordering::SeqCst);
                    let (id, mut receipt) = chain_fixture::finalized(&s.cost, mode == 2);
                    if mode == 3 {
                        receipt["meta"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] =
                            "1".into();
                    }
                    let result = match body["method"].as_str().unwrap() {
                        "getGenesisHash" => json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
                        "getSignaturesForAddress" => {
                            assert_eq!(body["params"][0], s.cost.wallet_address);
                            assert_eq!(body["params"][1]["commitment"], "finalized");
                            if mode == 0 {
                                json!([])
                            } else {
                                json!([{"signature":id,"slot":13}])
                            }
                        }
                        "getTransaction" => {
                            assert_eq!(body["params"][0], id);
                            assert_eq!(body["params"][1]["encoding"], "base64");
                            assert_eq!(body["params"][1]["commitment"], "finalized");
                            if mode == 0 {
                                Value::Null
                            } else {
                                receipt
                            }
                        }
                        other => panic!("unexpected remote mutation {other}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                },
            ),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (
        state,
        root,
        reqwest::Client::builder().no_proxy().build().unwrap(),
        task,
    )
}

#[tokio::test]
async fn stock_stablecoin_submission_timeout_restart_queries_original_without_resending() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("stablecoin.jsonl");
    let (service, request) = reserved(path.clone());
    let cost = service
        .stablecoin_store
        .get(&request.plan_id)
        .unwrap()
        .preview
        .cost
        .unwrap();
    let (remote, root, client, server) = remote(cost.clone()).await;
    let hub = realtime::WsHub::new(8);
    let mut frames = hub.subscribe(realtime::channels::STOCKS);
    let url = format!("{root}/execute");
    let http = client.clone();
    let journal = path.clone();
    let sent = service
        .submit_stablecoin_with(
            request.clone(),
            hub.clone(),
            risk,
            |_| Box::pin(async { Ok(()) }),
            chain_fixture::signed,
            move |c, s| {
                Box::pin(async move {
                    let rows = std::fs::read_to_string(journal).unwrap();
                    assert!(rows.lines().last().unwrap().contains("submission_unknown"));
                    chain::submit_with(&http, &url, None, c, s).await
                })
            },
        )
        .await
        .unwrap();
    let row = &sent.stablecoin_plans[0];
    assert_eq!(row.phase, StockStablecoinPlanPhase::SubmissionUnknown);
    assert!(!row.submission.as_ref().unwrap().provider_acknowledged);
    assert!(row.holds_funds(row.preview.valid_until_ms + 60_000));
    assert_eq!(remote.submits.load(Ordering::SeqCst), 1);
    let frame = frames.recv().await.unwrap();
    assert_eq!(
        frame.payload_json().unwrap()["stablecoinPlans"][0]["phase"],
        "submission_unknown"
    );
    assert!(service
        .cancel_stablecoin_plan(
            StockPlanRevisionRequest {
                plan_id: request.plan_id.clone(),
                revision: row.revision
            },
            &hub
        )
        .is_err());
    let retry = service
        .submit_stablecoin_with(
            request.clone(),
            hub.clone(),
            || panic!("retry must not read live mode or credentials"),
            |_| Box::pin(async { panic!("retry must not simulate") }),
            chain_fixture::signed,
            |_, _| Box::pin(async { panic!("retry must not submit") }),
        )
        .await
        .unwrap();
    assert_eq!(retry.stablecoin_plans, sent.stablecoin_plans);
    let before = std::fs::read(&path).unwrap();
    drop(service);
    let service = Arc::new(
        BackpackStocks::new()
            .unwrap()
            .with_stablecoin_store(path.clone()),
    );
    assert!(service.stablecoin_store.problem().is_none());
    assert!(service
        .wallet_claims
        .check(
            "solana",
            &cost.wallet_address,
            row.preview.valid_until_ms + 60_000
        )
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let url = format!("{root}/rpc");
    let http = client.clone();
    let pending = service
        .recheck_stablecoin_with(&request.plan_id, &hub, move |c, s| {
            Box::pin(async move { chain::lookup_with(&http, &url, c, s).await })
        })
        .await
        .unwrap();
    assert_eq!(
        pending.stablecoin_plans[0].phase,
        StockStablecoinPlanPhase::SubmissionUnknown
    );
    assert!(service
        .recheck_stablecoin_with(&request.plan_id, &hub, |_, _| Box::pin(async {
            panic!("cooldown must not query")
        }))
        .await
        .is_err());
    tokio::time::sleep(Duration::from_millis(5050)).await;
    remote.mode.store(1, Ordering::SeqCst);
    let url = format!("{root}/rpc");
    let http = client.clone();
    let completed = service
        .recheck_stablecoin_with(&request.plan_id, &hub, move |c, s| {
            Box::pin(async move { chain::lookup_with(&http, &url, c, s).await })
        })
        .await
        .unwrap();
    let p = &completed.stablecoin_plans[0];
    assert_eq!(p.phase, StockStablecoinPlanPhase::Completed);
    let receipt = p.submission.as_ref().unwrap().receipt.as_ref().unwrap();
    assert_eq!(
        stablecoin_change(receipt, STOCK_SOLANA_USDT),
        Some(-10_000_000)
    );
    assert_eq!(
        stablecoin_change(receipt, shared_types::stocks::comparison::SOLANA_USDC),
        Some(9_950_000)
    );
    assert_eq!(receipt.network_fee_lamports, "7000");
    assert_eq!(
        receipt.wallet_native_change_lamports, "0",
        "sponsored fee is not charged to the wallet"
    );
    assert!(service
        .wallet_claims
        .check("solana", &cost.wallet_address, common::time::now_ms())
        .is_ok());
    let bytes = std::fs::read(&path).unwrap();
    service
        .recheck_stablecoin_with(&request.plan_id, &hub, |_, _| {
            Box::pin(async { panic!("terminal receipt must not query") })
        })
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    drop(service);
    let restored = BackpackStocks::new().unwrap().with_stablecoin_store(path);
    assert_eq!(
        restored.snapshot().stablecoin_plans,
        completed.stablecoin_plans
    );
    assert!(restored.stablecoin_store.problem().is_none());
    assert_eq!(remote.submits.load(Ordering::SeqCst), 1);
    if let Ok(path) = std::env::var("STOCK_STABLECOIN_EXECUTION_CAPTURE_PATH") {
        std::fs::write(
            path,
            serde_json::to_vec(&json!({"pending":pending,"completed":completed})).unwrap(),
        )
        .unwrap();
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn stock_stablecoin_failed_receipt_keeps_fees_and_mismatched_credit_keeps_claim() {
    for (mode, expected, held) in [
        (2, StockStablecoinPlanPhase::Failed, false),
        (3, StockStablecoinPlanPhase::NeedsReview, true),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("stablecoin.jsonl");
        let (service, r) = reserved(path.clone());
        let hub = realtime::WsHub::new(8);
        let cost = service
            .stablecoin_store
            .get(&r.plan_id)
            .unwrap()
            .preview
            .cost
            .unwrap();
        let (remote, root, client, server) = remote(cost.clone()).await;
        remote.mode.store(mode, Ordering::SeqCst);
        let signed = chain_fixture::signed(&cost).unwrap();
        service
            .stablecoin_store
            .begin(&r, &signed, common::time::now_ms())
            .unwrap();
        let url = format!("{root}/rpc");
        let snapshot = service
            .recheck_stablecoin_with(&r.plan_id, &hub, move |c, s| {
                Box::pin(async move { chain::lookup_with(&client, &url, c, s).await })
            })
            .await
            .unwrap();
        let p = &snapshot.stablecoin_plans[0];
        assert_eq!(p.phase, expected);
        let receipt = p.submission.as_ref().unwrap().receipt.as_ref().unwrap();
        assert_eq!(receipt.network_fee_lamports, "7000");
        assert_eq!(p.holds_funds(p.preview.valid_until_ms + 1), held);
        assert_eq!(
            service
                .wallet_claims
                .check("solana", &cost.wallet_address, common::time::now_ms())
                .is_err(),
            held
        );
        if mode == 2 {
            assert_eq!(stablecoin_change(receipt, STOCK_SOLANA_USDT), Some(0));
        }
        if mode == 3 {
            assert_eq!(
                stablecoin_change(receipt, shared_types::stocks::comparison::SOLANA_USDC),
                Some(1)
            );
        }
        assert_eq!(remote.submits.load(Ordering::SeqCst), 0);
        drop(service);
        let restored = BackpackStocks::new().unwrap().with_stablecoin_store(path);
        assert_eq!(
            restored.snapshot().stablecoin_plans,
            snapshot.stablecoin_plans
        );
        assert_eq!(
            restored
                .wallet_claims
                .check("solana", &cost.wallet_address, common::time::now_ms())
                .is_err(),
            held
        );
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn stock_stablecoin_request_disconnect_keeps_the_single_submission_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let (service, r) = reserved(tmp.path().join("stablecoin.jsonl"));
    let hub = realtime::WsHub::new(8);
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let task = {
        let service = service.clone();
        let r = r.clone();
        let hub = hub.clone();
        let entered = entered.clone();
        let release = release.clone();
        tokio::spawn(async move {
            service
                .submit_stablecoin_with(
                    r,
                    hub,
                    risk,
                    |_| Box::pin(async { Ok(()) }),
                    chain_fixture::signed,
                    move |_, _| {
                        Box::pin(async move {
                            entered.notify_one();
                            release.notified().await;
                            Ok(None)
                        })
                    },
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    assert!(service
        .stablecoin_store
        .get(&r.plan_id)
        .unwrap()
        .submission
        .is_some());
    service
        .submit_stablecoin_with(
            r.clone(),
            hub,
            risk,
            |_| Box::pin(async { panic!() }),
            chain_fixture::signed,
            |_, _| Box::pin(async { panic!("detached owner must prevent re-send") }),
        )
        .await
        .unwrap();
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if service
                .stablecoin_store
                .get(&r.plan_id)
                .unwrap()
                .submission
                .unwrap()
                .provider_acknowledged
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let p = service.stablecoin_store.get(&r.plan_id).unwrap();
    assert_eq!(
        p.phase,
        StockStablecoinPlanPhase::SubmissionUnknown,
        "ACK cannot mark funds received"
    );
}

#[tokio::test]
async fn stock_stablecoin_submission_rechecks_mode_revision_and_preflight_before_signing() {
    let tmp = tempfile::tempdir().unwrap();
    let (service, r) = reserved(tmp.path().join("stablecoin.jsonl"));
    let hub = realtime::WsHub::new(8);
    fn no_verify(_: &StockChainCost) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { panic!("must not read remote data") })
    }
    let no_sign = |_: &StockChainCost| -> Result<String, String> { panic!("must not sign") };
    for request in [
        StockStablecoinSubmitRequest {
            confirm_live: false,
            ..r.clone()
        },
        StockStablecoinSubmitRequest {
            revision: 0,
            ..r.clone()
        },
    ] {
        assert!(service
            .submit_stablecoin_with(request, hub.clone(), risk, no_verify, no_sign, |_, _| {
                Box::pin(async { panic!() })
            })
            .await
            .is_err());
    }
    assert!(service
        .submit_stablecoin_with(
            r.clone(),
            hub.clone(),
            trading::RiskConfig::default,
            no_verify,
            no_sign,
            |_, _| Box::pin(async { panic!() })
        )
        .await
        .unwrap_err()
        .contains("模拟"));
    assert!(service
        .submit_stablecoin_with(
            r.clone(),
            hub.clone(),
            risk,
            |_| Box::pin(async { Err("original simulation failed".into()) }),
            no_sign,
            |_, _| Box::pin(async { panic!() })
        )
        .await
        .unwrap_err()
        .contains("simulation"));
    let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let switch = killed.clone();
    let result = service
        .submit_stablecoin_with(
            r.clone(),
            hub,
            risk_with(killed),
            move |_| {
                Box::pin(async move {
                    switch.store(true, Ordering::SeqCst);
                    Ok(())
                })
            },
            no_sign,
            |_, _| Box::pin(async { panic!() }),
        )
        .await;
    assert!(result.unwrap_err().contains("急停"));
    assert!(service
        .stablecoin_store
        .get(&r.plan_id)
        .unwrap()
        .submission
        .is_none());
}

fn risk_with(killed: Arc<std::sync::atomic::AtomicBool>) -> impl Fn() -> trading::RiskConfig {
    move || trading::RiskConfig {
        kill_switch_active: killed.load(Ordering::SeqCst),
        ..risk()
    }
}
