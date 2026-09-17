use super::super::rfq_tests::{keys, params, signed, until};
use super::*;
use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize},
};

#[derive(Clone, Default)]
struct Mock {
    native: Arc<Mutex<Value>>,
    frames: Option<tokio::sync::broadcast::Sender<String>>,
    posts: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    fill_reads: Arc<AtomicUsize>,
    history_reads: Arc<AtomicUsize>,
    fill_override: Arc<Mutex<Option<Value>>>,
    cancels: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    subscriptions: Arc<AtomicUsize>,
    bad_ack: Arc<AtomicBool>,
    reject: Arc<AtomicBool>,
    empty_fills: Arc<AtomicBool>,
    emit: Arc<AtomicBool>,
    plan_path: Arc<Mutex<PathBuf>>,
}
impl Mock {
    fn new() -> Self {
        Self {
            frames: Some(tokio::sync::broadcast::channel(64).0),
            ..Self::default()
        }
    }
    fn frame(&self, name: &str) -> String {
        let r = self.native.lock();
        let now = common::time::now_ms() * 1000;
        json!({"stream":"account.rfqUpdate","data":{"e":name,"R":r["rfqId"],"C":r["clientId"],"s":r["symbol"],"S":r["side"],"q":r["quantity"],
            "u":"9007199254740997","p":"601","E":now,"T":now,"X":if name=="rfqFilled"{"Filled"}else{"New"}}}).to_string()
    }
    fn fill(&self) -> Value {
        let r = self.native.lock();
        json!({"rfqId":r["rfqId"],"quoteId":"9007199254740997","clientId":r["clientId"],"symbol":r["symbol"],"side":r["side"],"quantity":r["quantity"],
            "fillQuantity":"0.02","fillQuoteQuantity":"12.02","fillPrice":"601"})
    }
    fn filled(&self) {
        let mut r = self.native.lock();
        r["status"] = json!("Filled");
        r["executedQuantity"] = json!("0.02");
        r["executedQuoteQuantity"] = json!("12.02");
    }
}
struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(m: Mock) -> (String, Server) {
    let router = Router::new()
        .route("/api/v1/rfq/accept",post(|State(m):State<Mock>,h:HeaderMap,Json(body):Json<Value>|async move{
            signed(&h,"quoteAccept",params(&body));
            assert_eq!(body,json!({"rfqId":"9007199254740993","quoteId":"9007199254740997"}));
            assert!(m.subscriptions.load(Ordering::SeqCst)>0);
            let journal=std::fs::read_to_string(&*m.plan_path.lock()).unwrap();
            let last:Value=serde_json::from_str(journal.lines().last().unwrap()).unwrap();
            assert_eq!(last["plan"]["phase"],"submission_unknown");
            assert_eq!(last["plan"]["rfqAcceptance"]["acceptance"]["quoteId"],body["quoteId"]);
            m.posts.fetch_add(1,Ordering::SeqCst);
            if m.reject.load(Ordering::SeqCst){return (StatusCode::BAD_REQUEST,Json(json!({"code":"INSUFFICIENT_FUNDS","message":"do_not_expose_secret"})));}
            if m.emit.load(Ordering::SeqCst){let _=m.frames.as_ref().unwrap().send(m.frame("rfqAcceptedBinding"));}
            tokio::time::sleep(Duration::from_millis(40)).await;
            (StatusCode::OK,Json(if m.bad_ack.load(Ordering::SeqCst){json!({"lost":"after acceptance"})}else{m.native.lock().clone()}))
        }))
        .route("/api/v1/rfqs",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"rfqQuery",p.clone());assert_eq!(p.get("rfqId").unwrap(),"9007199254740993");
            m.reads.fetch_add(1,Ordering::SeqCst);let r=m.native.lock();
            Json(if r["status"]=="New"{json!([{"rfq":*r}])}else{json!([])})
        }))
        .route("/wapi/v1/history/rfq",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"rfqHistoryQueryAll",p.clone());assert_eq!(p.get("rfqId").unwrap(),"9007199254740993");
            m.history_reads.fetch_add(1,Ordering::SeqCst);
            let mut r=m.native.lock().clone();r["deferredSettlementQuoteId"]=json!("9007199254740997");Json(json!([r]))
        }))
        .route("/wapi/v1/history/rfq/fill",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move{
            signed(&h,"rfqFillHistoryQueryAll",p.clone());assert_eq!(p.get("rfqId").unwrap(),"9007199254740993");
            m.fill_reads.fetch_add(1,Ordering::SeqCst);
            let fill=m.fill_override.lock().clone().unwrap_or_else(||m.fill());
            Json(if m.empty_fills.load(Ordering::SeqCst){json!([])}else{json!([fill.clone(),fill])})
        }))
        .route("/api/v1/rfq/cancel",post(|State(m):State<Mock>|async move{m.cancels.fetch_add(1,Ordering::SeqCst);StatusCode::BAD_REQUEST}))
        .route("/ws",get(|State(m):State<Mock>,ws:WebSocketUpgrade|async move{ws.on_upgrade(move|mut socket|async move{
            m.connections.fetch_add(1,Ordering::SeqCst);let mut frames=m.frames.as_ref().unwrap().subscribe();
            loop{tokio::select!{
                message=socket.next()=>match message{
                    Some(Ok(Message::Text(t)))=>{let v:Value=serde_json::from_str(&t).unwrap();assert_eq!(v["params"],json!(["account.rfqUpdate"]));m.subscriptions.fetch_add(1,Ordering::SeqCst);},
                    Some(Ok(Message::Ping(b)))=>{let _=socket.send(Message::Pong(b)).await;},
                    Some(Ok(Message::Close(_)))|None=>break,_=>{}
                },
                frame=frames.recv()=>{let Ok(t)=frame else{break};if socket.send(Message::Text(t.into())).await.is_err(){break;}}
            }}
        })}))
        .with_state(m);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap()
        })),
    )
}
fn configured(root: &str, dir: &Path, inquiry_log: bool) -> Arc<BackpackStocks> {
    let mut s = BackpackStocks::new()
        .unwrap()
        .with_plan_store(dir.join("plans.jsonl"));
    if inquiry_log {
        s = s.with_rfq_store(dir.join("rfq.jsonl"));
    }
    s.credential_loader = keys;
    s.root = root.into();
    s.ws_url = root.replace("http://", "ws://") + "/ws";
    Arc::new(s)
}
fn native(r: &StockRfq) -> Value {
    json!({"rfqId":r.rfq_id,"clientId":r.client_id,"symbol":r.symbol,"side":r.request.side,"quantity":r.request.quantity,
        "executionMode":"AwaitAccept","status":"New","createdAt":r.created_at_ms,"submissionTime":r.submission_time_ms,"expiryTime":r.expiry_time_ms})
}
async fn prepared(
    root: &str,
    dir: &Path,
    m: &Mock,
) -> (Arc<BackpackStocks>, StockExecutionPlan, realtime::WsHub) {
    let s = configured(root, dir, true);
    let hub = realtime::WsHub::new(64);
    // Start the one RFQ socket before injecting fresh local preflight evidence.
    let now = common::time::now_ms();
    let (initial, _, _) = plans::tests::rfq_fixture(now);
    let r = &initial.rfqs[0];
    let (claim, _) = s
        .rfq_store
        .claim(
            r.request.clone(),
            &r.account_fingerprint,
            r.symbol.clone(),
            now,
        )
        .unwrap();
    let mut ready = r.clone();
    ready.client_id = claim.client_id;
    s.rfq_store
        .change(&ready.request.request_id, true, |r| {
            *r = ready.clone();
            Ok(true)
        })
        .unwrap();
    *m.native.lock() = native(&ready);
    s.ensure_rfq_started(hub.clone());
    until(|| s.rfq_subscription.borrow().is_some() && m.subscriptions.load(Ordering::SeqCst) > 0)
        .await;
    let now = common::time::now_ms();
    let (mut snapshot, account, request) = plans::tests::rfq_fixture(now);
    snapshot.rfqs[0].client_id = claim.client_id;
    let r = snapshot.rfqs[0].clone();
    s.rfq_store
        .change(&r.request.request_id, true, |old| {
            *old = r.clone();
            Ok(true)
        })
        .unwrap();
    *m.native.lock() = native(&r);
    *m.plan_path.lock() = dir.join("plans.jsonl");
    *s.snapshot.write() = snapshot;
    *s.rfq_problem.write() = None;
    s.account.write().fingerprint = account.fingerprint.clone();
    s.account.write().evidence = Some(account);
    let plan = s.reserve_plan(request, &hub).unwrap().plans[0].clone();
    (s, plan, hub)
}
async fn stop(s: &BackpackStocks) {
    let worker = s.rfq_worker.lock().take();
    if let Some(worker) = worker {
        worker.abort();
        let _ = worker.await;
    }
}
fn receipt(s: &BackpackStocks, plan: &StockExecutionPlan) -> StockRfq {
    s.plan_store
        .get(&plan.plan_id)
        .unwrap()
        .rfq_acceptance
        .unwrap()
}

#[tokio::test]
async fn stock_rfq_acceptance_signed_once_binding_actual_fill_and_plan_only_restart() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    m.emit.store(true, Ordering::SeqCst);
    let (root, _server) = server(m.clone()).await;
    let (s, p, hub) = prepared(&root, dir.path(), &m).await;
    let sent = s.send_rfq_leg(&p.plan_id, hub.clone()).await.unwrap();
    let r = sent.rfq_acceptance.unwrap();
    assert_eq!(r.phase, StockRfqPhase::AcceptedBinding);
    assert!(r.acceptance.as_ref().unwrap().acknowledged);
    assert!(r.executed_quantity.is_none());
    assert!(r.current_candidate(true, common::time::now_ms()).is_none());
    assert!(s
        .cancel_rfq(&r.request.request_id, hub.clone())
        .await
        .is_err());
    assert!(s.cancel_plan(&p.plan_id, &hub).is_err());
    s.send_rfq_leg(&p.plan_id, hub.clone()).await.unwrap();
    m.filled();
    m.frames
        .as_ref()
        .unwrap()
        .send(m.frame("rfqFilled"))
        .unwrap();
    until(|| receipt(&s, &p).phase == StockRfqPhase::Filled).await;
    assert!(receipt(&s, &p).settlement_pending());
    s.recheck_stock_order(&p.plan_id, hub.clone())
        .await
        .unwrap();
    let r = receipt(&s, &p);
    assert!(!r.settlement_pending());
    assert_eq!(r.fills.len(), 1);
    assert_eq!(r.executed_quantity.as_deref(), Some("0.02"));
    assert_eq!(r.executed_quote_quantity.as_deref(), Some("12.02"));
    let bytes = std::fs::metadata(dir.path().join("plans.jsonl"))
        .unwrap()
        .len();
    rfq_runtime::apply_frame(
        &s,
        &m.frame("rfqFilled"),
        &keys().unwrap().fingerprint(),
        common::time::now_ms(),
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(dir.path().join("plans.jsonl"))
            .unwrap()
            .len(),
        bytes
    );
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
    assert_eq!(m.connections.load(Ordering::SeqCst), 1);
    stop(&s).await;
    drop(s);
    // The acceptance/receipt survives even if the separate inquiry log is not configured.
    let restored = configured(&root, dir.path(), false);
    assert_eq!(restored.stock_rfq(&r.request.request_id).unwrap(), r);
    assert!(restored
        .plan_store
        .get(&p.plan_id)
        .unwrap()
        .holds_funds(common::time::now_ms() + 120_000));
    restored.send_rfq_leg(&p.plan_id, hub).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stock_rfq_acceptance_lost_ack_restart_bounded_reads_never_accepts_again() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    m.bad_ack.store(true, Ordering::SeqCst);
    m.empty_fills.store(true, Ordering::SeqCst);
    let (root, _server) = server(m.clone()).await;
    let (s, p, hub) = prepared(&root, dir.path(), &m).await;
    s.send_rfq_leg(&p.plan_id, hub.clone()).await.unwrap();
    assert!(!receipt(&s, &p).acceptance.unwrap().acknowledged);
    stop(&s).await;
    drop(s);
    m.filled();
    let s = configured(&root, dir.path(), false);
    s.resume_rfq(hub.clone());
    until(|| {
        receipt(&s, &p).phase == StockRfqPhase::Filled
            && receipt(&s, &p)
                .problem
                .as_deref()
                .is_some_and(|p| p.contains("暂无实际成交明细"))
    })
    .await;
    assert_eq!(receipt(&s, &p).settlement.attempts, 1);
    stop(&s).await;
    let id = p.terms.rfq.as_ref().unwrap().request_id.clone();
    for _ in 1..6 {
        s.change_rfq(&id, true, |r| {
            r.settlement.next_at_ms = Some(common::time::now_ms() - 1);
            Ok(true)
        })
        .unwrap();
        assert!(s
            .reconcile_rfq_with_mode(&id, &keys().unwrap(), true)
            .await
            .is_err());
    }
    let r = receipt(&s, &p);
    assert!(r.settlement.paused);
    assert_eq!(r.settlement.attempts, 6);
    let reads = m.fill_reads.load(Ordering::SeqCst);
    drop(s);
    let s = configured(&root, dir.path(), false);
    s.resume_rfq(hub.clone());
    assert!(s.rfq_worker.lock().is_none());
    s.reconcile_rfq_with_mode(&id, &keys().unwrap(), true)
        .await
        .unwrap();
    assert_eq!(m.fill_reads.load(Ordering::SeqCst), reads);
    m.empty_fills.store(false, Ordering::SeqCst);
    s.recheck_stock_order(&p.plan_id, hub.clone())
        .await
        .unwrap();
    assert!(!receipt(&s, &p).settlement_pending());
    s.send_rfq_leg(&p.plan_id, hub).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
    stop(&s).await;
}

#[tokio::test]
async fn stock_rfq_acceptance_rejects_stale_candidate_and_failed_intent_before_http() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let (s, p, hub) = prepared(&root, dir.path(), &m).await;
    let id = &p.terms.rfq.as_ref().unwrap().request_id;
    s.change_rfq(id, false, |r| {
        r.candidate.as_mut().unwrap().quote_id = "42".into();
        Ok(true)
    })
    .unwrap();
    assert!(s.send_rfq_leg(&p.plan_id, hub).await.is_err());
    assert!(s
        .plan_store
        .get(&p.plan_id)
        .unwrap()
        .rfq_acceptance
        .is_none());
    stop(&s).await;
    drop(s);
    let other = tempfile::tempdir().unwrap();
    let (s, p, hub) = prepared(&root, other.path(), &m).await;
    std::fs::rename(
        other.path().join("plans.jsonl"),
        other.path().join("plans.backup"),
    )
    .unwrap();
    std::fs::create_dir(other.path().join("plans.jsonl")).unwrap();
    assert!(s.send_rfq_leg(&p.plan_id, hub).await.is_err());
    assert!(s.plan_store.problem().is_some());
    assert_eq!(m.posts.load(Ordering::SeqCst), 0);
    stop(&s).await;
}

#[tokio::test]
async fn stock_rfq_acceptance_conflicting_quote_is_durable_and_never_released() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let (s, p, hub) = prepared(&root, dir.path(), &m).await;
    s.send_rfq_leg(&p.plan_id, hub.clone()).await.unwrap();
    let mut frame: Value = serde_json::from_str(&m.frame("rfqFilled")).unwrap();
    frame["data"]["u"] = json!("42");
    let apply = || {
        rfq_runtime::apply_frame(
            &s,
            &frame.to_string(),
            &keys().unwrap().fingerprint(),
            common::time::now_ms(),
        )
    };
    assert!(apply().is_err());
    let r = receipt(&s, &p);
    assert!(r.acceptance.as_ref().unwrap().evidence_conflict);
    assert!(r.fills.is_empty());
    assert!(!r.needs_follow_up());
    let bytes = std::fs::metadata(dir.path().join("plans.jsonl"))
        .unwrap()
        .len();
    assert!(apply().is_err());
    assert_eq!(
        std::fs::metadata(dir.path().join("plans.jsonl"))
            .unwrap()
            .len(),
        bytes
    );
    stop(&s).await;
    drop(s);
    let s = configured(&root, dir.path(), false);
    assert!(s.plan_store.problem().is_none());
    assert_eq!(receipt(&s, &p), r);
    assert!(s.cancel_plan(&p.plan_id, &hub).is_err());
    s.send_rfq_leg(&p.plan_id, hub).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stock_rfq_acceptance_definitive_rejection_is_redacted_and_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    m.reject.store(true, Ordering::SeqCst);
    let (root, _server) = server(m.clone()).await;
    let (s, p, hub) = prepared(&root, dir.path(), &m).await;
    s.send_rfq_leg(&p.plan_id, hub.clone()).await.unwrap();
    let r = receipt(&s, &p);
    assert!(r.acceptance.as_ref().unwrap().rejected);
    assert!(!r
        .problem
        .as_deref()
        .unwrap()
        .contains("do_not_expose_secret"));
    s.send_rfq_leg(&p.plan_id, hub).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    stop(&s).await;
}

#[tokio::test]
async fn stock_rfq_acceptance_open_new_recovers_binding_and_checks_quote_and_price() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    m.bad_ack.store(true, Ordering::SeqCst);
    let (root, _server) = server(m.clone()).await;
    let (s, p, hub) = prepared(&root, dir.path(), &m).await;
    s.send_rfq_leg(&p.plan_id, hub.clone()).await.unwrap();
    stop(&s).await;
    drop(s);
    let s = configured(&root, dir.path(), false);
    s.recheck_stock_order(&p.plan_id, hub.clone())
        .await
        .unwrap();
    let r = receipt(&s, &p);
    assert_eq!(r.phase, StockRfqPhase::AcceptedBinding);
    assert!(r.executed_quantity.is_none());
    assert!(!r.acceptance.as_ref().unwrap().acknowledged);
    let now = common::time::now_ms();
    let envelope: Value = serde_json::from_str(&m.frame("rfqFilled")).unwrap();
    for (field, value) in [
        ("u", json!("42")),
        ("S", json!("Bid")),
        ("q", json!("2")),
        ("p", json!("599")),
    ] {
        let mut frame = envelope["data"].clone();
        frame[field] = value;
        let mut probe = r.clone();
        assert!(
            rfq_protocol::apply_event(&mut probe, &frame, now).is_err(),
            "accepted wrong {field}"
        );
    }
    let mut history = m.native.lock().clone();
    history["deferredSettlementQuoteId"] = json!("42");
    let native = rfq_history::select(&serde_json::to_vec(&vec![history]).unwrap(), &r)
        .unwrap()
        .unwrap();
    assert!(rfq_history::apply(&mut r.clone(), native, now).is_err());
    let mut candidate = envelope["data"].clone();
    candidate["e"] = json!("rfqCandidate");
    candidate["u"] = json!("42");
    candidate["X"] = json!("New");
    assert!(!rfq_protocol::apply_event(&mut r.clone(), &candidate, now).unwrap());
    s.send_rfq_leg(&p.plan_id, hub).await.unwrap();
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
    stop(&s).await;
}

mod reconciliation;
