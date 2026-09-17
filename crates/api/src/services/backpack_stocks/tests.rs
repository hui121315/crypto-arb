use super::*;
use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        State,
    },
    routing::get,
    Router,
};
use futures::StreamExt;
use protocol::tests::{assets, frame, markets, securities};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn backpack_stocks_socket_to_shared_snapshot_reuses_connection_and_suspends_without_viewers()
{
    let connections = Arc::new(AtomicUsize::new(0));
    let opens = connections.clone();
    let calendar_calls = Arc::new(AtomicUsize::new(0));
    let queried = calendar_calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let router = Router::new()
        .route("/api/v1/securities", get(|| async { securities() }))
        .route("/api/v1/markets", get(|| async { markets() }))
        .route("/api/v1/assets", get(|| async { assets() }))
        .route(
            "/api/v1/market-sessions",
            get(move || {
                queried.fetch_add(1, Ordering::SeqCst);
                async { calendar::tests::sessions() }
            }),
        )
        .route("/api/v1/market-holidays", get(|| async { "[]" }))
        .route(
            "/ws",
            get(
                |State(opens): State<Arc<AtomicUsize>>, ws: WebSocketUpgrade| async move {
                    ws.on_upgrade(move |mut socket| async move {
                        opens.fetch_add(1, Ordering::SeqCst);
                        while let Some(Ok(message)) = socket.next().await {
                            match message {
                                Message::Text(text)
                                    if text.contains("SUBSCRIBE")
                                        && !text.contains("UNSUBSCRIBE") =>
                                {
                                    assert!(text.contains("bookTicker.AAPL.US_USDC"));
                                    assert!(text.contains("stockPrice.AAPL"));
                                    let now = common::time::now_ms();
                                    for reference in [false, true] {
                                        socket
                                            .send(Message::Text(frame(now, reference).into()))
                                            .await
                                            .unwrap();
                                    }
                                }
                                Message::Close(_) => break,
                                _ => {}
                            }
                        }
                    })
                },
            ),
        )
        .with_state(opens);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut service = BackpackStocks::new().unwrap();
    service.root = format!("http://127.0.0.1:{port}");
    service.ws_url = format!("ws://127.0.0.1:{port}/ws");
    let service = Arc::new(service);
    let hub = realtime::WsHub::new(32);
    let mut rx = hub.subscribe(realtime::channels::STOCKS);
    service
        .watch(
            StockWatchRequest {
                asset: Some("AAPL.US".into()),
            },
            hub.clone(),
        )
        .await
        .unwrap();
    service.ensure_started(hub.clone());
    service.ensure_started(hub.clone());
    let got = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let value = rx.recv().await.unwrap().payload_json().unwrap();
            let s: StockMarketSnapshot = serde_json::from_value(value).unwrap();
            if s.reference.is_some()
                && !s.books.is_empty()
                && s.trading_route
                    .as_ref()
                    .is_some_and(|r| r.kind != StockRouteKind::Unknown)
            {
                break s;
            }
        }
    })
    .await
    .unwrap();
    assert!(got.connected);
    assert_eq!(got.books[0].bid_quantity.as_deref(), Some("2.5"));
    assert_eq!(got.tokens[0].deposit_enabled, Some(false));
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    assert_eq!(calendar_calls.load(Ordering::SeqCst), 1);
    let repeated = service
        .watch(
            StockWatchRequest {
                asset: Some("AAPL.US".into()),
            },
            hub.clone(),
        )
        .await
        .unwrap();
    assert!(repeated.connected);
    assert_eq!(repeated.books, got.books);
    assert_eq!(repeated.trading_route, got.trading_route);
    assert_eq!(calendar_calls.load(Ordering::SeqCst), 1);
    let second_viewer = hub.subscribe(realtime::channels::STOCKS);
    drop(rx);
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert!(service.snapshot().connected);
    drop(second_viewer);
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert!(!service.snapshot().connected);
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    drop(service);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
#[ignore = "Public read-only Backpack HTTP and WebSocket probe; run explicitly"]
async fn backpack_stocks_live_public_quotes() {
    public_quotes_probe(true, false).await;
}

#[tokio::test]
#[ignore = "Public read-only native stock BBO probe; external reference is reported separately"]
async fn backpack_stocks_live_public_native_quotes() {
    public_quotes_probe(false, false).await;
}

#[tokio::test]
#[ignore = "Public read-only stock and USDT/USDC BBO probe on the product's shared WebSocket"]
async fn backpack_stocks_live_public_conversion_quotes() {
    public_quotes_probe(false, true).await;
}

async fn public_quotes_probe(require_reference: bool, require_conversion: bool) {
    let service = Arc::new(BackpackStocks::new().unwrap());
    let catalog = service.catalog().await.unwrap();
    assert!(!catalog.rows.is_empty());
    let security = catalog
        .rows
        .iter()
        .find(|s| s.ticker == "MU" && !s.order_books.is_empty())
        .or_else(|| catalog.rows.iter().find(|s| !s.order_books.is_empty()))
        .expect("no officially listed stock spot book to probe");
    let hub = realtime::WsHub::new(64);
    let mut rx = hub.subscribe(realtime::channels::STOCKS);
    service
        .watch(
            StockWatchRequest {
                asset: Some(security.asset.clone()),
            },
            hub,
        )
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let s: StockMarketSnapshot =
                serde_json::from_value(rx.recv().await.unwrap().payload_json().unwrap()).unwrap();
            let now = common::time::now_ms();
            let native_current = s.connected
                && s.problem.is_none()
                && s.books.iter().any(|b| {
                    now.abs_diff(b.source_at_ms) < 3000 && b.bid.is_some() && b.ask.is_some()
                });
            let conversion_current = s.conversion_book_problem.is_none()
                && s.conversion_book.as_ref().is_some_and(|b| {
                    now.abs_diff(b.source_at_ms) < 3000
                        && now.abs_diff(b.received_at_ms) < 3000
                        && b.bid.is_some()
                        && b.ask.is_some()
                });
            if native_current
                && (!require_reference || s.reference.is_some())
                && (!require_conversion || conversion_current)
            {
                break s;
            }
        }
    })
    .await;
    let current = service.snapshot();
    println!("public stock stream: connected={} books={} reference={} native_problem={:?} reference_problem={:?}",
        current.connected, current.books.len(), current.reference.is_some(), current.problem, current.reference_problem);
    if require_conversion {
        println!(
            "public conversion BBO: {} problem={:?}",
            serde_json::to_string(&current.conversion_book).unwrap(),
            current.conversion_book_problem
        );
    }
    let snapshot = result.expect("required public stock feeds did not arrive within 25 seconds");
    println!(
        "verified native BBO: {}",
        serde_json::to_string(&snapshot.books).unwrap()
    );
    drop(rx);
    drop(service);
}
