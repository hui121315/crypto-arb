use super::*;
use axum::{
    extract::ws::{Message, WebSocketUpgrade},
    routing::{get, post},
    Json, Router,
};
use exchange::{Kraken, KrakenConfig, KrakenCredentials, KrakenSpotCredentials};
use serde_json::{json, Value};
use std::time::Duration;

#[tokio::test]
async fn kraken_spot_socket_to_journal_preserves_fees_and_replay_does_not_double_fill() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route("/0/private/GetWebSocketsToken", post(|| async {
                Json(json!({"error": [], "result": {"token": "local-fixture", "expires": 900}}))
            }))
            .route("/private", get(|ws: WebSocketUpgrade| async {
                ws.on_upgrade(|mut socket| async move {
                    while let Some(Ok(Message::Text(text))) = socket.recv().await {
                        let request: Value = serde_json::from_str(&text).unwrap();
                        assert_eq!(request["method"], "subscribe", "test must not send orders");
                        if request["params"]["channel"] == "executions" {
                            assert_eq!(request["params"]["snap_trades"], true);
                            let mut frame: Value = serde_json::from_str(include_str!(
                                    "../../../../../exchange/fixtures/kraken/spot_v2_execution_update.json"
                            )).unwrap();
                            frame["type"] = json!("snapshot");
                            let row = &mut frame["data"][0];
                            row["cl_ord_id"] = json!("fixture-client");
                            row["side"] = json!("buy");
                            row["order_qty"] = json!(1.0);
                            row["last_qty"] = json!(0.4);
                            row["last_price"] = json!(100.0);
                            row["cum_qty"] = json!(0.4);
                            row["cum_cost"] = json!(40.0);
                            row["avg_price"] = json!(100.0);
                            row["fee_usd_equiv"] = json!(999.0);
                            row["fees"] = json!([{"asset":"USD","qty":0.04}]);
                            socket.send(Message::Text(frame.to_string().into())).await.unwrap();
                            frame["type"] = json!("update");
                            frame["sequence"] = json!(11);
                            let row = &mut frame["data"][0];
                            row["exec_id"] = json!("trade-2");
                            row["order_status"] = json!("filled");
                            row["last_qty"] = json!(0.6);
                            row["last_price"] = json!(120.0);
                            row["cum_qty"] = json!(1.0);
                            row["cum_cost"] = json!(112.0);
                            row["avg_price"] = json!(112.0);
                            row["fees"] = json!([{"asset":"USD","qty":0.072}]);
                            socket.send(Message::Text(frame.to_string().into())).await.unwrap();
                        }
                    }
                })
            }));
        axum::serve(listener, app).await.unwrap();
    });
    let adapter = Kraken::new(KrakenConfig {
        credentials: Some(KrakenCredentials {
            spot: Some(KrakenSpotCredentials {
                api_key: "local-fixture-key".into(),
                api_secret: "c2VjcmV0".into(),
            }),
            futures: None,
        }),
        spot_rest_url_override: Some(format!("http://{address}")),
        spot_private_ws_url_override: Some(format!("ws://{address}/private")),
        timeout_secs: 2,
        ..Default::default()
    })
    .unwrap();
    let mut events = adapter.subscribe_spot_executions().unwrap();
    let service = TradingService::new_mock();
    let mut order = intent("kraken-order", "fixture-client");
    order.exchange = "kraken".into();
    order.symbol = "BTC/USD".into();
    seed_accepted_order(&service, order, "OK4GJX-KSTLS-7DZZO5");
    adapter.warm_private_ws().await.unwrap();
    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        crate::trading_service::private_ws_mapper::apply_events(
            &service,
            crate::trading_service::private_ws_mapper::map_kraken_spot_execution(event),
        )
        .await;
    }
    for event in adapter.spot_execution_snapshot().unwrap() {
        crate::trading_service::private_ws_mapper::apply_events(
            &service,
            crate::trading_service::private_ws_mapper::map_kraken_spot_execution(event),
        )
        .await;
    }
    let ledger = service.list_execution_ledger_events();
    let fills: Vec<_> = ledger
        .iter()
        .filter(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
        .map(|event| match &event.payload {
            ExecutionLedgerPayload::FillSnapshot(fill) => fill,
            _ => panic!("fill expected"),
        })
        .collect();
    assert_eq!(
        fills.len(),
        2,
        "snapshot replay cannot add the same trades again"
    );
    assert!((fills.iter().map(|fill| fill.quantity).sum::<f64>() - 1.0).abs() < 1e-9);
    assert!((fills.iter().map(|fill| fill.quote_value).sum::<f64>() - 112.0).abs() < 1e-9);
    assert!(
        (fills
            .iter()
            .map(|fill| fill.fee.as_ref().unwrap().amount)
            .sum::<f64>()
            - 0.112)
            .abs()
            < 1e-9
    );
    assert!(fills
        .iter()
        .all(|fill| fill.fee.as_ref().unwrap().currency.as_deref() == Some("USD")));
    let record = service.get_order("kraken-order").unwrap();
    assert_eq!(record.state, shared_types::LiveOrderState::Filled);
    assert_eq!(record.filled_quantity, Some(1.0));
    assert_eq!(record.filled_price, Some(112.0));
    drop(adapter);
    server.abort();
    let _ = server.await;
}
