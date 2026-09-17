use super::*;
use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::StreamExt;
use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

fn keys() -> Result<credentials::Credentials, String> {
    credentials::Credentials::parse(
        &STANDARD.encode(common::signing::ed25519_public_key(&[7; 32]).unwrap()),
        &STANDARD.encode([7; 32]),
    )
}
fn market_json() -> Value {
    json!({"symbol":"USDT_USDC","baseSymbol":"USDT","quoteSymbol":"USDC","marketType":"SPOT","orderBookState":"Open","filters":{"quantity":{"minQuantity":"1","maxQuantity":null,"stepSize":"1"},"price":{"tickSize":"0.0001"}}})
}
fn request(id: &str) -> StockExchangeConversionRequest {
    StockExchangeConversionRequest {
        request_id: id.into(),
        input_usdt: "10".into(),
        minimum_usdc: "9.98".into(),
    }
}
fn fixture(now: i64, id: &str) -> StockExchangeConversionPlan {
    let market = parse_market(&serde_json::to_vec(&market_json()).unwrap(), now).unwrap();
    let book = StockBookQuote {
        symbol: STOCK_CONVERSION_SYMBOL.into(),
        bid: Some("0.9997".into()),
        bid_quantity: Some("500".into()),
        ask: Some("0.9998".into()),
        ask_quantity: Some("600".into()),
        update_id: 1,
        source_at_ms: now,
        received_at_ms: now,
    };
    let a = account::parse(
        br#"{"spotMakerFee":"5","spotTakerFee":"10","liquidating":false}"#,
        br#"{"USDT":{"available":"20","locked":"300","staked":"0"}}"#,
        &keys().unwrap().fingerprint(),
        now,
    )
    .unwrap();
    let r = request(id);
    let terms = compile(&r, market, book, &a, now).unwrap();
    StockExchangeConversionPlan {
        plan_id: store::id(&r, &terms).unwrap(),
        request: r,
        terms,
        revision: 1,
        updated_at_ms: now,
        cancelled_at_ms: None,
        order: None,
    }
}

#[test]
fn stock_exchange_conversion_actual_fees_shortfall_partial_and_wrong_assets_hold_funds() {
    let now = common::time::now_ms();
    let mut p = fixture(now, "conversion-receipt-risks");
    let mut o = StockCexOrder::intent(now + 1);
    o.phase = StockCexOrderPhase::Filled;
    o.order_id = Some("100".into());
    o.executed_quantity = Some("10".into());
    o.executed_quote_quantity = Some("9.997".into());
    o.fills = vec![StockCexFill {
        trade_id: "200".into(),
        quantity: "10".into(),
        price: "0.9997".into(),
        fee: Some(StockTradeFee {
            asset: "USDC".into(),
            quantity: "0.009997".into(),
        }),
    }];
    p.order = Some(o.clone());
    assert!(p.accounting().is_ok());
    for (asset, quantity) in [("USDC", "0.02"), ("USDT", "0.001"), ("SOL", "0.000001")] {
        let mut bad = p.clone();
        bad.order.as_mut().unwrap().fills[0].fee = Some(StockTradeFee {
            asset: asset.into(),
            quantity: quantity.into(),
        });
        assert!(bad.accounting().is_err());
        assert!(bad.holds_funds(now + 1_000_000));
    }
    let mut missing = p.clone();
    missing.order.as_mut().unwrap().fills[0].fee = None;
    assert!(missing.accounting().is_err());
    assert!(missing.holds_funds(now + 1_000_000));
    let mut partial = p.clone();
    let o = partial.order.as_mut().unwrap();
    o.phase = StockCexOrderPhase::Expired;
    o.executed_quantity = Some("5".into());
    o.executed_quote_quantity = Some("4.9985".into());
    o.fills[0].quantity = "5".into();
    o.fills[0].fee.as_mut().unwrap().quantity = "0.0049985".into();
    assert!(partial.accounting().is_err());
    let mut improved = p.clone();
    let o = improved.order.as_mut().unwrap();
    o.executed_quote_quantity = Some("10.01".into());
    o.fills[0].price = "1.001".into();
    o.fills[0].fee.as_mut().unwrap().quantity = "0.01001".into();
    assert_eq!(improved.accounting().unwrap()["USDC"], "9.99999");
    let mut no_fill = p.clone();
    let o = no_fill.order.as_mut().unwrap();
    o.phase = StockCexOrderPhase::Cancelled;
    o.executed_quantity = Some("0".into());
    o.executed_quote_quantity = Some("0".into());
    o.fills.clear();
    assert!(!no_fill.holds_funds(now + 1));
    let mut unknown = no_fill;
    unknown.order.as_mut().unwrap().evidence_conflict = true;
    assert!(unknown.holds_funds(now + 1_000_000));
    let mut b = p.terms.book.clone();
    b.bid_quantity = Some("9".into());
    assert!(exchange_conversion_amounts(&p.request, &p.terms.market, &b, "10").is_err());
    let mut a = account::parse(
        br#"{"spotMakerFee":"5","spotTakerFee":"10","liquidating":false}"#,
        br#"{"USDT":{"available":"9","locked":"300","staked":"100"}}"#,
        &keys().unwrap().fingerprint(),
        now,
    )
    .unwrap();
    assert!(compile(
        &p.request,
        p.terms.market.clone(),
        p.terms.book.clone(),
        &a,
        now
    )
    .is_err());
    a.balances.get_mut("USDT").unwrap().available = "20".into();
    a.liquidating = true;
    assert!(compile(
        &p.request,
        p.terms.market.clone(),
        p.terms.book.clone(),
        &a,
        now
    )
    .is_err());
}
#[test]
fn stock_exchange_conversion_decimal_plan_claim_cancel_expiry_and_restart() {
    let now = common::time::now_ms();
    let p = fixture(now, "cex-conversion-plan-1");
    assert_eq!(p.terms.fee_budget_usdc, "0.009997");
    assert_eq!(p.terms.minimum_net_usdc, "9.987003");
    let mut wrong = p.request.clone();
    wrong.input_usdt = "10.5".into();
    assert!(exchange_conversion_amounts(&wrong, &p.terms.market, &p.terms.book, "10").is_err());
    wrong = p.request.clone();
    wrong.minimum_usdc = "10".into();
    assert!(exchange_conversion_amounts(&wrong, &p.terms.market, &p.terms.book, "10").is_err());
    let mut m = p.terms.market.clone();
    m.quote_symbol = "USD".into();
    assert!(exchange_conversion_amounts(&p.request, &m, &p.terms.book, "10").is_err());
    m = p.terms.market.clone();
    m.order_book_state = "Closed".into();
    assert!(exchange_conversion_amounts(&p.request, &m, &p.terms.book, "10").is_err());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("conversion.jsonl");
    let claims = Arc::new(crate::services::onchain_wallet_claims::WalletClaims::default());
    let s = store::Store::load(Some(path.clone()), claims.clone());
    s.insert(p.clone(), now).unwrap();
    assert!(s
        .previous(&p.request, &p.terms.account_fingerprint)
        .unwrap()
        .is_some());
    assert!(s
        .insert(fixture(now, "cex-conversion-plan-2"), now)
        .is_err());
    let hold = crate::services::onchain_wallet_claims::Hold {
        wallets: Default::default(),
        expires_at_ms: None,
    }
    .with_account("backpack_stocks", "configured-account")
    .unwrap();
    assert!(claims
        .commit(
            crate::services::onchain_wallet_claims::Owner::new(
                crate::services::onchain_wallet_claims::Module::Stocks,
                "other-stock-plan"
            ),
            Some(hold),
            now,
            || Ok(())
        )
        .is_err());
    s.change(&p.plan_id, now + 1, |p| {
        p.cancelled_at_ms = Some(now + 1);
        Ok(true)
    })
    .unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    s.change(&p.plan_id, now + 2, |_| Ok(false)).unwrap();
    assert_eq!(size, std::fs::metadata(&path).unwrap().len());
    let second = s
        .insert(fixture(now, "cex-conversion-plan-2"), now)
        .unwrap();
    assert!(!second.holds_funds(now + 10_000));
    drop(s);
    drop(claims);
    let restored = store::Store::load(Some(path.clone()), Default::default());
    assert!(restored.problem().is_none());
    assert_eq!(restored.get(&second.plan_id).unwrap(), second);
    drop(restored);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{")
        .unwrap();
    let broken = store::Store::load(Some(path), Default::default());
    assert!(broken.problem().is_some());
    assert!(broken
        .insert(fixture(now, "cex-conversion-plan-3"), now)
        .is_err());
}
#[derive(Clone)]
struct Remote {
    path: std::path::PathBuf,
    posts: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    order: Arc<Mutex<Option<Value>>>,
    frames: tokio::sync::broadcast::Sender<String>,
    fees: Arc<AtomicUsize>,
}
struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn verify(headers: &HeaderMap, instruction: &str, params: BTreeMap<String, String>) {
    let at = headers["x-timestamp"].to_str().unwrap().parse().unwrap();
    assert_eq!(headers["x-api-key"], keys().unwrap().public);
    assert_eq!(
        headers["x-signature"],
        keys().unwrap().signature(instruction, &params, at).unwrap()
    );
}
async fn server(path: std::path::PathBuf) -> (Remote, String, Server) {
    let r = Remote {
        path,
        posts: Default::default(),
        reads: Default::default(),
        order: Default::default(),
        frames: tokio::sync::broadcast::channel(32).0,
        fees: Default::default(),
    };
    let app=Router::new().route("/api/v1/market",get(||async{Json(market_json())}))
        .route("/api/v1/account",get(|h:HeaderMap|async move{verify(&h,"accountQuery",Default::default());Json(json!({"spotMakerFee":"5","spotTakerFee":"10","liquidating":false}))}))
        .route("/api/v1/capital",get(|h:HeaderMap|async move{verify(&h,"balanceQuery",Default::default());Json(json!({"USDT":{"available":"20","locked":"300","staked":"0"}}))}))
        .route("/api/v1/order",get(|State(s):State<Remote>,h:HeaderMap,Query(q):Query<BTreeMap<String,String>>|async move{
            verify(&h,"orderQuery",q.clone());assert_ne!(q.contains_key("orderId"),q.contains_key("clientId"));s.reads.fetch_add(1,Ordering::SeqCst);
            (StatusCode::NOT_FOUND,Json(json!({"code":"ORDER_NOT_FOUND","message":"not resting"})))
        }).post(|State(s):State<Remote>,h:HeaderMap,Json(v):Json<Value>|async move{
            verify(&h,"orderExecute",v.as_object().unwrap().iter().map(|(k,v)|(k.clone(),v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string()))).collect());
            assert_eq!(v["symbol"],"USDT_USDC");assert_eq!(v["side"],"Ask");assert_eq!(v["quantity"],"10");assert_eq!(v["price"],"0.9997");assert_eq!(v["timeInForce"],"FOK");
            for key in ["autoBorrow","autoBorrowRepay","autoLend","autoLendRedeem"]{assert_eq!(v[key],false);}assert!(v.get("reduceOnly").is_none());
            let log=std::fs::read_to_string(&s.path).unwrap();let entry:Value=serde_json::from_str(log.lines().last().unwrap()).unwrap();assert_eq!(entry["plan"]["order"]["phase"],"submission_unknown");
            s.posts.fetch_add(1,Ordering::SeqCst);let mut order=v;order["id"]=json!("900719925474099333");order["status"]=json!("Filled");order["executedQuantity"]=json!("10");order["executedQuoteQuantity"]=json!("9.997");*s.order.lock()=Some(order);
            (StatusCode::GATEWAY_TIMEOUT,Json(json!({"message":"fixture lost acknowledgement"})))
        }))
        .route("/wapi/v1/history/orders",get(|State(s):State<Remote>,h:HeaderMap,Query(q):Query<BTreeMap<String,String>>|async move{verify(&h,"orderHistoryQueryAll",q);s.reads.fetch_add(1,Ordering::SeqCst);Json(s.order.lock().clone().into_iter().collect::<Vec<_>>())}))
        .route("/wapi/v1/history/fills",get(|State(s):State<Remote>,h:HeaderMap,Query(q):Query<BTreeMap<String,String>>|async move{
            verify(&h,"fillHistoryQueryAll",q);s.reads.fetch_add(1,Ordering::SeqCst);let o=s.order.lock().clone().unwrap();
            Json(vec![json!({"clientId":o["clientId"],"orderId":o["id"],"symbol":"USDT_USDC","side":"Ask","tradeId":"900719925474099444","quantity":"10","price":"0.9997","fee":if s.fees.load(Ordering::SeqCst)==0{Value::Null}else{json!("0.009997")},"feeSymbol":"USDC"})])
        }))
        .route("/ws",get(|State(s):State<Remote>,ws:WebSocketUpgrade|async move{ws.on_upgrade(move|mut socket|async move {
            let mut frames=s.frames.subscribe();
            loop {tokio::select!{
                msg=socket.next()=>match msg {
                    Some(Ok(Message::Text(t)))=>{
                        let v:Value=serde_json::from_str(&t).unwrap();
                        if v["params"].as_array().is_some_and(|p|p.iter().any(|p|p=="bookTicker.USDT_USDC")) &&v["method"]=="SUBSCRIBE" {
                            let now=common::time::now_ms();let text=json!({"stream":"bookTicker.USDT_USDC","data":{"e":"bookTicker","s":"USDT_USDC","a":"0.9998","A":"600","b":"0.9997","B":"500","u":now as u64,"T":now*1000}}).to_string();if socket.send(Message::Text(text.into())).await.is_err(){break;}
                        }
                    },Some(Ok(Message::Ping(p)))=>{if socket.send(Message::Pong(p)).await.is_err(){break;}},Some(Ok(_))=>{},_=>break,
                },frame=frames.recv()=>if let Ok(t)=frame{if socket.send(Message::Text(t.into())).await.is_err(){break;}}
            }}
        })})).with_state(r.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        r,
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        })),
    )
}
fn service(path: std::path::PathBuf, root: &str) -> Arc<BackpackStocks> {
    let mut s = BackpackStocks::new()
        .unwrap()
        .with_exchange_conversion_store(path);
    s.root = root.into();
    s.ws_url = format!("{}/ws", root.replace("http:", "ws:"));
    s.credential_loader = keys;
    *s.snapshot.write() = super::super::comparison::tests::snapshot();
    s.snapshot.write().connected = false;
    Arc::new(s)
}
async fn stop(s: &BackpackStocks) {
    for task in [&s.worker, &s.monitor_worker, &s.rfq_worker, &s.alert_worker] {
        let worker = task.lock().take();
        if let Some(worker) = worker {
            worker.abort();
            let _ = worker.await;
        }
    }
}
#[tokio::test]
async fn stock_exchange_conversion_loopback_ws_submit_timeout_restart_fees_and_no_resend() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("conversion.jsonl");
    let (remote, root, _server) = server(path.clone()).await;
    let hub = realtime::WsHub::new(64);
    let _viewer = hub.subscribe(realtime::channels::STOCKS);
    let s = service(path.clone(), &root);
    let saved = s
        .build_exchange_conversion(request("conversion-live-fixture-1"), &hub)
        .await
        .unwrap();
    let p = saved.exchange_conversions[0].clone();
    assert_eq!(remote.posts.load(Ordering::SeqCst), 0);
    assert!(p.can_submit(common::time::now_ms()));
    let mut r = StockStablecoinSubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        confirm_live: false,
    };
    assert!(s
        .submit_conversion_with(r.clone(), hub.clone(), || Ok(()))
        .await
        .is_err());
    r.confirm_live = true;
    assert!(s
        .submit_conversion_with(r.clone(), hub.clone(), || Err("fixture paper mode".into()))
        .await
        .is_err());
    let pending = s
        .submit_conversion_with(r.clone(), hub.clone(), || Ok(()))
        .await
        .unwrap();
    assert!(pending.exchange_conversions[0].holds_funds(common::time::now_ms()));
    assert_eq!(remote.posts.load(Ordering::SeqCst), 1);
    s.submit_conversion_with(r.clone(), hub.clone(), || {
        panic!("retry cannot start a new order")
    })
    .await
    .unwrap();
    assert_eq!(remote.posts.load(Ordering::SeqCst), 1);
    assert!(s
        .cancel_exchange_conversion(
            StockPlanRevisionRequest {
                plan_id: p.plan_id.clone(),
                revision: 2
            },
            &hub
        )
        .is_err());
    stop(&s).await;
    drop(s);
    let s = service(path.clone(), &root);
    s.submit_conversion_with(r.clone(), hub.clone(), || panic!("restart cannot resubmit"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5100)).await;
    s.recheck_exchange_conversion(&p.plan_id, &hub)
        .await
        .unwrap();
    let missing = s.snapshot();
    let p = &missing.exchange_conversions[0];
    assert_eq!(p.order.as_ref().unwrap().phase, StockCexOrderPhase::Filled);
    assert!(p.accounting().is_err());
    assert!(p.holds_funds(common::time::now_ms()));
    remote.fees.store(1, Ordering::SeqCst);
    let o = remote.order.lock().clone().unwrap();
    let now = common::time::now_ms();
    let frame=json!({"stream":"account.orderUpdate","data":{"e":"orderFill","s":"USDT_USDC","c":o["clientId"],"S":"Ask","o":"LIMIT","f":"FOK","q":"10","p":"0.9997","X":"Filled","i":o["id"],"z":"10","Z":"9.997","t":"900719925474099444","l":"10","L":"0.9997","n":"0.009997","N":"USDC","T":now*1000}}).to_string();
    s.order_tracking_until_ms
        .store(now + 30_000, Ordering::SeqCst);
    s.ensure_rfq_started(hub.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while s.order_subscription.borrow().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    remote.frames.send(frame.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while s.snapshot().exchange_conversions[0].accounting().is_err() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let completed = s.snapshot();
    let p = &completed.exchange_conversions[0];
    assert_eq!(p.accounting().unwrap()["USDT"], "-10");
    assert_eq!(p.accounting().unwrap()["USDC"], "9.987003");
    assert!(!p.holds_funds(now));
    let rev = p.revision;
    assert!(!s
        .apply_conversion_frame(&frame, &keys().unwrap().fingerprint(), now)
        .unwrap());
    assert_eq!(s.snapshot().exchange_conversions[0].revision, rev);
    let reads = remote.reads.load(Ordering::SeqCst);
    s.recheck_exchange_conversion(&p.plan_id, &hub)
        .await
        .unwrap();
    assert_eq!(remote.reads.load(Ordering::SeqCst), reads);
    assert_eq!(remote.posts.load(Ordering::SeqCst), 1);
    if let Ok(path) = std::env::var("STOCK_EXCHANGE_CONVERSION_CAPTURE_PATH") {
        std::fs::write(
            path,
            serde_json::to_vec(
                &json!({"ready":saved,"pending":pending,"missing":missing,"completed":completed}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    stop(&s).await;
    drop(s);
    let restored = store::Store::load(Some(path), Default::default());
    assert!(restored.rows()[0].accounting().is_ok());
}
