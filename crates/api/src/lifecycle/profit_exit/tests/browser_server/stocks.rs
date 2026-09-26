//! Local public-data substitutes for one real backend/browser monitoring cycle.
use axum::{
    extract::{ws::{Message, WebSocketUpgrade}, Query},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use parking_lot::Mutex;
use serde_json::{json, Value};
use shared_types::stocks::{comparison::SOLANA_USDC, identity::backpack_issuer_profile};
use std::{collections::BTreeMap, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

#[derive(Default)]
struct Counters {
    quotes: Mutex<Vec<Value>>,
    quote_times_ms: Mutex<Vec<i64>>,
    quote_failure: AtomicUsize,
    rpc_batches: Mutex<Vec<Value>>,
    metadata: AtomicUsize,
    active_ws: AtomicUsize,
    max_ws: AtomicUsize,
    ws_subscriptions: Mutex<Vec<Vec<String>>>,
    unexpected: AtomicUsize,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuoteFailure {
    None,
    OneSell,
    All,
    ExpiringBuys,
}

pub(super) struct Fixture {
    pub root: String,
    counters: Arc<Counters>,
    hold: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) { self.task.abort(); }
}

impl Fixture {
    pub(super) async fn start() -> anyhow::Result<Self> {
        let capacity = std::env::var("CROSSLINE_PAPER_STOCK_CAPACITY").as_deref() == Ok("1");
        let assets = Arc::new(["MU.US".to_owned(), "SNDK.US".to_owned()].into_iter()
            .chain((1..=if capacity { 42 } else { 0 }).map(|i| format!("T{i:03}.US")))
            .collect::<Vec<_>>());
        let securities = assets.clone();
        let markets = assets.clone();
        let counters = Arc::new(Counters::default());
        let (hold, _) = tokio::sync::watch::channel(false);
        let metadata = counters.clone();
        let assets_count = counters.clone();
        let rpc = counters.clone();
        let quotes = counters.clone();
        let quote_hold = hold.clone();
        let streams = counters.clone();
        let unexpected = counters.clone();
        let app = Router::new()
            .route("/api/v1/securities", get(move || {
                metadata.metadata.fetch_add(1, Ordering::SeqCst);
                let securities = securities.clone();
                async move { Json(json!(securities.iter().map(|asset| {
                    let profile = backpack_issuer_profile(asset);
                    json!({"asset":asset,"name":profile.map(|p|p.native_name.to_owned()).unwrap_or_else(||format!("Fixture {asset}")),
                        "cusip":profile.and_then(|p|p.cusip),"sessions":[{"name":"US_EQUITIES_REGULAR",
                            "minQuantity":"0.01","stepSize":"0.01"}]})
                }).collect::<Vec<_>>())) }
            }))
            // Synthetic all-week calendar for deterministic local quote sizing, not market hours.
            .route("/api/v1/market-sessions", get(|| async { Json(json!([{
                "name":"US_EQUITIES_REGULAR","startTime":"00:00:00","endTime":"23:59:59",
                "timezone":"UTC","startWeekday":1,"endWeekday":7
            }])) }))
            .route("/api/v1/market-holidays", get(|| async { Json(json!([])) }))
            .route("/api/v1/markets", get(move || {
                let markets = markets.clone();
                async move { Json(json!(markets.iter().map(|asset| json!({
                    "symbol":format!("{asset}_USDC"),"baseSymbol":asset,"quoteSymbol":"USDC",
                    "marketType":"SPOT","rwaMarketType":"STOCK","orderBookState":"Open",
                    "filters":{"price":{"tickSize":"0.01"},"quantity":{"minQuantity":"0.01","stepSize":"0.01"}}
                })).collect::<Vec<_>>())) }
            }))
            .route("/api/v1/assets", get(move || {
                assets_count.metadata.fetch_add(1, Ordering::SeqCst);
                let assets = assets.clone();
                async move { Json(json!(assets.iter().map(|asset| json!({
                    "symbol":asset,"tokens":[{"blockchain":"Solana",
                        "contractAddress":fixture_mint(asset),
                        "nativeDecimals":6,"depositEnabled":false,"withdrawEnabled":false}]
                })).collect::<Vec<_>>())) }
            }))
            .route("/rpc", post(move |Json(body): Json<Value>| {
                let rpc = rpc.clone();
                async move {
                    assert_eq!(body["method"], "getMultipleAccounts");
                    assert_eq!(body["params"][1]["encoding"], "jsonParsed");
                    assert_eq!(body["params"][1]["commitment"], "finalized");
                    rpc.rpc_batches.lock().push(body["params"][0].clone());
                    let addresses = body["params"][0].as_array().unwrap();
                    let rows = addresses.iter().map(|address| {
                        if address.as_str() == Some("SysvarC1ock11111111111111111111111111111111") {
                            json!({"owner":"Sysvar1111111111111111111111111111111111111","executable":false,
                                "data":{"parsed":{"type":"clock","info":{"unixTimestamp":common::time::now_ms()/1000}}}})
                        } else {
                            json!({"owner":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA","executable":false,
                                "data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":6}}}})
                        }
                    }).collect::<Vec<_>>();
                    Json(json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":1},"value":rows}}))
                }
            }))
            .route("/quote", get(move |Query(params): Query<BTreeMap<String, String>>, headers: HeaderMap| {
                let quotes = quotes.clone();
                let mut hold = quote_hold.subscribe();
                async move {
                    assert!(!headers.contains_key("x-api-key"));
                    assert_eq!(params.len(), 3);
                    quotes.quotes.lock().push(json!(params));
                    quotes.quote_times_ms.lock().push(common::time::now_ms());
                    while *hold.borrow_and_update() {
                        if hold.changed().await.is_err() { break; }
                    }
                    let failure = quotes.quote_failure.load(Ordering::SeqCst);
                    if failure == QuoteFailure::All as usize
                        || (failure == QuoteFailure::OneSell as usize
                            && params["inputMint"] == backpack_issuer_profile("MU.US").unwrap().solana_mint)
                    {
                        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"isolated quote source unavailable"})));
                    }
                    let raw: u64 = params["amount"].parse().unwrap();
                    let output = if params["inputMint"] == SOLANA_USDC { raw/50 } else { raw*48 };
                    let expiry = (failure == QuoteFailure::ExpiringBuys as usize && params["inputMint"] == SOLANA_USDC)
                        .then(|| (chrono::Utc::now() + chrono::Duration::milliseconds(700)).to_rfc3339());
                    (StatusCode::OK, Json(json!({"inputMint":params["inputMint"],"outputMint":params["outputMint"],
                        "inAmount":params["amount"],"outAmount":output.to_string(),
                        "otherAmountThreshold":output.to_string(),"swapMode":"ExactIn","router":"isolated-fixture",
                        "expireAt":expiry})))
                }
            }))
            .route("/ws", get(move |ws: WebSocketUpgrade| {
                let streams = streams.clone();
                async move {
                    ws.on_upgrade(move |mut socket| async move {
                        let active = streams.active_ws.fetch_add(1, Ordering::SeqCst) + 1;
                        streams.max_ws.fetch_max(active, Ordering::SeqCst);
                        while let Some(Ok(message)) = socket.next().await {
                            match message {
                                Message::Text(text) => {
                                    let Ok(frame) = serde_json::from_str::<Value>(&text) else { continue };
                                    if frame["method"] != "SUBSCRIBE" { continue; }
                                    streams.ws_subscriptions.lock().push(frame["params"].as_array().unwrap()
                                        .iter().map(|v|v.as_str().unwrap().to_owned()).collect());
                                    for stream in frame["params"].as_array().unwrap() {
                                        let stream = stream.as_str().unwrap();
                                        let Some(symbol) = stream.strip_prefix("bookTicker.") else {
                                            assert!(stream.starts_with("stockPrice."));
                                            continue;
                                        };
                                        let now = common::time::now_ms();
                                        let body = json!({"stream":stream,"data":{"e":"bookTicker","s":symbol,
                                            "b":"49","B":"100","a":"51","A":"100","u":now.to_string(),"E":now*1000,"T":now*1000}});
                                        if socket.send(Message::Text(body.to_string().into())).await.is_err() { break; }
                                    }
                                }
                                Message::Ping(data) => { let _ = socket.send(Message::Pong(data)).await; }
                                Message::Close(_) => break,
                                _ => {}
                            }
                        }
                        streams.active_ws.fetch_sub(1, Ordering::SeqCst);
                    })
                }
            }))
            .fallback(move || {
                unexpected.unexpected.fetch_add(1, Ordering::SeqCst);
                async { StatusCode::NOT_FOUND }
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let root = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        Ok(Self { root, counters, hold, task })
    }

    pub(super) fn controls(&self) -> Router {
        let counters = self.counters.clone();
        let failures = self.counters.clone();
        let hold = self.hold.clone();
        Router::new().route("/__paper/stocks", get(move |headers: HeaderMap| {
            let counters = counters.clone();
            async move {
                authorized(&headers)?;
                Ok::<_, StatusCode>(Json(json!({
                    "quotes":*counters.quotes.lock(),"rpcBatches":*counters.rpc_batches.lock(),
                    "quoteTimesMs":*counters.quote_times_ms.lock(),
                    "metadataReads":counters.metadata.load(Ordering::SeqCst),
                    "activeWs":counters.active_ws.load(Ordering::SeqCst),
                    "maxWs":counters.max_ws.load(Ordering::SeqCst),
                    "wsSubscriptions":*counters.ws_subscriptions.lock(),
                    "unexpected":counters.unexpected.load(Ordering::SeqCst)
                })))
            }
        }).post(move |headers: HeaderMap, Json(block): Json<bool>| {
            let hold = hold.clone();
            async move {
                authorized(&headers)?;
                hold.send_replace(block);
                Ok::<_, StatusCode>(StatusCode::NO_CONTENT)
            }
        })).route("/__paper/stocks/quote-failure", post(move |headers: HeaderMap, Json(mode): Json<QuoteFailure>| {
            let counters = failures.clone();
            async move {
                authorized(&headers)?;
                counters.quote_failure.store(mode as usize, Ordering::SeqCst);
                Ok::<_, StatusCode>(StatusCode::NO_CONTENT)
            }
        }))
    }
}

fn fixture_mint(asset: &str) -> String {
    if let Some(profile) = backpack_issuer_profile(asset) { return profile.solana_mint.into(); }
    let mut bytes = [0_u8; 32];
    bytes[..asset.len()].copy_from_slice(asset.as_bytes());
    bs58::encode(bytes).into_string()
}

fn authorized(headers: &HeaderMap) -> Result<(), StatusCode> {
    if headers.get("authorization").and_then(|h| h.to_str().ok()) == Some("Bearer isolated-paper-browser") {
        Ok(())
    } else { Err(StatusCode::UNAUTHORIZED) }
}
