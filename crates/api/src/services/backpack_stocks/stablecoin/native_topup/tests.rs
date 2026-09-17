use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use chain::tests as fixture;
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;

struct Remote {
    cost: StockChainCost,
    mode: AtomicUsize,
    posts: AtomicUsize,
    journal: std::path::PathBuf,
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn remote(
    cost: StockChainCost,
    journal: std::path::PathBuf,
) -> (Arc<Remote>, String, Server) {
    let state = Arc::new(Remote {
        cost,
        mode: AtomicUsize::new(1),
        posts: AtomicUsize::new(0),
        journal,
    });
    let app=Router::new().route("/execute",post(|State(s):State<Arc<Remote>>,Json(body):Json<Value>|async move {
        assert_eq!(body["signedTransaction"],fixture::signed(&s.cost).unwrap());
        assert_eq!(body["lastValidBlockHeight"],"1000");
        let log=std::fs::read_to_string(&s.journal).unwrap();
        let last:Value=serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert!(last["plan"]["nativeTopups"].as_array().unwrap().last().unwrap()["terms"]["submission"]["walletSignature"].is_string());
        s.posts.fetch_add(1,Ordering::SeqCst);
        (StatusCode::GATEWAY_TIMEOUT,Json(json!({"error":"local response lost"})))
    })).route("/rpc",post(|State(s):State<Arc<Remote>>,Json(body):Json<Value>|async move {
        let mode=s.mode.load(Ordering::SeqCst);
        let native=s.cost.quote.output_mint==STOCK_WRAPPED_SOL;
        let (id,mut value)=if native {fixture::finalized_native(&s.cost,mode==2)}else{fixture::finalized(&s.cost,mode==2)};
        if mode==3 {value["meta"]["postTokenBalances"][0]["uiTokenAmount"]["amount"]="1".into();}
        let account=|n:Value|json!({"owner":"11111111111111111111111111111111","executable":false,"data":["","base64"],"lamports":n});
        let result=match body["method"].as_str().unwrap() {
            "getGenesisHash"=>json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
            "getTransaction"=>{assert_eq!(body["params"][0],id);if mode==0 {Value::Null}else{value}},
            "getFeeForMessage"=>json!({"context":{"slot":13},"value":7000}),
            "getAccountInfo"=>json!({"context":{"slot":13},"value":account(json!(10_000_000))}),
            "getMinimumBalanceForRentExemption"=>json!(890880),
            "simulateTransaction"=>{
                assert_eq!(body["params"][1]["replaceRecentBlockhash"],false);
                let mut v=fixture::finalized_native(&s.cost,false).1["meta"].clone();
                v["accounts"]=json!([account(v["postBalances"][0].clone())]);v["innerInstructions"]=json!([]);
                json!({"context":{"slot":13},"value":v})
            },
            other=>panic!("unexpected external operation {other}"),
        };Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
    })).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        state,
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        })),
    )
}

async fn completed(path: &std::path::Path) -> (Arc<BackpackStocks>, StockStablecoinPlan) {
    let service = Arc::new(
        BackpackStocks::new()
            .unwrap()
            .with_stablecoin_store(path.into()),
    );
    let now = common::time::now_ms();
    let mut p = stablecoin_store::tests::fixture(now);
    let c = p.cost.as_mut().unwrap();
    fixture::attach(c, false);
    c.wallet_required_lamports = Some("897880".into());
    c.wallet_budget_lamports = Some("7000".into());
    c.native_valuation = Some(super::super::super::native_topup::tests::valuation(
        c, 7000, 20_000, now, 90,
    ));
    p.request.wallet_address = c.wallet_address.clone();
    p.wallet.owner = c.wallet_address.clone();
    p.wallet.sol_lamports = Some("10000000".into());
    p.quote = c.quote.clone();
    let p = stablecoin_preview(p.request, p.wallet, p.quote, p.cost, vec![], now).unwrap();
    assert!(p.can_reserve(now), "{:?}", p.blockers);
    let p = service
        .stablecoin_store
        .reserve(stablecoin_store::tests::request(&p), p, now)
        .unwrap();
    let c = p.preview.cost.as_ref().unwrap();
    let request = StockStablecoinSubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        confirm_live: true,
    };
    let (p, _) = service
        .stablecoin_store
        .begin(&request, &fixture::signed(c).unwrap(), now)
        .unwrap();
    let (_, url, _server) = remote(p.preview.cost.clone().unwrap(), path.into()).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let receipt = chain::lookup_with(
        &client,
        &format!("{url}/rpc"),
        p.preview.cost.as_ref().unwrap(),
        p.submission.as_ref().unwrap(),
    )
    .await
    .unwrap()
    .receipt
    .unwrap();
    let p = service
        .stablecoin_store
        .change_submission(&p.plan_id, common::time::now_ms(), |s| {
            s.transaction_id = Some(receipt.transaction_id.clone());
            s.receipt = Some(receipt);
            Ok(())
        })
        .unwrap();
    assert_eq!(p.phase, StockStablecoinPlanPhase::Completed);
    assert_eq!(store::target(&p).unwrap().0, 7000);
    (service, p)
}

fn row(p: &StockStablecoinPlan, salt: u8) -> StockNativeTopup {
    let now = common::time::now_ms();
    let valuation = super::super::super::native_topup::tests::valuation(
        p.preview.cost.as_ref().unwrap(),
        store::target(p).unwrap().0,
        10_000,
        now,
        salt,
    );
    StockNativeTopup {
        source_revision: p.revision,
        prepared_at_ms: now,
        valuation,
        wallet: StockWalletEvidence {
            owner: p.request.conversion.wallet_address.clone(),
            mint: STOCK_SOLANA_USDT.into(),
            stock_raw: Some("0".into()),
            usdc_raw: Some("9950000".into()),
            sol_lamports: Some("10000000".into()),
            checked_at_ms: now,
            problems: vec![],
        },
        submission: None,
    }
}

#[tokio::test]
async fn stock_stablecoin_native_topup_failure_restart_retry_preserves_fees_and_sends_once() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stablecoin.jsonl");
    let (mut service, mut p) = completed(&path).await;
    let hub = realtime::WsHub::new(16);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut captures = serde_json::Map::new();
    for (index, failed) in [true, false].into_iter().enumerate() {
        let terms = row(&p, index as u8 + 1);
        assert_eq!(
            terms.valuation.native_lamports,
            if failed { "7000" } else { "14000" }
        );
        p = service
            .stablecoin_store
            .prepare_topup(&p.plan_id, terms)
            .unwrap();
        assert!(p.holds_funds(common::time::now_ms()));
        captures.insert(
            "ready".into(),
            serde_json::to_value(service.snapshot()).unwrap(),
        );
        let c = store::cost(&p, index).unwrap();
        let (remote, url, _server) = remote(c, path.clone()).await;
        remote
            .mode
            .store(if failed { 2 } else { 1 }, Ordering::SeqCst);
        let request = StockStablecoinSubmitRequest {
            plan_id: p.plan_id.clone(),
            revision: p.revision,
            confirm_live: true,
        };
        let http = client.clone();
        let rpc = format!("{url}/rpc");
        let send_client = client.clone();
        let execute = format!("{url}/execute");
        service
            .submit_stablecoin_operation(
                request.clone(),
                Some(index),
                hub.clone(),
                || trading::RiskConfig {
                    live_trading_enabled: true,
                    ..Default::default()
                },
                move |c| {
                    Box::pin(async move { chain::recheck_original_with(&http, &rpc, c).await })
                },
                fixture::signed,
                move |c, s| {
                    Box::pin(async move {
                        chain::submit_with(&send_client, &execute, Some("local-fixture"), c, s)
                            .await
                    })
                },
            )
            .await
            .unwrap();
        captures.insert(
            "pending".into(),
            serde_json::to_value(service.snapshot()).unwrap(),
        );
        assert_eq!(remote.posts.load(Ordering::SeqCst), 1);
        assert!(service
            .stablecoin_store
            .cancel_topup(&p.plan_id, index, common::time::now_ms())
            .is_err());
        drop(service);
        service = Arc::new(
            BackpackStocks::new()
                .unwrap()
                .with_stablecoin_store(path.clone()),
        );
        assert!(
            service.stablecoin_store.problem().is_none(),
            "{:?}",
            service.stablecoin_store.problem()
        );
        assert!(service
            .wallet_claims
            .check(
                "solana",
                &p.request.conversion.wallet_address,
                common::time::now_ms() + 100_000
            )
            .is_err());
        service
            .submit_stablecoin_operation(
                request,
                Some(index),
                hub.clone(),
                || panic!("no risk/key access on replay"),
                |_| panic!("no resimulation"),
                |_| panic!("no resign"),
                |_, _| panic!("no resend"),
            )
            .await
            .unwrap();
        let rpc = format!("{url}/rpc");
        let http = client.clone();
        service
            .recheck_stablecoin_operation(&p.plan_id, Some(index), &hub, move |c, s| {
                Box::pin(async move { chain::lookup_with(&http, &rpc, c, s).await })
            })
            .await
            .unwrap();
        p = service.stablecoin_store.get(&p.plan_id).unwrap();
        let report = p.native_accounting().unwrap();
        assert_eq!(report.net_native_lamports, if failed { -14000 } else { 0 });
        assert_eq!(report.spent_usdc_raw, if failed { 0 } else { 10000 });
        captures.insert(
            if failed { "failed" } else { "completed" }.into(),
            serde_json::to_value(service.snapshot()).unwrap(),
        );
        assert!(service
            .wallet_claims
            .check(
                "solana",
                &p.request.conversion.wallet_address,
                common::time::now_ms()
            )
            .is_ok());
        let before = std::fs::read(&path).unwrap();
        service
            .recheck_stablecoin_operation(&p.plan_id, Some(index), &hub, |_, _| {
                panic!("terminal must not refetch")
            })
            .await
            .unwrap();
        assert_eq!(before, std::fs::read(&path).unwrap());
    }
    assert_eq!(p.native_accounting().unwrap().retained_usdc_raw, 9_940_000);
    drop(service);
    let service = BackpackStocks::new().unwrap().with_stablecoin_store(path);
    assert_eq!(service.stablecoin_store.get(&p.plan_id).unwrap(), p);
    if let Ok(path) = std::env::var("STOCK_STABLECOIN_TOPUP_CAPTURE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&captures).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn stock_stablecoin_native_topup_budget_cancel_expiry_shared_claims_and_bad_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stablecoin.jsonl");
    let (service, mut p) = completed(&path).await;
    use crate::services::onchain_wallet_claims::{Owner,Module,Hold};
    let owner=Owner::new(Module::CrossChain,"other-local-funding");
    let now=common::time::now_ms();
    service.wallet_claims.commit(owner.clone(),Some(Hold::wallet("solana",&p.request.conversion.wallet_address,None).unwrap()),now,||Ok(())).unwrap();
    let bytes=std::fs::read(&path).unwrap();
    assert!(service.stablecoin_store.prepare_topup(&p.plan_id,row(&p,1)).unwrap_err().contains("占用"));
    assert_eq!(bytes,std::fs::read(&path).unwrap());
    service.wallet_claims.commit(owner,None,now,||Ok(())).unwrap();
    for case in ["budget", "usdc", "sol", "slot", "wallet", "version"] {
        let mut r = row(&p, 1);
        match case {
            "budget" => {
                r.valuation.quote.input_raw = "20001".into();
            }
            "usdc" => r.wallet.usdc_raw = Some("9509999".into()),
            "sol" => r.wallet.sol_lamports = Some("1".into()),
            "slot" => r.valuation.replenishment.as_mut().unwrap().simulation_slot = 12,
            "wallet" => r.wallet.owner = bs58::encode([8u8; 32]).into_string(),
            _ => r.source_revision = 0,
        }
        assert!(
            service
                .stablecoin_store
                .prepare_topup(&p.plan_id, r)
                .is_err(),
            "{case}"
        );
    }
    let r = row(&p, 1);
    p = service
        .stablecoin_store
        .prepare_topup(&p.plan_id, r)
        .unwrap();
    assert!(service
        .stablecoin_store
        .prepare_topup(&p.plan_id, row(&p, 2))
        .is_err());
    p = service
        .stablecoin_store
        .cancel_topup(&p.plan_id, 0, common::time::now_ms())
        .unwrap();
    assert!(!p.holds_funds(common::time::now_ms()));
    let cancel = std::fs::read(&path).unwrap();
    service
        .stablecoin_store
        .cancel_topup(&p.plan_id, 0, common::time::now_ms())
        .unwrap();
    assert_eq!(cancel, std::fs::read(&path).unwrap());
    p = service
        .stablecoin_store
        .prepare_topup(&p.plan_id, row(&p, 2))
        .unwrap();
    let deadline = p.native_topups[1]
        .terms
        .valuation
        .replenishment
        .as_ref()
        .unwrap()
        .valid_until_ms;
    assert!(!p.holds_funds(deadline));
    let c = store::cost(&p, 1).unwrap();
    let signed = fixture::signed(&c).unwrap();
    let request = StockStablecoinSubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        confirm_live: true,
    };
    assert!(service
        .stablecoin_store
        .begin_topup(&request, 1, &signed, deadline)
        .is_err());
    service
        .stablecoin_store
        .begin_topup(&request, 1, &signed, common::time::now_ms())
        .unwrap();
    let (remote, url, _server) = remote(c.clone(), path.clone()).await;
    remote.mode.store(3, Ordering::SeqCst);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let rpc = format!("{url}/rpc");
    service
        .recheck_stablecoin_operation(
            &p.plan_id,
            Some(1),
            &realtime::WsHub::new(8),
            move |c, s| Box::pin(async move { chain::lookup_with(&client, &rpc, c, s).await }),
        )
        .await
        .unwrap();
    let p = service.stablecoin_store.get(&p.plan_id).unwrap();
    assert!(p.native_accounting().is_err());
    assert!(p.holds_funds(deadline));
    assert!(service
        .stablecoin_store
        .prepare_topup(&p.plan_id, row_without_target(&p))
        .is_err());
    drop(service);
    let service = BackpackStocks::new().unwrap().with_stablecoin_store(path);
    assert_eq!(service.stablecoin_store.get(&p.plan_id).unwrap(), p);
    assert!(service
        .wallet_claims
        .check("solana", &p.request.conversion.wallet_address, deadline)
        .is_err());
}

fn row_without_target(p: &StockStablecoinPlan) -> StockNativeTopup {
    let mut r = p.native_topups.last().unwrap().terms.clone();
    r.source_revision = p.revision;
    r.submission = None;
    r
}
