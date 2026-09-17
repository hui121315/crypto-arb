use super::*;
use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        Query, State,
    },
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize},
};

pub(super) fn keys() -> Result<credentials::Credentials, String> {
    credentials::Credentials::parse(
        &STANDARD.encode(common::signing::ed25519_public_key(&[7; 32]).unwrap()),
        &STANDARD.encode([7; 32]),
    )
}
fn request(side: StockRfqSide) -> StockRfqRequest {
    StockRfqRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        asset: "MU.US".into(),
        side,
        quantity: "1.00".into(),
    }
}
fn configured(root: &str, path: std::path::PathBuf) -> Arc<BackpackStocks> {
    let mut s = BackpackStocks::new().unwrap().with_rfq_store(path);
    s.credential_loader = keys;
    s.root = root.into();
    s.ws_url = root.replace("http://", "ws://") + "/ws";
    let now = common::time::now_ms();
    let security = calendar::tests::security();
    *s.catalog.write() = Some(StockCatalog {
        rows: vec![security.clone()],
        observed_at_ms: now,
    });
    *s.calendar.write() =
        Some(calendar::Calendar::parse(&calendar::tests::sessions(), b"[]", now).unwrap());
    *s.snapshot.write() = StockMarketSnapshot {
        security: Some(security.clone()),
        token_metadata_at_ms: Some(now),
        trading_route: Some(StockTradingRoute {
            kind: StockRouteKind::Rfq,
            session: Some(security.sessions[0].clone()),
            symbol: Some(security.rfq_symbol),
            reason: "local fixture session".into(),
            timezone: Some("America/New_York".into()),
            calendar_at_ms: Some(now),
            valid_until_ms: now + 60_000,
        }),
        ..Default::default()
    };
    Arc::new(s)
}
#[derive(Clone)]
struct Mock {
    rows: Arc<Mutex<BTreeMap<u32, Value>>>,
    frames: tokio::sync::broadcast::Sender<String>,
    posts: Arc<AtomicUsize>,
    cancels: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    subscriptions: Arc<AtomicUsize>,
    account_reads: Arc<AtomicUsize>,
    balance_subscriptions: Arc<AtomicUsize>,
    order_subscriptions: Arc<AtomicUsize>,
    fill_reads: Arc<AtomicUsize>,
    complete_fills: Arc<AtomicBool>,
    allow_ws: Arc<AtomicBool>,
    emit: Arc<AtomicBool>,
    bad_ack: Arc<AtomicBool>,
    reject: Arc<AtomicBool>,
}
impl Mock {
    fn new() -> Self {
        Self {
            rows: Default::default(),
            frames: tokio::sync::broadcast::channel(64).0,
            posts: Default::default(),
            cancels: Default::default(),
            connections: Default::default(),
            subscriptions: Default::default(),
            account_reads: Default::default(),
            balance_subscriptions: Default::default(),
            order_subscriptions: Default::default(),
            fill_reads: Default::default(),
            complete_fills: Default::default(),
            allow_ws: Arc::new(AtomicBool::new(true)),
            emit: Arc::new(AtomicBool::new(true)),
            bad_ack: Default::default(),
            reject: Default::default(),
        }
    }
    fn events(&self, row: &Value) {
        if !self.emit.load(Ordering::SeqCst) {
            return;
        }
        for name in ["rfqAccepted", "rfqCandidate"] {
            let _ = self.frames.send(event(row, name));
        }
    }
}
fn native(body: &Value) -> Value {
    let now = common::time::now_ms();
    json!({"rfqId":(100+body["clientId"].as_u64().unwrap()).to_string(),"clientId":body["clientId"],"symbol":body["symbol"],"side":body["side"],"quantity":body["quantity"],
        "executionMode":"AwaitAccept","status":"New","createdAt":now,"submissionTime":now+10,"expiryTime":now+60_000})
}
fn event(row: &Value, name: &str) -> String {
    let time = common::time::now_ms() * 1000
        + match name {
            "rfqCandidate" => 1,
            "rfqAcceptedBinding" => 2,
            "rfqFilled" | "rfqCancelled" => 3,
            _ => 0,
        };
    json!({"stream":"account.rfqUpdate","data":{"e":name,"E":time,"T":time,"R":row["rfqId"],"C":row["clientId"],"s":row["symbol"],"S":row["side"],"q":row["quantity"],"u":"9007199254740997","p":"101.05","X":if name=="rfqFilled"{"Filled"}else if name=="rfqCancelled"{"Cancelled"}else{"New"},"w":row["submissionTime"],"W":row["expiryTime"]}}).to_string()
}
pub(super) fn signed(headers: &HeaderMap, instruction: &str, params: BTreeMap<String, String>) {
    let time = headers["x-timestamp"]
        .to_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!((common::time::now_ms() - time).abs() < 5000);
    assert_eq!(headers["x-window"], "5000");
    assert_eq!(
        headers["x-api-key"].to_str().unwrap(),
        keys().unwrap().public
    );
    let fields = params
        .iter()
        .map(|(k, v)| format!("&{k}={v}"))
        .collect::<String>();
    let canonical = format!("instruction={instruction}{fields}&timestamp={time}&window=5000");
    let expected =
        common::signing::ed25519_sign_bytes_base64(&[7; 32], canonical.as_bytes()).unwrap();
    assert_eq!(headers["x-signature"].to_str().unwrap(), expected);
}
pub(super) fn params(body: &Value) -> BTreeMap<String, String> {
    body.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string()),
            )
        })
        .collect()
}
struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(mock: Mock) -> (String, Server) {
    let router=Router::new()
        .route("/api/v1/account",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"accountQuery",p);m.account_reads.fetch_add(1,Ordering::SeqCst);Json(json!({"spotMakerFee":"8","spotTakerFee":"10","liquidating":false}))
        }))
        .route("/api/v1/capital",get(|h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"balanceQuery",p);Json(json!({"MU.US":{"available":"2","locked":"9","staked":"0"},"USDC":{"available":"100","locked":"20","staked":"0"}}))
        }))
        .route("/api/v1/rfq",post(|State(m):State<Mock>,h:HeaderMap,Json(body):Json<Value>|async move{
            signed(&h,"rfqSubmit",params(&body));assert_eq!(body["executionMode"],"AwaitAccept");
            assert!(body.get("price").is_none() && body.get("quoteQuantity").is_none());
            for field in ["autoBorrow","autoBorrowRepay","autoLend","autoLendRedeem"]{assert_eq!(body[field],false);}
            m.posts.fetch_add(1,Ordering::SeqCst);
            assert!(m.subscriptions.load(Ordering::SeqCst)>0,"RFQ POST preceded private subscription");
            if m.reject.load(Ordering::SeqCst){return (axum::http::StatusCode::UNAUTHORIZED,Json(json!({"code":"INVALID_SIGNATURE","message":"secret_should_be_redacted"})));}
            let row=native(&body);
            m.rows.lock().insert(body["clientId"].as_u64().unwrap() as u32,row.clone());m.events(&row);
            (axum::http::StatusCode::OK,Json(if m.bad_ack.load(Ordering::SeqCst){json!({"fixture":"ack lost after server accepted"})}else{row}))
        }))
        .route("/api/v1/rfqs",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"rfqQuery",p);Json(m.rows.lock().values().filter(|r|r["status"]=="New").map(|r|json!({"rfq":r,"quotes":[{"bidPrice":"999999"}]})).collect::<Vec<_>>())
        }))
        .route("/api/v1/rfq/cancel",post(|State(m):State<Mock>,h:HeaderMap,Json(body):Json<Value>|async move{
            signed(&h,"rfqCancel",params(&body));assert_eq!(body.as_object().unwrap().len(),1);m.cancels.fetch_add(1,Ordering::SeqCst);
            let mut rows=m.rows.lock();let row=rows.values_mut().find(|r|r["rfqId"]==body["rfqId"] || (body.get("clientId").is_some() && r["clientId"]==body["clientId"])).unwrap();row["status"]="Cancelled".into();
            let _=m.frames.send(event(row,"rfqCancelled"));Json(row.clone())
        }))
        .route("/wapi/v1/history/rfq",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"rfqHistoryQueryAll",p.clone());Json(m.rows.lock().values().filter(|r|r["rfqId"].as_str()==p.get("rfqId").map(String::as_str)).map(|r|{let mut r=r.clone();for k in ["createdAt","submissionTime","expiryTime"]{r[k]="2026-09-16T13:30:00".into();}r}).collect::<Vec<_>>())
        }))
        .route("/wapi/v1/history/rfq/fill",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"rfqFillHistoryQueryAll",p.clone());let rows=m.rows.lock();let r=rows.values().find(|r|r["rfqId"].as_str()==p.get("rfqId").map(String::as_str)).unwrap();
            m.fill_reads.fetch_add(1,Ordering::SeqCst);
            let mut fills=vec![fill_row(r,"9007199254740997")];
            if m.complete_fills.load(Ordering::SeqCst){fills.push(fill_row(r,"9007199254740998"));}
            Json(fills)
        }))
        .route("/ws",get(|State(m):State<Mock>,ws:WebSocketUpgrade|async move{
            if !m.allow_ws.load(Ordering::SeqCst){return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();}
            tokio::time::sleep(Duration::from_millis(100)).await;
            ws.on_upgrade(move |mut socket|async move{
            m.connections.fetch_add(1,Ordering::SeqCst);let mut frames=m.frames.subscribe();
            while let Some(Ok(message))=socket.next().await {
                if let Message::Text(t)=message {
                    let v:Value=serde_json::from_str(&t).unwrap();assert_eq!(v["params"],json!(["account.rfqUpdate"]));
                    let time=v["signature"][2].as_str().unwrap().parse().unwrap();assert_eq!(v["signature"][1],keys().unwrap().signature("subscribe",&BTreeMap::new(),time).unwrap());
                    m.subscriptions.fetch_add(1,Ordering::SeqCst);break;
                }
            }
            loop {tokio::select!{
                frame=frames.recv()=>{let Ok(frame)=frame else{break};if socket.send(Message::Text(frame.into())).await.is_err(){break;}},
                msg=socket.next()=>{match msg{Some(Ok(Message::Ping(b)))=>{let _=socket.send(Message::Pong(b)).await;},Some(Ok(Message::Text(t)))=>{
                    let v:Value=serde_json::from_str(&t).unwrap();
                    let time=v["signature"][2].as_str().unwrap().parse().unwrap();assert_eq!(v["signature"][1],keys().unwrap().signature("subscribe",&BTreeMap::new(),time).unwrap());
                    if v["params"]==json!(["account.balanceUpdate"]) {m.balance_subscriptions.fetch_add(1,Ordering::SeqCst);}
                    else {assert_eq!(v["params"],json!(["account.orderUpdate"]));m.order_subscriptions.fetch_add(1,Ordering::SeqCst);}
                },Some(Ok(Message::Close(_)))|None=>break,_=>{}}}
            }}
        })}))
        .with_state(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        })),
    )
}
pub(super) async fn until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(6), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn backpack_stock_preflight_reports_shared_wallet_hold_and_release_without_submitting() {
    use crate::services::onchain_execution_run_store::{test_checkpoint, OnchainExecutionRunStore};

    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let s = configured(&root, dir.path().join("rfq.jsonl"));
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_execution_run_ledger_path =
        Some(dir.path().join("execution.jsonl").to_string_lossy().into());
    let store = OnchainExecutionRunStore::load(&config)
        .store
        .with_wallet_claims(s.wallet_claims.clone());
    let mut checkpoint = test_checkpoint();
    checkpoint.build.chain = "solana".into();
    checkpoint.build.wallet_address = bs58::encode([9u8; 32]).into_string();
    checkpoint.response.updated_at_ms = common::time::now_ms();
    store.append_pending(&checkpoint).unwrap();

    let read = StockPreflightRequest {
        asset: "MU.US".into(),
        wallet_address: Some(checkpoint.build.wallet_address.clone()),
    };
    let hub = realtime::WsHub::new(64);
    // No Mint quote: this exercises the local account server without calling a public RPC.
    let snapshot = s.preflight(read.clone(), hub.clone()).await.unwrap();
    assert!(snapshot
        .preflight
        .as_ref()
        .unwrap()
        .problems
        .iter()
        .any(|p| { p.contains("链上 / CEX 执行") && p.contains(&checkpoint.response.run_id) }));
    let mut finished = checkpoint.response;
    finished.status = shared_types::OnchainExecutionRunStatus::Completed;
    finished.updated_at_ms = common::time::now_ms();
    store.append_run(&finished).unwrap();
    let snapshot = s.preflight(read, hub).await.unwrap();
    assert!(!snapshot
        .preflight
        .as_ref()
        .unwrap()
        .problems
        .iter()
        .any(|p| p.contains("钱包已由")));
    assert_eq!(m.connections.load(Ordering::SeqCst), 1);
    assert_eq!(m.account_reads.load(Ordering::SeqCst), 1);
    assert_eq!(m.posts.load(Ordering::SeqCst), 0);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
    until(|| m.order_subscriptions.load(Ordering::SeqCst) == 1).await;
    assert_eq!(
        s.order_subscription.borrow().as_deref(),
        Some(keys().unwrap().fingerprint().as_str())
    );
    s.account_tracking_until_ms.store(0, Ordering::SeqCst);
    s.order_tracking_until_ms.store(0, Ordering::SeqCst);
}

#[tokio::test]
async fn backpack_stock_preflight_shares_private_ws_reads_fees_once_and_invalidates_on_balance_change(
) {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let s = configured(&root, dir.path().join("rfq.jsonl"));
    let hub = realtime::WsHub::new(64);
    let req = request(StockRfqSide::Ask);
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    until(|| s.snapshot().rfqs.iter().any(|r| r.candidate.is_some())).await;
    let now = common::time::now_ms();
    let mint = StockMintEvidence {
        address: bs58::encode([5u8; 32]).into_string(),
        decimals: 6,
        ui_multiplier: "1".into(),
        slot: 1,
        chain_time_ms: now,
        checked_at_ms: now,
        next_change_at_ms: None,
        extensions: vec![],
    };
    let buy = StockDexQuote {
        input_mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
        output_mint: mint.address.clone(),
        input_raw: "100000000".into(),
        output_raw: "1000000".into(),
        minimum_output_raw: "1000000".into(),
        router: "fixture".into(),
        fee_bps: Some(0),
        fee_mint: Some(shared_types::stocks::comparison::SOLANA_USDC.into()),
        requested_at_ms: now,
        received_at_ms: now,
        expires_at_ms: None,
    };
    s.snapshot.write().comparison = Some(StockComparison {
        asset: "MU.US".into(),
        issuer_docs: "local fixture".into(),
        budget_usdc: "100".into(),
        keyed: false,
        mint,
        buy,
        sell: None,
        sell_problem: Some("fixture unavailable sell".into()),
        quantity_limit: None,
    });
    let read = StockPreflightRequest {
        asset: "MU.US".into(),
        wallet_address: None,
    };
    let snapshot = s.preflight(read.clone(), hub.clone()).await.unwrap();
    let report = snapshot.preflight.as_ref().unwrap();
    assert_eq!(report.spot_taker_fee_pct.as_deref(), Some("0.1"));
    assert_eq!(
        report.directions[0].inventory[0].available.as_deref(),
        Some("2")
    );
    assert_eq!(report.directions[0].cex_fee_usdc.as_deref(), Some("0"));
    assert!(!report.directions[0].executable);
    assert!(report.wallet_at_ms.is_none());
    until(|| m.balance_subscriptions.load(Ordering::SeqCst) == 1).await;
    let t = common::time::now_ms() * 1000 + 1;
    m.frames.send(json!({"stream":"account.balanceUpdate","data":{"e":"balanceUpdate","a":"MU.US","A":"0","L":"11","S":"0","T":t,"E":t}}).to_string()).unwrap();
    until(|| {
        s.account
            .read()
            .evidence
            .as_ref()
            .is_some_and(|e| e.balances["MU.US"].available == "0")
    })
    .await;
    let snapshot = s.snapshot();
    assert!(!snapshot
        .preflight
        .as_ref()
        .unwrap()
        .current(&snapshot, common::time::now_ms()));
    let snapshot = s.preflight(read, hub.clone()).await.unwrap();
    assert_eq!(
        snapshot.preflight.as_ref().unwrap().directions[0].inventory[0].sufficient,
        Some(false)
    );
    assert_eq!(m.connections.load(Ordering::SeqCst), 1);
    assert_eq!(m.account_reads.load(Ordering::SeqCst), 1);
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    assert_eq!(m.order_subscriptions.load(Ordering::SeqCst), 1);
    s.account_tracking_until_ms.store(0, Ordering::SeqCst);
    s.order_tracking_until_ms.store(0, Ordering::SeqCst);
    s.cancel_rfq(&req.request_id, hub).await.unwrap();
}

#[tokio::test]
async fn backpack_stock_rfq_finish_unsent_fences_delayed_requests_without_credentials_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let path = dir.path().join("rfq.jsonl");
    let mut service = Arc::try_unwrap(configured(&root, path.clone())).ok().unwrap();
    service.credential_loader = || panic!("local finish must not load account credentials");
    let s = Arc::new(service);
    let req = request(StockRfqSide::Ask);
    let hub = realtime::WsHub::new(16);
    {
        let _in_flight = s.rfq_lock.lock().await;
        assert!(s.finish_unsent_rfq(req.clone(), hub.clone()).await.is_err());
        assert!(s.rfq_store.get(&req.request_id).is_none());
    }
    let finished = s.finish_unsent_rfq(req.clone(), hub.clone()).await.unwrap();
    assert_eq!(finished.rfqs[0].phase, StockRfqPhase::NotSent);
    assert_eq!(finished.rfqs[0].client_id, 0);
    let bytes = std::fs::read(&path).unwrap();
    s.finish_unsent_rfq(req.clone(), hub.clone()).await.unwrap();
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    s.recheck_rfq(&req.request_id, hub.clone()).await.unwrap();
    s.cancel_rfq(&req.request_id, hub.clone()).await.unwrap();
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    drop(s);
    let s = configured(&root, path);
    assert_eq!(s.request_rfq(req.clone(), hub.clone()).await.unwrap().rfqs[0].phase, StockRfqPhase::NotSent);
    let mut different = req;
    different.quantity = "2".into();
    assert!(s.request_rfq(different.clone(), hub.clone()).await.is_err());
    assert!(s.finish_unsent_rfq(different, hub).await.is_err());
    assert_eq!(m.posts.load(Ordering::SeqCst), 0);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
    assert_eq!(m.connections.load(Ordering::SeqCst), 0);

    let broken_path = dir.path().join("broken.jsonl");
    std::fs::write(&broken_path, "incomplete").unwrap();
    let broken = configured(&root, broken_path.clone());
    assert!(broken.finish_unsent_rfq(request(StockRfqSide::Ask), realtime::WsHub::new(16)).await.is_err());
    assert_eq!(std::fs::read_to_string(broken_path).unwrap(), "incomplete");
}

#[tokio::test]
async fn backpack_stock_rfq_finish_unsent_never_cancels_or_replaces_an_actual_submission() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let s = configured(&root, dir.path().join("rfq.jsonl"));
    let req = request(StockRfqSide::Ask);
    let hub = realtime::WsHub::new(16);
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    let original = s.stock_rfq(&req.request_id).unwrap();
    let result = s.finish_unsent_rfq(req.clone(), hub.clone()).await.unwrap();
    let row = result.rfqs.iter().find(|r| r.request.request_id == req.request_id).unwrap();
    assert_eq!(row.client_id, original.client_id);
    assert_eq!(row.rfq_id, original.rfq_id);
    assert_ne!(row.phase, StockRfqPhase::NotSent);
    assert!(!row.cancel_requested);
    s.request_rfq(req, hub).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn backpack_stock_rfq_old_receipt_remains_recoverable_after_recent_history_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let path = dir.path().join("rfq.jsonl");
    let s = configured(&root, path.clone());
    let mut original = request(StockRfqSide::Ask);
    original.quantity = "1".into();
    for index in 0..50 {
        let mut req = if index == 0 { original.clone() } else { request(StockRfqSide::Ask) };
        req.quantity = "1".into();
        let (row, _) = s.rfq_store.claim(req, &keys().unwrap().fingerprint(), "MU.US_USDC_RFQ".into(), common::time::now_ms()).unwrap();
        s.rfq_store.change(&row.request.request_id, true, |r| {
            r.phase = StockRfqPhase::NotSent;
            r.needs_recheck = false;
            r.problem = Some("fixture: never sent".into());
            Ok(true)
        }).unwrap();
    }
    assert_eq!(s.snapshot().rfqs.len(), 48);
    assert!(!s.snapshot().rfqs.iter().any(|r| r.request == original));
    drop(s);
    let s = configured(&root, path);
    let hub = realtime::WsHub::new(16);
    for snapshot in [
        s.request_rfq(original.clone(), hub.clone()).await.unwrap(),
        s.recheck_rfq(&original.request_id, hub.clone()).await.unwrap(),
        s.cancel_rfq(&original.request_id, hub).await.unwrap(),
    ] {
        assert_eq!(snapshot.rfqs.len(), 48);
        let recovered = snapshot.rfqs.iter().find(|r| r.request == original).unwrap();
        assert_eq!(recovered.phase, StockRfqPhase::NotSent);
        assert!(recovered.rfq_id.is_none());
    }
    assert_eq!(m.posts.load(Ordering::SeqCst), 0);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
    assert_eq!(m.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn backpack_stock_rfq_unavailable_private_ws_is_not_sent_and_does_not_use_a_pending_slot() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    m.allow_ws.store(false, Ordering::SeqCst);
    let (root, _server) = server(m.clone()).await;
    let s = configured(&root, dir.path().join("rfq.jsonl"));
    let hub = realtime::WsHub::new(16);
    let req = request(StockRfqSide::Ask);
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    let record = s.rfq_store.get(&req.request_id).unwrap();
    assert_eq!(record.phase, StockRfqPhase::NotSent);
    assert!(!record.needs_recheck && record.rfq_id.is_none());
    assert_eq!(m.posts.load(Ordering::SeqCst), 0);
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    s.recheck_rfq(&req.request_id, hub.clone()).await.unwrap();
    s.cancel_rfq(&req.request_id, hub).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 0);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
    assert!(s.rfq_store.records().iter().all(|r| r.phase.terminal()));
}

#[tokio::test]
async fn backpack_stock_rfq_definite_rejection_releases_pending_slot_without_automatic_retry() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    m.reject.store(true, Ordering::SeqCst);
    let (root, _server) = server(m.clone()).await;
    let s = configured(&root, dir.path().join("rfq.jsonl"));
    let hub = realtime::WsHub::new(16);
    let req = request(StockRfqSide::Ask);
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    let rejected = s.rfq_store.get(&req.request_id).unwrap();
    assert_eq!(rejected.phase, StockRfqPhase::Rejected);
    assert!(!rejected.needs_recheck);
    assert!(rejected
        .problem
        .as_deref()
        .unwrap()
        .contains("INVALID_SIGNATURE"));
    assert!(!serde_json::to_string(&rejected)
        .unwrap()
        .contains("secret_should_be_redacted"));
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    m.reject.store(false, Ordering::SeqCst);
    let next = request(StockRfqSide::Ask);
    s.request_rfq(next.clone(), hub.clone()).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 2);
    assert!(!s.rfq_store.get(&next.request_id).unwrap().phase.terminal());
    s.cancel_rfq(&next.request_id, hub).await.unwrap();
}

#[tokio::test]
async fn backpack_stock_rfq_signed_http_private_ws_cancel_and_actual_settlement_close_the_fixture_loop(
) {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let s = configured(&root, dir.path().join("rfq.jsonl"));
    let hub = realtime::WsHub::new(64);
    let ask = request(StockRfqSide::Ask);
    s.request_rfq(ask.clone(), hub.clone()).await.unwrap();
    until(|| {
        s.snapshot().rfqs.iter().any(|r| {
            r.current_candidate(s.snapshot().rfq_connected, common::time::now_ms())
                .is_some()
        })
    })
    .await;
    let replay = s.request_rfq(ask.clone(), hub.clone()).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    assert_eq!(
        replay.rfqs[0].candidate.as_ref().unwrap().quote_id,
        "9007199254740997"
    );
    let bid = request(StockRfqSide::Bid);
    s.request_rfq(bid.clone(), hub.clone()).await.unwrap();
    until(|| s.snapshot().rfqs.iter().all(|r| r.candidate.is_some())).await;
    assert_eq!(m.connections.load(Ordering::SeqCst), 1);
    s.cancel_rfq(&ask.request_id, hub.clone()).await.unwrap();
    s.cancel_rfq(&ask.request_id, hub.clone()).await.unwrap();
    assert_eq!(m.cancels.load(Ordering::SeqCst), 1);
    let row = m
        .rows
        .lock()
        .values()
        .find(|r| r["side"] == "Bid")
        .unwrap()
        .clone();
    m.frames.send(event(&row, "rfqAcceptedBinding")).unwrap();
    until(|| s.rfq_store.get(&bid.request_id).unwrap().phase == StockRfqPhase::AcceptedBinding)
        .await;
    assert!(s
        .cancel_rfq(&bid.request_id, hub.clone())
        .await
        .unwrap_err()
        .contains("不允许"));
    m.rows
        .lock()
        .get_mut(&(row["clientId"].as_u64().unwrap() as u32))
        .unwrap()["status"] = "Filled".into();
    m.frames.send(event(&row, "rfqFilled")).unwrap();
    until(|| s.rfq_store.get(&bid.request_id).unwrap().phase == StockRfqPhase::Filled).await;
    until(|| {
        s.rfq_store
            .get(&bid.request_id)
            .unwrap()
            .executed_quantity
            .is_some()
    })
    .await;
    let filled = s.rfq_store.get(&bid.request_id).unwrap();
    assert_eq!(filled.executed_quantity.as_deref(), Some("0.5"));
    assert_eq!(filled.executed_quote_quantity.as_deref(), Some("50.525"));
    assert_eq!(filled.settlement.attempts, 1);
    assert!(filled.settlement_pending());
    assert_eq!(m.fill_reads.load(Ordering::SeqCst), 1);
    m.complete_fills.store(true, Ordering::SeqCst);
    s.recheck_rfq(&bid.request_id, hub.clone()).await.unwrap();
    let filled = s.rfq_store.get(&bid.request_id).unwrap();
    assert_eq!(filled.executed_quantity.as_deref(), Some("1"));
    assert_eq!(filled.executed_quote_quantity.as_deref(), Some("101.05"));
    assert_eq!(filled.fills.len(), 2);
    assert!(!filled.settlement_pending());
    s.recheck_rfq(&bid.request_id, hub).await.unwrap();
    assert_eq!(
        m.fill_reads.load(Ordering::SeqCst),
        3,
        "manual check rereads completed fills"
    );
    assert_eq!(s.rfq_store.get(&bid.request_id).unwrap(), filled);
    assert_eq!(m.posts.load(Ordering::SeqCst), 2);
    let journal = std::fs::read_to_string(dir.path().join("rfq.jsonl")).unwrap();
    assert!(
        !journal.contains(&STANDARD.encode([7; 32])) && !journal.contains(&keys().unwrap().public)
    );
}

fn fill_row(row: &Value, quote: &str) -> Value {
    json!({"rfqId":row["rfqId"],"clientId":row["clientId"],"quoteId":quote,"symbol":row["symbol"],"side":row["side"],"quantity":row["quantity"],"fillQuantity":"0.5","fillQuoteQuantity":"50.525","fillPrice":"101.05"})
}

mod settlement;

#[tokio::test]
async fn backpack_stock_rfq_lost_ack_restart_rechecks_original_without_another_post_or_rest_maker_price(
) {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    m.bad_ack.store(true, Ordering::SeqCst);
    m.emit.store(false, Ordering::SeqCst);
    let (root, _server) = server(m.clone()).await;
    let path = dir.path().join("rfq.jsonl");
    let s = configured(&root, path.clone());
    let hub = realtime::WsHub::new(32);
    let req = request(StockRfqSide::Ask);
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    let initial = s.rfq_store.get(&req.request_id).unwrap();
    assert!(initial.needs_recheck);
    assert!(initial.candidate.is_none());
    assert_eq!(initial.phase, StockRfqPhase::SubmissionUnknown);
    let weak = Arc::downgrade(&s);
    drop(s);
    until(|| weak.upgrade().is_none()).await;
    let s = configured(&root, path);
    s.request_rfq(req.clone(), hub.clone()).await.unwrap();
    s.recheck_rfq(&req.request_id, hub.clone()).await.unwrap();
    let restored = s.rfq_store.get(&req.request_id).unwrap();
    assert!(restored.rfq_id.is_some() && restored.needs_recheck && restored.candidate.is_none());
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    s.cancel_rfq(&req.request_id, hub).await.unwrap();
    assert_eq!(
        s.rfq_store.get(&req.request_id).unwrap().phase,
        StockRfqPhase::Cancelled
    );
}

#[test]
fn backpack_stock_rfq_journal_is_exclusive_replay_safe_and_rejects_corrupt_tail() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rfq.jsonl");
    let store = rfq_store::RfqStore::load(Some(path.clone()));
    let req = request(StockRfqSide::Ask);
    let (record, _) = store
        .claim(req.clone(), "fixture", "MU.US_USDC_RFQ".into(), 1000)
        .unwrap();
    assert!(store
        .claim(req.clone(), "other", "MU.US_USDC_RFQ".into(), 1000)
        .is_err());
    let other = rfq_store::RfqStore::load(Some(path.clone()));
    assert!(other
        .claim(
            request(StockRfqSide::Bid),
            "fixture",
            "MU.US_USDC_RFQ".into(),
            1000
        )
        .is_err());
    drop(other);
    let length = std::fs::metadata(&path).unwrap().len();
    store
        .change(&req.request_id, false, |r| {
            r.candidate = Some(StockRfqCandidate {
                quote_id: "1".into(),
                taker_price: "100".into(),
                source_at_us: 1000000,
                received_at_ms: 1000,
            });
            Ok(true)
        })
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), length);
    drop(store);
    let restored = rfq_store::RfqStore::load(Some(path.clone()));
    let r = restored.get(&req.request_id).unwrap();
    assert!(r.candidate.is_none() && r.needs_recheck);
    assert_eq!(r.client_id, record.client_id);
    drop(restored);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{broken")
        .unwrap();
    let bad = rfq_store::RfqStore::load(Some(path.clone()));
    assert!(bad.problem().is_some());
    assert!(bad
        .claim(
            request(StockRfqSide::Bid),
            "fixture",
            "MU.US_USDC_RFQ".into(),
            2000
        )
        .is_err());
    assert!(std::fs::read(&path).unwrap().ends_with(b"{broken"));
}

#[test]
fn backpack_stock_rfq_protocol_rejects_wrong_identity_and_never_regresses_terminal_or_uses_requested_fill(
) {
    let dir = tempfile::tempdir().unwrap();
    let store = rfq_store::RfqStore::load(Some(dir.path().join("rfq.jsonl")));
    let (mut r, _) = store
        .claim(
            request(StockRfqSide::Ask),
            "fixture",
            "MU.US_USDC_RFQ".into(),
            common::time::now_ms(),
        )
        .unwrap();
    let body = rfq_protocol::submit_body(&r);
    let ack = native(&body);
    let now = common::time::now_ms();
    let (_, frame) = rfq_protocol::frame_id(&event(&ack, "rfqCandidate"))
        .unwrap()
        .unwrap();
    assert!(rfq_protocol::apply_event(&mut r, &frame, now).unwrap());
    assert!(r.current_candidate(true, now).is_none());
    rfq_protocol::apply_rest(
        &mut r,
        rfq_protocol::acknowledgement(&serde_json::to_vec(&ack).unwrap()).unwrap(),
        now,
    )
    .unwrap();
    assert!(!r.needs_recheck);
    assert!(r.current_candidate(true, now + 20).is_some());
    assert!(!rfq_protocol::apply_event(&mut r, &frame, now).unwrap());
    let mut refreshed = ack.clone();
    refreshed["submissionTime"] = (ack["submissionTime"].as_i64().unwrap() + 1000).into();
    refreshed["expiryTime"] = (ack["expiryTime"].as_i64().unwrap() + 1000).into();
    let mut refreshed_record = r.clone();
    rfq_protocol::apply_rest(
        &mut refreshed_record,
        rfq_protocol::acknowledgement(&serde_json::to_vec(&refreshed).unwrap()).unwrap(),
        now + 1,
    )
    .unwrap();
    assert!(refreshed_record.candidate.is_none() && refreshed_record.needs_recheck);
    let retained = refreshed_record.clone();
    assert!(!rfq_protocol::apply_rest(
        &mut refreshed_record,
        rfq_protocol::acknowledgement(&serde_json::to_vec(&ack).unwrap()).unwrap(),
        now + 1
    )
    .unwrap());
    assert_eq!(refreshed_record, retained);
    let (_, fill) = rfq_protocol::frame_id(&event(&ack, "rfqFilled"))
        .unwrap()
        .unwrap();
    rfq_protocol::apply_event(&mut r, &fill, now + 1).unwrap();
    assert!(r.executed_quantity.is_none());
    assert!(!rfq_protocol::apply_rest(
        &mut r,
        rfq_protocol::acknowledgement(&serde_json::to_vec(&ack).unwrap()).unwrap(),
        now + 1
    )
    .unwrap());
    assert_eq!(r.phase, StockRfqPhase::Filled);
    let mut wrong = ack.clone();
    wrong["side"] = "Bid".into();
    assert!(rfq_protocol::apply_rest(
        &mut r,
        rfq_protocol::acknowledgement(&serde_json::to_vec(&wrong).unwrap()).unwrap(),
        now + 1
    )
    .is_err());
    assert!(rfq_protocol::frame_id(r#"{"error":null,"result":null}"#)
        .unwrap()
        .is_none());
}
