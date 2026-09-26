use super::*;
mod public_probe;

#[test]
fn batch_late_row_cannot_restore_replaced_parameters_or_paused_quotes() {
    let service = BackpackStocks::new().unwrap();
    let rows = discover(&catalog(), &protocol::tests::assets(), &["AAPL.US".into()]).unwrap();
    let mut late = rows[0].clone();
    late.checked_at_ms = Some(10);
    service.batch.write().rows = rows;
    assert!(service.accept_batch_row(0, late.clone()));
    assert_eq!(service.batch.read().rows[0].checked_at_ms, Some(10));
    {
        let mut batch = service.batch.write();
        service.batch_generation.fetch_add(1, Ordering::SeqCst);
        batch.rows[0].checked_at_ms = None;
    }
    assert!(!service.accept_batch_row(0, late));
    assert_eq!(service.batch.read().rows[0].checked_at_ms, None);
}

fn catalog() -> StockCatalog {
    protocol::catalog(
        &protocol::tests::securities(),
        &protocol::tests::markets(),
        100,
    )
    .unwrap()
}

#[test]
fn batch_discovery_allows_observation_without_claiming_issuer_or_transfer_proof() {
    let rows = discover(
        &catalog(),
        &protocol::tests::assets(),
        &["AAPL.US".into(), "BRK.B.US".into()],
    )
    .unwrap();
    assert!(rows[0].token.is_some());
    assert!(!rows[0].issuer_verified);
    assert_eq!(rows[0].token.as_ref().unwrap().deposit_enabled, Some(false));
    assert!(rows[1].token.is_none() && rows[1].problem.is_some());
    let mut duplicate: serde_json::Value =
        serde_json::from_slice(&protocol::tests::assets()).unwrap();
    let first = duplicate[0].clone();
    duplicate.as_array_mut().unwrap().push(first);
    assert!(discover(
        &catalog(),
        &serde_json::to_vec(&duplicate).unwrap(),
        &["AAPL.US".into()]
    )
    .is_err());
    assert!(discover(
        &catalog(),
        &protocol::tests::assets(),
        &["UNKNOWN.US".into()]
    )
    .is_err());
}

#[test]
fn batch_known_issuer_conflict_is_not_downgraded_into_unverified_quote() {
    let mut catalog = catalog();
    catalog.rows[0].asset = "MU.US".into();
    let mut assets: serde_json::Value = serde_json::from_slice(&protocol::tests::assets()).unwrap();
    assets[0]["symbol"] = "MU.US".into();
    let rows = discover(
        &catalog,
        &serde_json::to_vec(&assets).unwrap(),
        &["MU.US".into()],
    )
    .unwrap();
    assert!(rows[0].token.is_none());
    assert!(rows[0].problem.as_deref().unwrap().contains("冲突"));
}

#[test]
fn batch_metadata_refresh_preserves_books_but_not_changed_token_quotes() {
    let mut previous =
        discover(&catalog(), &protocol::tests::assets(), &["AAPL.US".into()]).unwrap();
    let mut snapshot = StockMarketSnapshot {
        security: Some(previous[0].security.clone()),
        ..Default::default()
    };
    protocol::apply(&mut snapshot, &protocol::tests::frame(1000, false), 1000).unwrap();
    previous[0].books = snapshot.books;
    previous[0].connected = true;
    previous[0].checked_at_ms = Some(1000);
    let mut rows = discover(&catalog(), &protocol::tests::assets(), &["AAPL.US".into()]).unwrap();
    retain_quotes(&mut rows, &previous);
    assert!(rows[0].connected && !rows[0].books.is_empty());
    assert_eq!(rows[0].checked_at_ms, Some(1000));
    rows[0].token.as_mut().unwrap().native_decimals = Some(9);
    rows[0].checked_at_ms = None;
    retain_quotes(&mut rows, &previous);
    assert_eq!(rows[0].checked_at_ms, None);
}

#[test]
fn batch_request_bounds_and_pause_do_not_depend_on_valid_draft() {
    let mut request = StockBatchRequest {
        enabled: true,
        assets: vec!["AAPL.US".into()],
        budget_usdc: "100".into(),
        keyed: false,
        interval_secs: 5,
    };
    assert!(validate_request(&request).is_ok());
    request.assets.push("AAPL.US".into());
    assert!(validate_request(&request).is_err());
    request.assets = vec![String::new()];
    assert!(validate_request(&request).is_err());
    request.assets = (0..33).map(|i| format!("STOCK{i}.US")).collect();
    assert!(validate_request(&request).is_err());
    request.assets = vec!["AAPL.US".into()];
    request.interval_secs = 0;
    assert!(validate_request(&request).is_err());
    request.budget_usdc = "invalid".into();
    request.enabled = false;
    assert!(validate_request(&request).is_ok());
}

#[test]
fn batch_retry_backoff_is_bounded_and_success_restores_configured_cadence() {
    assert_eq!(retry_delay_ms(5, 0), 5_000);
    assert_eq!(retry_delay_ms(5, 1), 5_000);
    assert_eq!(retry_delay_ms(5, 3), 20_000);
    assert_eq!(retry_delay_ms(60, 99), 300_000);
    assert_eq!(retry_delay_ms(60, 0), 60_000);
}

#[tokio::test]
async fn batch_ws_shares_one_connection_and_releases_it_without_viewers() {
    use axum::{
        extract::ws::{Message, WebSocketUpgrade},
        routing::get,
        Router,
    };
    use std::sync::atomic::AtomicUsize;
    let opens = Arc::new(AtomicUsize::new(0));
    let count = opens.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route(
        "/ws",
        get(move |ws: WebSocketUpgrade| {
            let count = count.clone();
            async move {
                ws.on_upgrade(move |mut socket| async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    while let Some(Ok(message)) = socket.next().await {
                        match message {
                            Message::Text(text)
                                if text.contains("SUBSCRIBE") && !text.contains("UNSUBSCRIBE") =>
                            {
                                let message: serde_json::Value =
                                    serde_json::from_str(&text).unwrap();
                                let streams = message["params"].as_array().unwrap();
                                assert_eq!(
                                    streams
                                        .iter()
                                        .filter(|s| s.as_str() == Some("bookTicker.AAPL.US_USDC"))
                                        .count(),
                                    1
                                );
                                socket
                                    .send(Message::Text(
                                        protocol::tests::frame(common::time::now_ms(), false)
                                            .into(),
                                    ))
                                    .await
                                    .unwrap();
                            }
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                })
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut service = BackpackStocks::new().unwrap();
    service.ws_url = format!("ws://{address}/ws");
    let service = Arc::new(service);
    // Seed metadata directly: this test must never call a live RPC or quote provider.
    let rows = discover(&catalog(), &protocol::tests::assets(), &["AAPL.US".into()]).unwrap();
    service.snapshot.write().security = Some(rows[0].security.clone());
    *service.batch.write() = StockBatchStatus {
        request: Some(StockBatchRequest {
            enabled: true,
            assets: vec!["AAPL.US".into()],
            budget_usdc: "10".into(),
            keyed: false,
            interval_secs: 15,
        }),
        rows,
        ..Default::default()
    };
    let hub = realtime::WsHub::new(32);
    let mut viewer = hub.subscribe(realtime::channels::STOCKS);
    service.ensure_started(hub.clone());
    service.ensure_started(hub);
    let got = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let event = viewer.recv().await.unwrap().payload_json().unwrap();
            let snapshot: StockMarketSnapshot = serde_json::from_value(event).unwrap();
            if !snapshot.batch.rows[0].books.is_empty() {
                break snapshot;
            }
        }
    })
    .await
    .unwrap();
    assert!(got.batch.rows[0].connected);
    assert_eq!(got.batch.rows[0].books, got.books);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    drop(viewer);
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.snapshot().batch.rows[0].connected {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert!(service.snapshot().batch.rows[0].books.is_empty());
    drop(service);
    server.abort();
    let _ = server.await;
}
