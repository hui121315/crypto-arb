use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscription_batches_partial_depth_twenty_streams() {
    let symbols = vec!["btcusdt".to_owned(), "solusdc".to_owned()];
    let value: Value =
        serde_json::from_str(&subscription_payload("SUBSCRIBE", &symbols, 7)).unwrap();
    assert_eq!(value["method"], "SUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@depth20@100ms");
    assert_eq!(value["params"][1], "solusdc@depth20@100ms");
    assert_eq!(value["id"], 7);
}

#[test]
fn parse_combined_snapshot_preserves_levels() {
    let raw = r#"{
      "stream":"btcusdt@depth20@100ms",
      "data":{"lastUpdateId":160,
        "bids":[["10","2"],["9","3"]],
        "asks":[["11","4"],["12","5"]]}
    }"#;
    let (symbol, book) = parse_snapshot(raw).expect("snapshot parses");
    assert_eq!(symbol, "btcusdt");
    assert_eq!(book.symbol, "BTC/USDT");
    assert_eq!(book.bids, vec![[10.0, 2.0], [9.0, 3.0]]);
    assert_eq!(book.asks, vec![[11.0, 4.0], [12.0, 5.0]]);
}

#[test]
fn circuit_open_invalidates_spot_depth_cache() {
    let stream = SpotDepthStream {
        manager: test_manager(),
        books: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
        pending_unsubscribes: Arc::new(DashMap::new()),
        next_request_id: AtomicU64::new(1),
    };
    stream.on_text(r#"{"stream":"btcusdt@depth20@100ms","data":{"lastUpdateId":1,"bids":[["1","1"]],"asks":[["2","1"]]}}"#);
    assert!(!stream.books.is_empty());

    stream.handle_event(WsEvent::CircuitOpened);

    assert!(stream.books.is_empty());
}

fn test_manager() -> Arc<WsManager> {
    Arc::new(WsManager::new(WsConfig {
        url: WS_URL.into(),
        exchange: EXCHANGE.into(),
        heartbeat_interval: Duration::from_secs(20),
        heartbeat: WsHeartbeat::PingFrame,
        inbound_codec: WsInboundCodec::Plain,
        server_ping: WsServerPing::None,
        initial_reconnect_delay: Duration::from_secs(1),
        max_reconnect_delay: Duration::from_secs(30),
        circuit_breaker_threshold: 10,
    }))
}
