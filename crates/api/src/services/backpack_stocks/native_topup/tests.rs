use super::*;
use axum::{extract::State, routing::post, Json, Router};
use chain::tests::{attach, attach_variant, finalized, finalized_native, signed};
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;

pub(in crate::services::backpack_stocks) fn valuation(
    main: &StockChainCost,
    native: u64,
    input: u64,
    now: i64,
    salt: u8,
) -> StockNativeValuation {
    let mut cost = main.clone();
    cost.quote = StockDexQuote {
        input_mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
        output_mint: STOCK_WRAPPED_SOL.into(),
        input_raw: input.to_string(),
        output_raw: (native + 7000).to_string(),
        minimum_output_raw: (native + 7000).to_string(),
        requested_at_ms: now,
        received_at_ms: now,
        expires_at_ms: None,
        ..main.quote.clone()
    };
    attach_variant(&mut cost, salt);
    StockNativeValuation {
        native_lamports: native.to_string(),
        quote: cost.quote,
        replenishment: Some(StockNativeReplenishment {
            wallet_address: cost.wallet_address,
            transaction: cost.transaction.unwrap(),
            transaction_fingerprint: cost.transaction_fingerprint,
            network_fee_lamports: "7000".into(),
            wallet_outflow_lamports: "7000".into(),
            wallet_required_lamports: "897880".into(),
            minimum_credit_lamports: native.to_string(),
            simulation_slot: 13,
            checked_at_ms: now,
            valid_until_ms: now + 5000,
        }),
    }
}

fn setup(path: std::path::PathBuf) -> (BackpackStocks, StockExecutionPlan) {
    let now = common::time::now_ms() - 20_000;
    let (service, request) = BackpackStocks::stock_plan_fixture(path, now);
    {
        let mut snapshot = service.snapshot.write();
        let cost = &mut snapshot.chain_costs[0];
        attach(cost, false);
        cost.native_valuation = Some(valuation(cost, 7000, 50_000, now, 1));
        snapshot.comparison.as_mut().unwrap().buy = snapshot.chain_costs[0].quote.clone();
        plans::tests::refresh_report(
            &mut snapshot,
            service.account.read().evidence.as_ref().unwrap(),
            now,
        );
    }
    let plan = plans::prepare(
        request,
        &service.snapshot.read(),
        service.account.read().evidence.as_ref().unwrap(),
        now,
    )
    .unwrap();
    service.plan_store.reserve(plan.clone(), now).unwrap();
    service
        .plan_store
        .begin_pair(
            &plan.plan_id,
            &plan.terms.account_fingerprint,
            &signed(&plan.terms.chain_cost).unwrap(),
            None,
            now + 1,
        )
        .unwrap();
    let at = common::time::now_ms();
    service.plan_store.change_order(&plan.plan_id,at,|r,i| {
        let mut value=i.request_body();value["id"]="stock-native-e2e".into();value["status"]="Filled".into();
        value["executedQuantity"]="0.02".into();value["executedQuoteQuantity"]="12".into();
        order_protocol::apply_order(r,i,&value,false,at)?;
        order_protocol::apply_fills(r,i,&[json!({"orderId":"stock-native-e2e","clientId":value["clientId"],"symbol":value["symbol"],"side":"Ask",
            "tradeId":"stock-native-fill","quantity":"0.02","price":"600","fee":"0.012","feeSymbol":"USDC"})],at)?;
        Ok(true)
    }).unwrap();
    (service, plan)
}

#[derive(Clone)]
struct Mock {
    cost: StockChainCost,
    failed: bool,
    path: std::path::PathBuf,
    posts: Arc<AtomicUsize>,
}
struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(mock: Mock) -> (String, Server) {
    let app = Router::new()
        .route(
            "/execute",
            post(
                |State(m): State<Mock>, Json(body): Json<Value>| async move {
                    let text = std::fs::read_to_string(&m.path).unwrap();
                    let journal: Value =
                        serde_json::from_str(text.lines().last().unwrap()).unwrap();
                    assert!(journal["plan"]["nativeTopups"]
                        .as_array()
                        .unwrap()
                        .last()
                        .unwrap()["submission"]["walletSignature"]
                        .is_string());
                    assert_eq!(body["signedTransaction"], signed(&m.cost).unwrap());
                    m.posts.fetch_add(1, Ordering::SeqCst);
                    "reply lost after submission"
                },
            ),
        )
        .route(
            "/rpc",
            post(
                |State(m): State<Mock>, Json(body): Json<Value>| async move {
                    let (id, value) = if m.cost.quote.output_mint == STOCK_WRAPPED_SOL {
                        finalized_native(&m.cost, m.failed)
                    } else {
                        finalized(&m.cost, m.failed)
                    };
                    let result = match body["method"].as_str().unwrap() {
                        "getGenesisHash" => json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
                        "getTransaction" => {
                            assert_eq!(body["params"][0], id);
                            value
                        }
                        other => panic!("unexpected {other}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                },
            ),
        )
        .with_state(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        })),
    )
}

#[tokio::test]
async fn stock_native_topup_failed_attempt_restart_requote_receipt_settlement_releases_wallet() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let (service, plan) = setup(path.clone());
    let mut service = Arc::new(service);
    let hub = realtime::WsHub::new(16);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let posts = Arc::new(AtomicUsize::new(0));
    let (root, _server) = server(Mock {
        cost: plan.terms.chain_cost.clone(),
        failed: false,
        path: path.clone(),
        posts: posts.clone(),
    })
    .await;
    let row = service.plan_store.get(&plan.plan_id).unwrap();
    service
        .finish_chain_recheck(
            &plan.plan_id,
            chain::lookup_with(
                &client,
                &format!("{root}/rpc"),
                &row.terms.chain_cost,
                row.chain_submission.as_ref().unwrap(),
            )
            .await,
        )
        .unwrap();
    for index in 0..2 {
        let old = service.plan_store.get(&plan.plan_id).unwrap();
        assert!(!old.accounting().can_settle());
        assert!(service
            .plan_store
            .settle(&old.plan_id, old.revision, common::time::now_ms())
            .is_err());
        let (native, _) = settlement::native_target(&old).unwrap();
        assert_eq!(native, if index == 0 { 7000 } else { 14000 });
        let now = common::time::now_ms();
        let topup = StockNativeTopup {
            source_revision: old.revision,
            prepared_at_ms: now,
            valuation: valuation(&old.terms.chain_cost, native, 10_000, now, index as u8 + 2),
            wallet: StockWalletEvidence {
                owner: old.request.wallet_address.clone(),
                mint: old.terms.chain_cost.mint.address.clone(),
                stock_raw: Some("19000".into()),
                usdc_raw: Some("15000000".into()),
                sol_lamports: Some("9990000".into()),
                checked_at_ms: now,
                problems: vec![],
            },
            submission: None,
        };
        let mut costly = topup.clone();
        costly.valuation.quote.input_raw = "100000".into();
        assert!(service
            .plan_store
            .prepare_topup(&old.plan_id, costly)
            .unwrap_err()
            .contains("超过原计划"));
        let prepared = service
            .plan_store
            .prepare_topup(&old.plan_id, topup)
            .unwrap();
        let cost = settlement::native_cost(&prepared, &prepared.native_topups[index]).unwrap();
        assert!(cost.valid_until_ms > old.terms.market_valid_until_ms);
        let (root, _server) = server(Mock {
            cost: cost.clone(),
            failed: index == 0,
            path: path.clone(),
            posts: posts.clone(),
        })
        .await;
        let send_client = client.clone();
        let url = format!("{root}/execute");
        service
            .execute_owned(StockPlanExecutionRequest {plan_id:plan.plan_id.clone(),revision:prepared.revision,action:StockExecutionAction::NativeTopup{index},confirm_live:true},hub.clone(), move |s,r,h| async move {
                s.send_topup_with(&r.plan_id,index,&h,signed,move |c,signature| {
                    Box::pin(async move { chain::submit_with(&send_client,&url,None,c,signature).await })
                }).await
            })
            .await
            .unwrap();
        let pending = service.plan_store.get(&plan.plan_id).unwrap();
        assert!(service
            .wallet_claims
            .check("solana", &plan.request.wallet_address, now + 999999)
            .is_err());
        drop(service);
        service = Arc::new(BackpackStocks::stock_plan_fixture(path.clone(), now).0);
        assert!(
            service.plan_store.problem().is_none(),
            "{:?}",
            service.plan_store.problem()
        );
        assert_eq!(
            service
                .send_topup_with(
                    &plan.plan_id,
                    index,
                    &hub,
                    |_| panic!("no resign"),
                    |_, _| panic!("no resend")
                )
                .await
                .unwrap(),
            pending
        );
        let submission = pending.native_topups[index].submission.as_ref().unwrap();
        service
            .finish_topup_recheck(
                &plan.plan_id,
                index,
                chain::lookup_with(&client, &format!("{root}/rpc"), &cost, submission).await,
            )
            .unwrap();
    }
    let complete = service.plan_store.get(&plan.plan_id).unwrap();
    assert!(
        complete.accounting().can_settle(),
        "{:?}",
        complete.accounting()
    );
    assert_eq!(complete.accounting().net_sol_change.as_deref(), Some("0"));
    assert_eq!(
        complete.accounting().net_usdc_change.as_deref(),
        Some("1.978")
    );
    assert_eq!(posts.load(Ordering::SeqCst), 2);
    let preserved = path.with_extension("preserved");
    std::fs::rename(&path, &preserved).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(service
        .plan_store
        .settle(&plan.plan_id, complete.revision, common::time::now_ms())
        .unwrap_err()
        .contains("写入结果未核清"));
    assert!(service
        .plan_store
        .get(&plan.plan_id)
        .unwrap()
        .settlement
        .is_none());
    assert!(service
        .wallet_claims
        .check(
            "solana",
            &plan.request.wallet_address,
            common::time::now_ms()
        )
        .is_err());
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&preserved, &path).unwrap();
    drop(service);
    service = Arc::new(BackpackStocks::stock_plan_fixture(path.clone(), common::time::now_ms()).0);
    assert!(service.plan_store.problem().is_none());
    assert_eq!(service.plan_store.get(&plan.plan_id).unwrap(), complete);
    let journal = std::fs::read(&path).unwrap();
    for lock in [&service.order_lock, &service.rfq_lock, &service.chain_lock] {
        let _checking = lock.lock().await;
        assert!(service.settle_plan(
            StockPlanRevisionRequest { plan_id: plan.plan_id.clone(), revision: complete.revision },
            &hub,
        ).unwrap_err().contains("暂不能释放预留"));
        assert_eq!(std::fs::read(&path).unwrap(), journal);
        assert!(service.wallet_claims.check("solana", &plan.request.wallet_address, common::time::now_ms()).is_err());
    }
    service
        .settle_plan(
            StockPlanRevisionRequest {
                plan_id: plan.plan_id.clone(),
                revision: complete.revision,
            },
            &hub,
        )
        .unwrap();
    let settled = service.plan_store.get(&plan.plan_id).unwrap();
    assert_eq!(settled.phase, StockPlanPhase::Settled);
    assert!(!settled.holds_funds(common::time::now_ms()));
    assert!(service
        .wallet_claims
        .check(
            "solana",
            &plan.request.wallet_address,
            common::time::now_ms()
        )
        .is_ok());
    assert!(service.account.read().evidence.is_none());
    assert!(service.snapshot().preflight.is_none());
    let bytes = std::fs::read(&path).unwrap();
    service
        .plan_store
        .settle(&plan.plan_id, complete.revision, common::time::now_ms())
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let fresh_report = plans::tests::fixture(common::time::now_ms()).0.preflight;
    service.snapshot.write().preflight = fresh_report.clone();
    service
        .settle_plan(
            StockPlanRevisionRequest {
                plan_id: plan.plan_id.clone(),
                revision: complete.revision,
            },
            &hub,
        )
        .unwrap();
    assert_eq!(
        service.snapshot().preflight,
        fresh_report,
        "a duplicate finish must not invalidate the next plan's preflight"
    );
    drop(service);
    let (restored, _) = BackpackStocks::stock_plan_fixture(path, common::time::now_ms());
    assert!(
        restored.plan_store.problem().is_none(),
        "{:?}",
        restored.plan_store.problem()
    );
    assert_eq!(restored.plan_store.get(&plan.plan_id).unwrap(), settled);
    assert!(restored
        .wallet_claims
        .check(
            "solana",
            &plan.request.wallet_address,
            common::time::now_ms()
        )
        .is_ok());
}
