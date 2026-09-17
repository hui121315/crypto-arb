use super::*;
use crate::adapters::kraken_stock_receipts::tests::{frame, pending};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn stock_receipts_socket_replay_uses_existing_connection_without_order_requests() {
    let http = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/0/private/GetWebSocketsToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"error":[],"result":{"token":"local-receipt-token","expires":900}}),
        ))
        .expect(1)
        .mount(&http)
        .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let (replayed, replay_finished) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let mut subscriptions = Vec::new();
        let mut replayed = Some(replayed);
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                message = ws.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            let request:Value = serde_json::from_str(&text).unwrap();
                            assert_eq!(request["method"],"subscribe","receipt recovery must not place, cancel, or query orders");
                            let channel=request["params"]["channel"].as_str().unwrap().to_owned();
                            subscriptions.push(channel.clone());
                            if channel == "executions" {
                                assert_eq!(request["params"]["rebased"],true);
                                assert_eq!(request["params"]["snap_trades"],true);
                                assert_eq!(request["params"]["snap_orders"],true);
                                // Includes a replay snapshot with a reset sequence after finality.
                                for f in [frame(1,false),frame(2,true),frame(1,false),frame(2,true)] {
                                    ws.send(Message::Text(f.to_string())).await.unwrap();
                                }
                                if let Some(done)=replayed.take() { let _=done.send(()); }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => ws.send(Message::Pong(data)).await.unwrap(),
                        Some(Ok(Message::Pong(_))) => (),
                        Some(Ok(Message::Close(_))) | None => break,
                        other => panic!("unexpected local stock receipt frame {other:?}"),
                    }
                }
            }
        }
        subscriptions
    });
    let credentials = KrakenSpotCredentials {
        api_key: "local-only".into(),
        api_secret: "c2VjcmV0".into(),
    };
    let stream = KrakenSpotPrivateStream::new(
        &url,
        &http.uri(),
        &credentials,
        &HttpClient::new("local-stock-receipts").unwrap(),
        3,
    );
    stream.track_stock_order(pending()).unwrap();
    let mut receipts = stream.subscribe_stock_receipts();
    let mut generic = stream.subscribe_executions();
    stream.warm().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), replay_finished)
        .await
        .unwrap()
        .unwrap();
    let partial = tokio::time::timeout(Duration::from_secs(3), receipts.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(!partial.receipt_complete());
    let complete = tokio::time::timeout(Duration::from_secs(3), receipts.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(complete.receipt_complete());
    assert_eq!(complete.fills.len(), 2);
    assert_eq!(complete.cash_settlement().unwrap().quote_change, "599.9994");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), receipts.recv())
            .await
            .is_err(),
        "replay must not emit duplicate stock receipt revisions"
    );
    assert_eq!(
        stream.stock_order_receipt("stock-test-1").unwrap(),
        complete
    );
    assert!(
        generic.try_recv().unwrap().fill.is_some(),
        "existing account/ledger channel remains connected"
    );
    let _ = stop.send(());
    let channels = server.await.unwrap();
    assert_eq!(channels, vec!["executions", "balances"]);
    assert_eq!(http.received_requests().await.unwrap().len(), 1);
}
