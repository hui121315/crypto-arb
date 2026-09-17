use super::*;
use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::StreamExt;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize},
};

mod reconciliation;

fn keys() -> Result<credentials::Credentials, String> {
    credentials::Credentials::parse(
        &STANDARD.encode(common::signing::ed25519_public_key(&[7; 32]).unwrap()),
        &STANDARD.encode([7; 32]),
    )
}

#[derive(Clone)]
struct Mock {
    frames: tokio::sync::broadcast::Sender<String>,
    order: Arc<Mutex<Option<Value>>>,
    fills: Arc<Mutex<Vec<Value>>>,
    posts: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    subscriptions: Arc<AtomicUsize>,
    bad_ack: Arc<AtomicBool>,
    reject: Arc<AtomicBool>,
    path: std::path::PathBuf,
}

impl Mock {
    fn new(path: std::path::PathBuf) -> Self {
        Self {
            frames: tokio::sync::broadcast::channel(64).0,
            order: Default::default(),
            fills: Default::default(),
            posts: Default::default(),
            reads: Default::default(),
            connections: Default::default(),
            subscriptions: Default::default(),
            bad_ack: Default::default(),
            reject: Default::default(),
            path,
        }
    }
    fn settle(&self, missing_fee: bool) {
        let mut guard = self.order.lock();
        let row = guard.as_mut().unwrap();
        row["status"] = json!("Filled");
        row["executedQuantity"] = json!("0.02");
        row["executedQuoteQuantity"] = json!("12.02");
        *self.fills.lock() = vec![
            json!({"clientId":row["clientId"].as_u64().unwrap().to_string(),"orderId":row["id"],"symbol":row["symbol"],"side":"Ask","tradeId":9007199254740993u64,"quantity":"0.01","price":"600","fee":"0.006","feeSymbol":"USDC"}),
            json!({"clientId":row["clientId"],"orderId":row["id"],"symbol":row["symbol"],"side":"Ask","tradeId":9007199254740994u64,"quantity":"0.01","price":"602","fee":if missing_fee {Value::Null}else{json!("0.00001")},"feeSymbol":"MU.US"}),
        ];
    }
    fn fill_frame(&self, index: usize) -> String {
        let row = self.order.lock().clone().unwrap();
        let fill = self.fills.lock()[index].clone();
        json!({"stream":"account.orderUpdate","data":{
            "e":"orderFill","E":common::time::now_ms()*1000,"T":common::time::now_ms()*1000,"c":row["clientId"],"s":row["symbol"],"S":row["side"],"o":"LIMIT","f":"FOK","q":row["quantity"],"p":row["price"],"X":if index==0{"PartiallyFilled"}else{"Filled"},"i":row["id"],"t":fill["tradeId"],"l":fill["quantity"],"L":fill["price"],"z":if index==0{"0.01"}else{"0.02"},"Z":if index==0{"6"}else{"12.02"},"n":fill["fee"],"N":fill["feeSymbol"],"O":"USER"
        }}).to_string()
    }
}

fn signed(headers: &HeaderMap, instruction: &str, params: BTreeMap<String, String>) {
    let now = headers["x-timestamp"]
        .to_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!((common::time::now_ms() - now).abs() < 5_000);
    assert_eq!(
        headers["x-api-key"].to_str().unwrap(),
        keys().unwrap().public
    );
    assert_eq!(headers["x-window"], "5000");
    let fields = params
        .iter()
        .map(|(k, v)| format!("&{k}={v}"))
        .collect::<String>();
    let message = format!("instruction={instruction}{fields}&timestamp={now}&window=5000");
    assert_eq!(
        headers["x-signature"].to_str().unwrap(),
        common::signing::ed25519_sign_bytes_base64(&[7; 32], message.as_bytes()).unwrap()
    );
}

struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(mock: Mock) -> (String, Server) {
    let router=Router::new()
        .route("/api/v1/order",get(|State(m):State<Mock>,headers:HeaderMap,Query(params):Query<BTreeMap<String,String>>|async move {
            signed(&headers,"orderQuery",params.clone());m.reads.fetch_add(1,Ordering::SeqCst);
            assert_ne!(params.contains_key("orderId"),params.contains_key("clientId"));
            (StatusCode::NOT_FOUND,Json(json!({"code":"ORDER_NOT_FOUND","message":"not resting"})))
        }).post(|State(m):State<Mock>,headers:HeaderMap,Json(body):Json<Value>|async move {
            let params=body.as_object().unwrap().iter().map(|(k,v)|(k.clone(),v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string()))).collect();
            signed(&headers,"orderExecute",params);
            assert!(m.subscriptions.load(Ordering::SeqCst)>0);
            let bytes=std::fs::read_to_string(&m.path).unwrap();
            let durable:Value=serde_json::from_str(bytes.lines().last().unwrap()).unwrap();
            assert_eq!(durable["plan"]["phase"],"submission_unknown");assert_eq!(durable["plan"]["cexOrder"]["phase"],"submission_unknown");
            for key in ["autoBorrow","autoBorrowRepay","autoLend","autoLendRedeem","postOnly"] {assert_eq!(body[key],false);}
            assert_eq!(body["selfTradePrevention"],"RejectTaker");assert_eq!(body["timeInForce"],"FOK");assert_eq!(body["orderType"],"Limit");
            assert_eq!(body["quantity"],"0.02");assert_eq!(body["price"],"600");assert!(body.get("reduceOnly").is_none());
            m.posts.fetch_add(1,Ordering::SeqCst);
            if m.reject.load(Ordering::SeqCst) {return (StatusCode::BAD_REQUEST,Json(json!({"code":"INSUFFICIENT_FUNDS","message":"sensitive-local-marker"})));}
            let mut ack=body.clone();ack["id"]=json!("900719925474099333");ack["status"]=json!("New");ack["executedQuantity"]=json!("0");ack["executedQuoteQuantity"]=json!("0");
            *m.order.lock()=Some(ack.clone());
            (StatusCode::OK,Json(if m.bad_ack.load(Ordering::SeqCst){json!({})}else{ack}))
        }))
        .route("/wapi/v1/history/orders",get(|State(m):State<Mock>,headers:HeaderMap,Query(params):Query<BTreeMap<String,String>>|async move {
            signed(&headers,"orderHistoryQueryAll",params);m.reads.fetch_add(1,Ordering::SeqCst);Json(m.order.lock().clone().into_iter().collect::<Vec<_>>())
        }))
        .route("/wapi/v1/history/fills",get(|State(m):State<Mock>,headers:HeaderMap,Query(params):Query<BTreeMap<String,String>>|async move {
            assert_eq!(params["orderId"],"900719925474099333");
            signed(&headers,"fillHistoryQueryAll",params);m.reads.fetch_add(1,Ordering::SeqCst);Json(m.fills.lock().clone())
        }))
        .route("/ws",get(|State(m):State<Mock>,ws:WebSocketUpgrade|async move {
            ws.on_upgrade(move |mut socket|async move {
                m.connections.fetch_add(1,Ordering::SeqCst);let mut frames=m.frames.subscribe();
                loop {tokio::select!{
                    msg=socket.next()=>match msg {
                        Some(Ok(Message::Text(text)))=>{
                            let v:Value=serde_json::from_str(&text).unwrap();
                            let stamp=v["signature"][2].as_str().unwrap().parse::<i64>().unwrap();
                            assert_eq!(v["signature"][0],keys().unwrap().public);
                            assert_eq!(v["signature"][1],keys().unwrap().signature("subscribe",&BTreeMap::new(),stamp).unwrap());
                            match v["params"][0].as_str() {Some("account.orderUpdate")=>{m.subscriptions.fetch_add(1,Ordering::SeqCst);},Some("account.rfqUpdate"|"account.balanceUpdate")=>{},_=>panic!("unexpected private subscription")}
                        },
                        Some(Ok(Message::Ping(p)))=>{if socket.send(Message::Pong(p)).await.is_err(){break;}},
                        Some(Ok(_))=>{},_=>break,
                    },
                    frame=frames.recv()=>{if let Ok(text)=frame {if socket.send(Message::Text(text.into())).await.is_err(){break;}}}
                }}
            }).into_response()
        })).with_state(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (root, Server(task))
}

async fn until(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(6), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn stop(s: &BackpackStocks) {
    let worker = s.rfq_worker.lock().take();
    if let Some(worker) = worker {
        worker.abort();
        let _ = worker.await;
    }
}
async fn prepared(
    root: &str,
    path: std::path::PathBuf,
) -> (Arc<BackpackStocks>, StockExecutionPlan) {
    let (mut service, _) = BackpackStocks::stock_plan_fixture(path, common::time::now_ms());
    service.root = root.into();
    service.ws_url = root.replace("http://", "ws://") + "/ws";
    let s = Arc::new(service);
    s.order_tracking_until_ms
        .store(common::time::now_ms() + 30_000, Ordering::SeqCst);
    s.ensure_rfq_started(realtime::WsHub::new(16));
    until(|| s.order_subscription.borrow().is_some()).await;
    let (snapshot, account, request) = plans::tests::fixture(common::time::now_ms());
    *s.snapshot.write() = snapshot;
    s.account.write().evidence = Some(account);
    let snapshot = s.reserve_plan(request, &realtime::WsHub::new(16)).unwrap();
    let plan = snapshot.plans[0].clone();
    (s, plan)
}

#[tokio::test]
async fn stock_order_signed_http_shared_ws_native_fees_replay_and_wallet_hold() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("orders.jsonl");
    let mock = Mock::new(path.clone());
    let (root, _server) = server(mock.clone()).await;
    let (s, plan) = prepared(&root, path.clone()).await;
    let sent = s
        .send_orderbook_leg(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .unwrap();
    assert_eq!(
        sent.cex_order.as_ref().unwrap().phase,
        StockCexOrderPhase::Open
    );
    assert!(!sent.cex_order.as_ref().unwrap().receipt_complete());
    mock.settle(false);
    // Deliver the terminal fill first, followed by the older partial fill.
    mock.frames.send(mock.fill_frame(1)).unwrap();
    mock.frames.send(mock.fill_frame(0)).unwrap();
    until(|| {
        s.plan_store
            .get(&plan.plan_id)
            .unwrap()
            .cex_order
            .unwrap()
            .receipt_complete()
    })
    .await;
    let complete = s.plan_store.get(&plan.plan_id).unwrap();
    let order = complete.cex_order.as_ref().unwrap();
    let changes = order
        .net_asset_changes(plan.terms.cex_instruction.as_ref().unwrap())
        .unwrap();
    assert_eq!(changes["MU.US"], "-0.02001");
    assert_eq!(changes["USDC"], "12.014");
    assert_eq!(order.fills.len(), 2);
    assert_eq!(order.fills[0].trade_id, "9007199254740994");
    let journal = std::fs::read(&path).unwrap();
    mock.frames.send(mock.fill_frame(1)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    s.send_orderbook_leg(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert_eq!(mock.connections.load(Ordering::SeqCst), 1);
    assert_eq!(complete.phase, StockPlanPhase::SubmissionUnknown);
    assert!(s
        .cancel_plan(&plan.plan_id, &realtime::WsHub::new(16))
        .is_err());
    stop(&s).await;
    drop(s);
    let restored = BackpackStocks::new().unwrap().with_plan_store(path);
    assert_eq!(restored.plan_store.get(&plan.plan_id).unwrap(), complete);
    assert!(restored.plan_store.problem().is_none());
    assert!(restored
        .wallet_claims
        .check(
            "solana",
            &plan.request.wallet_address,
            common::time::now_ms() + 100_000
        )
        .is_err());
}

#[tokio::test]
async fn stock_order_lost_ack_recheck_original_and_missing_fee_survive_restart_without_resend() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("orders.jsonl");
    let mock = Mock::new(path.clone());
    mock.bad_ack.store(true, Ordering::SeqCst);
    let (root, _server) = server(mock.clone()).await;
    let (s, plan) = prepared(&root, path.clone()).await;
    let sent = s
        .send_orderbook_leg(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .unwrap();
    assert_eq!(
        sent.cex_order.unwrap().phase,
        StockCexOrderPhase::SubmissionUnknown
    );
    stop(&s).await;
    drop(s);
    let (mut service, _) = BackpackStocks::stock_plan_fixture(path.clone(), common::time::now_ms());
    service.root = root.clone();
    service.ws_url = root.replace("http://", "ws://") + "/ws";
    let s = Arc::new(service);
    mock.settle(true);
    s.resume_rfq(realtime::WsHub::new(16));
    until(|| {
        s.plan_store
            .get(&plan.plan_id)
            .unwrap()
            .cex_order
            .as_ref()
            .is_some_and(|o| o.fills.len() == 2 && o.recheck.attempts == 1)
    })
    .await;
    let guard = s.order_lock.lock().await;
    let pending = s.plan_store.get(&plan.plan_id).unwrap();
    assert_eq!(
        pending.cex_order.as_ref().unwrap().phase,
        StockCexOrderPhase::Filled
    );
    assert!(!pending.cex_order.as_ref().unwrap().receipt_complete());
    // Exercise the persisted sixth-attempt boundary, not a memory-only retry counter.
    s.plan_store
        .change_order(&plan.plan_id, common::time::now_ms(), |r, _| {
            r.recheck.attempts = 5;
            r.recheck.next_at_ms = Some(0);
            Ok(true)
        })
        .unwrap();
    s.reconcile_stock_order(&plan.plan_id, &keys().unwrap(), true)
        .await
        .unwrap();
    assert!(
        s.plan_store
            .get(&plan.plan_id)
            .unwrap()
            .cex_order
            .unwrap()
            .recheck
            .paused
    );
    drop(guard);
    stop(&s).await;
    drop(s);
    let (mut restored, _) = BackpackStocks::stock_plan_fixture(path, common::time::now_ms());
    restored.root = root;
    let restored = Arc::new(restored);
    let reads = mock.reads.load(Ordering::SeqCst);
    restored.resume_rfq(realtime::WsHub::new(16));
    assert!(restored.rfq_worker.lock().is_none());
    assert_eq!(mock.reads.load(Ordering::SeqCst), reads);
    assert!(!restored
        .plan_store
        .get(&plan.plan_id)
        .unwrap()
        .cex_order
        .unwrap()
        .receipt_complete());
    mock.settle(false);
    restored
        .recheck_stock_order(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .unwrap();
    let complete = restored.plan_store.get(&plan.plan_id).unwrap();
    assert!(complete.cex_order.as_ref().unwrap().receipt_complete());
    assert_eq!(complete.cex_order.as_ref().unwrap().recheck.attempts, 6);
    assert_eq!(complete.cex_order.as_ref().unwrap().fills.len(), 2);
    assert!(complete.holds_funds(i64::MAX));
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    stop(&restored).await;
}

#[tokio::test]
async fn stock_order_changed_preflight_and_journal_failure_do_not_send() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("orders.jsonl");
    let mock = Mock::new(path.clone());
    let (root, _server) = server(mock.clone()).await;
    let (s, plan) = prepared(&root, path.clone()).await;
    s.account
        .write()
        .evidence
        .as_mut()
        .unwrap()
        .spot_taker_fee_bps = "20".into();
    assert!(s
        .send_orderbook_leg(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .is_err());
    assert_eq!(mock.posts.load(Ordering::SeqCst), 0);
    s.account
        .write()
        .evidence
        .as_mut()
        .unwrap()
        .spot_taker_fee_bps = "10".into();
    let backup = path.with_extension("saved");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(s
        .send_orderbook_leg(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .is_err());
    assert_eq!(mock.posts.load(Ordering::SeqCst), 0);
    assert!(s.plan_store.problem().is_some());
    stop(&s).await;
    drop(s);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    let restored = BackpackStocks::new().unwrap().with_plan_store(path);
    assert_eq!(restored.plan_store.get(&plan.plan_id).unwrap(), plan);
    assert_eq!(mock.posts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stock_order_definitive_rejection_preserves_plan_and_redacts_remote_message() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("orders.jsonl");
    let mock = Mock::new(path.clone());
    mock.reject.store(true, Ordering::SeqCst);
    let (root, _server) = server(mock.clone()).await;
    let (s, plan) = prepared(&root, path.clone()).await;
    let rejected = s
        .send_orderbook_leg(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .unwrap();
    assert_eq!(
        rejected.cex_order.as_ref().unwrap().phase,
        StockCexOrderPhase::Rejected
    );
    assert!(rejected.cex_order.as_ref().unwrap().receipt_complete());
    s.send_orderbook_leg(&plan.plan_id, realtime::WsHub::new(16))
        .await
        .unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert!(rejected.holds_funds(i64::MAX));
    assert!(!std::fs::read_to_string(path)
        .unwrap()
        .contains("sensitive-local-marker"));
    stop(&s).await;
}

#[test]
fn stock_order_receipts_enforce_identity_terminal_monotonicity_and_native_zero_fee() {
    let plan = plans::tests::fixture_plan(10_000);
    let instruction = plan.terms.cex_instruction.as_ref().unwrap();
    let StockCexInstruction::OrderBook { client_id, .. } = instruction else {
        unreachable!()
    };
    let mut ack = json!({"clientId":client_id,"symbol":"MU.US_USDC","side":"Ask","id":"900719925474099333","quantity":"0.02","price":"600","orderType":"Limit","timeInForce":"FOK","status":"Filled","executedQuantity":"0.02","executedQuoteQuantity":"12"});
    let mut order = StockCexOrder::intent(10_000);
    order_protocol::apply_order(&mut order, instruction, &ack, false, 10_001).unwrap();
    assert!(!order.receipt_complete());
    let mut fill = json!({"clientId":client_id.to_string(),"symbol":"MU.US_USDC","side":"Ask","orderId":"900719925474099333","tradeId":9007199254740995u64,"quantity":"0.02","price":"600"});
    order_protocol::apply_fills(&mut order, instruction, &[fill.clone()], 10_002).unwrap();
    assert!(!order.receipt_complete());
    fill["fee"] = json!("0");
    fill["feeSymbol"] = json!("BPT");
    order_protocol::apply_fills(
        &mut order,
        instruction,
        &[fill.clone(), fill.clone()],
        10_003,
    )
    .unwrap();
    assert!(order.receipt_complete());
    assert_eq!(order.net_asset_changes(instruction).unwrap()["BPT"], "0");
    assert_eq!(order.fills.len(), 1);
    let before = order.clone();
    ack["status"] = json!("New");
    ack["executedQuantity"] = json!("0");
    ack["executedQuoteQuantity"] = json!("0");
    assert!(!order_protocol::apply_order(&mut order, instruction, &ack, false, 10_004).unwrap());
    assert_eq!(order, before);
    fill["fee"] = json!("0.1");
    assert!(order_protocol::apply_fills(&mut order.clone(), instruction, &[fill], 10_005).is_err());
    ack["clientId"] = json!(client_id.wrapping_add(1));
    assert!(
        order_protocol::apply_order(&mut order.clone(), instruction, &ack, false, 10_005).is_err()
    );
    ack["clientId"] = json!(client_id);
    ack["id"] = json!("other-order");
    assert!(
        order_protocol::apply_order(&mut order.clone(), instruction, &ack, false, 10_005).is_err()
    );
    let mut cancelled = before.clone();
    cancelled.phase = StockCexOrderPhase::Cancelled;
    assert!(!order_protocol::transition(Some(&before), Some(&cancelled)));
}

#[test]
fn stock_order_conflicting_ws_fee_stops_net_receipt_without_rewriting_each_duplicate() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("orders.jsonl");
    let service = BackpackStocks::new().unwrap().with_plan_store(path.clone());
    let plan = plans::tests::fixture_plan(10_000);
    let fingerprint = &plan.terms.account_fingerprint;
    service.plan_store.reserve(plan.clone(), 10_000).unwrap();
    service
        .plan_store
        .begin_order(&plan.plan_id, fingerprint, 10_001)
        .unwrap();
    let Some(StockCexInstruction::OrderBook { client_id, .. }) =
        plan.terms.cex_instruction.as_ref()
    else {
        unreachable!()
    };
    let mut frame = json!({"e":"orderFill","c":client_id,"s":"MU.US_USDC","S":"Ask","o":"LIMIT","f":"FOK","q":"0.02","p":"600","X":"Filled","i":"900719925474099333","t":9007199254740993u64,"l":"0.02","L":"600","z":"0.02","Z":"12","n":"0","N":"USDC","O":"USER"});
    assert!(apply_frame(&service, &frame.to_string(), fingerprint, 10_002).unwrap());
    assert!(service
        .plan_store
        .get(&plan.plan_id)
        .unwrap()
        .cex_order
        .unwrap()
        .receipt_complete());
    frame["n"] = json!("0.1");
    assert!(apply_frame(&service, &frame.to_string(), fingerprint, 10_003).is_err());
    let once = std::fs::read(&path).unwrap();
    assert!(apply_frame(&service, &frame.to_string(), fingerprint, 10_004).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), once);
    drop(service);
    let restored = BackpackStocks::new().unwrap().with_plan_store(path);
    assert!(restored.plan_store.problem().is_none());
    let record = restored
        .plan_store
        .get(&plan.plan_id)
        .unwrap()
        .cex_order
        .unwrap();
    assert!(record.evidence_conflict);
    assert!(!record.needs_follow_up());
    assert!(!record.receipt_complete());
    assert_eq!(record.fills[0].fee.as_ref().unwrap().quantity, "0");
    assert!(record
        .net_asset_changes(plan.terms.cex_instruction.as_ref().unwrap())
        .is_none());
}

#[test]
fn stock_order_rest_fee_conflict_cannot_leave_a_confirmed_net_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let service = BackpackStocks::new()
        .unwrap()
        .with_plan_store(temp.path().join("orders.jsonl"));
    let plan = plans::tests::fixture_plan(10_000);
    service.plan_store.reserve(plan.clone(), 10_000).unwrap();
    service
        .plan_store
        .begin_order(&plan.plan_id, &plan.terms.account_fingerprint, 10_001)
        .unwrap();
    let Some(StockCexInstruction::OrderBook { client_id, .. }) =
        plan.terms.cex_instruction.as_ref()
    else {
        unreachable!()
    };
    let ack = json!({"clientId":client_id,"symbol":"MU.US_USDC","side":"Ask","id":"remote-order-1","quantity":"0.02","price":"600","orderType":"Limit","timeInForce":"FOK","status":"Filled","executedQuantity":"0.02","executedQuoteQuantity":"12"});
    service
        .record_order_receipt(&plan.plan_id, 10_002, |r, i| {
            order_protocol::apply_order(r, i, &ack, false, 10_002)
        })
        .unwrap();
    let mut fill = json!({"clientId":client_id,"symbol":"MU.US_USDC","side":"Ask","orderId":"remote-order-1","tradeId":"fill-1","quantity":"0.02","price":"600","fee":"-0.001","feeSymbol":"BPT"});
    let complete = service
        .record_order_receipt(&plan.plan_id, 10_003, |r, i| {
            order_protocol::apply_fills(r, i, &[fill.clone()], 10_003)
        })
        .unwrap();
    assert_eq!(
        complete
            .cex_order
            .unwrap()
            .net_asset_changes(plan.terms.cex_instruction.as_ref().unwrap())
            .unwrap()["BPT"],
        "0.001"
    );
    fill["fee"] = json!("0.1");
    assert!(service
        .record_order_receipt(&plan.plan_id, 10_004, |r, i| order_protocol::apply_fills(
            r,
            i,
            &[fill],
            10_004
        ))
        .is_err());
    let conflict = service
        .plan_store
        .get(&plan.plan_id)
        .unwrap()
        .cex_order
        .unwrap();
    assert!(conflict.evidence_conflict);
    assert!(!conflict.receipt_complete());
    assert!(!conflict.needs_follow_up());
    assert_eq!(conflict.fills[0].fee.as_ref().unwrap().quantity, "-0.001");
}
