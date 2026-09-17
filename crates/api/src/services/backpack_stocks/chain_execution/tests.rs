use super::*;
use axum::{extract::State, routing::post, Json, Router};
use chain::tests::{attach, finalized, signed};
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;

#[derive(Clone)]
struct Mock {
    path: std::path::PathBuf,
    posts: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    receipt: Arc<Mutex<Option<(String, Value)>>>,
    cost: StockChainCost,
}

struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn server(mock: Mock) -> (String, Server) {
    let app = Router::new().route("/execute", post(|State(m): State<Mock>, Json(body): Json<Value>| async move {
        let durable = std::fs::read_to_string(&m.path).unwrap();
        let last: Value = serde_json::from_str(durable.lines().last().unwrap()).unwrap();
        assert_eq!(last["plan"]["phase"], "submission_unknown");
        assert!(last["plan"]["chainSubmission"]["walletSignature"].as_str().is_some());
        assert_eq!(body["requestId"], "original-stock-request");
        assert_eq!(body["signedTransaction"], signed(&m.cost).unwrap());
        assert_eq!(body["lastValidBlockHeight"], "1000");
        m.posts.fetch_add(1, Ordering::SeqCst);
        *m.receipt.lock() = Some(finalized(&m.cost, false));
        // The remote side processed it but the client gets an unusable response.
        "truncated-provider-response"
    })).route("/rpc", post(|State(m): State<Mock>, Json(body): Json<Value>| async move {
        m.reads.fetch_add(1, Ordering::SeqCst);
        let result = match body["method"].as_str().unwrap() {
            "getGenesisHash" => json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
            "getSignaturesForAddress" => {
                assert_eq!(body["params"][0], m.cost.wallet_address);
                assert_eq!(body["params"][1]["commitment"], "finalized");
                assert_eq!(body["params"][1]["limit"], 8);
                json!([{ "signature":bs58::encode([88;64]).into_string(),"slot":14 },{"signature":m.receipt.lock().as_ref().unwrap().0,"slot":13}])
            },
            "getTransaction" => {
                assert_eq!(body["params"][1],json!({"encoding":"base64","commitment":"finalized","maxSupportedTransactionVersion":0}));
                let row=m.receipt.lock().clone().unwrap();
                if body["params"][0]==row.0 {row.1} else {Value::Null}
            },
            _ => panic!("unexpected outbound operation: {body}"),
        };
        Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
    })).with_state(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (
        url,
        Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        })),
    )
}

#[tokio::test]
async fn stock_chain_lost_ack_restart_recovers_original_transaction_without_resubmit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plans.jsonl");
    let now = common::time::now_ms();
    let (service, request) = BackpackStocks::stock_plan_fixture(path.clone(), now);
    {
        let mut s = service.snapshot.write();
        attach(&mut s.chain_costs[0], true);
        s.comparison.as_mut().unwrap().buy = s.chain_costs[0].quote.clone();
        plans::tests::refresh_report(
            &mut s,
            service.account.read().evidence.as_ref().unwrap(),
            now,
        );
    }
    let hub = realtime::WsHub::new(16);
    service.reserve_plan(request, &hub).unwrap();
    let plan = service.plan_store.records().remove(0);
    let mock = Mock {
        path: path.clone(),
        posts: Default::default(),
        reads: Default::default(),
        receipt: Default::default(),
        cost: plan.terms.chain_cost.clone(),
    };
    let (url, _server) = server(mock.clone()).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let send_client = client.clone();
    let endpoint = format!("{url}/execute");
    let sent = service
        .send_chain_with(&plan.plan_id, &hub, signed, move |cost, signed| {
            Box::pin(async move {
                chain::submit_with(&send_client, &endpoint, None, cost, signed).await
            })
        })
        .await
        .unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    let row = sent.chain_submission.as_ref().unwrap();
    assert!(row.transaction_id.is_none() && row.receipt.is_none() && row.problem.is_some());
    assert!(!row.provider_acknowledged);
    assert!(service.cancel_plan(&plan.plan_id, &hub).is_err());
    drop(service);
    let (restored, _) = BackpackStocks::stock_plan_fixture(path, now);
    assert!(restored.plan_store.problem().is_none());
    let duplicate = restored
        .send_chain_with(
            &plan.plan_id,
            &hub,
            |_| panic!("must not sign again"),
            |_, _| panic!("must not resend"),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.chain_submission, sent.chain_submission);
    let lookup = restored
        .start_chain_recheck(&plan.plan_id, now + 100)
        .unwrap();
    assert!(restored
        .start_chain_recheck(&plan.plan_id, now + 101)
        .is_err());
    let result = chain::lookup_with(
        &client,
        &format!("{url}/rpc"),
        &lookup.terms.chain_cost,
        lookup.chain_submission.as_ref().unwrap(),
    )
    .await;
    let complete = restored
        .finish_chain_recheck(&plan.plan_id, result)
        .unwrap();
    let receipt = complete
        .chain_submission
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap_or_else(|| panic!("missing receipt: {:?}", complete.chain_submission));
    assert!(receipt.succeeded && receipt.within_plan);
    assert_eq!(receipt.wallet_native_change_lamports, "0");
    assert_eq!(receipt.network_fee_lamports, "7000");
    assert!(complete.holds_funds(now + 100000));
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert_eq!(mock.reads.load(Ordering::SeqCst), 4);
    assert!(restored
        .plan_store
        .change_chain(&plan.plan_id, now + 200, |r| {
            r.receipt = None;
            Ok(())
        })
        .is_err());
    assert!(restored
        .plan_store
        .change_chain(&plan.plan_id, now + 200, |r| {
            r.wallet_signature = bs58::encode([44; 64]).into_string();
            Ok(())
        })
        .is_err());
    let final_path = mock.path.clone();
    drop(restored);
    let (again, _) = BackpackStocks::stock_plan_fixture(final_path, now);
    assert!(again.plan_store.problem().is_none());
    assert_eq!(again.plan_store.get(&plan.plan_id).unwrap(), complete);
}
