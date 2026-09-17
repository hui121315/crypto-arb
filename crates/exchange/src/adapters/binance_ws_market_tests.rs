use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_binance_schema() {
    let value: Value =
        serde_json::from_str(&subscribe_payload(&["BTCUSDT".into(), "ETH".into()])).unwrap();
    assert_eq!(value["method"], "SUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@depth20@100ms");
    assert_eq!(value["params"][1], "ethusdt@depth20@100ms");
    assert_eq!(value["id"], 1);
}

#[test]
fn unsubscribe_payload_matches_binance_schema() {
    let value: Value =
        serde_json::from_str(&unsubscribe_payload(&["BTC".into(), "ETHUSDT".into()])).unwrap();
    assert_eq!(value["method"], "UNSUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@depth20@100ms");
    assert_eq!(value["params"][1], "ethusdt@depth20@100ms");
}

#[test]
fn pending_subscription_batch_is_sorted_and_excludes_sent_symbols() {
    let subscriptions = DashMap::new();
    subscriptions.insert(
        "ethusdt".into(),
        SubscriptionState {
            last_touched_ms: 1,
            sent_on_current_connection: false,
        },
    );
    subscriptions.insert(
        "btcusdt".into(),
        SubscriptionState {
            last_touched_ms: 1,
            sent_on_current_connection: false,
        },
    );
    subscriptions.insert(
        "solusdt".into(),
        SubscriptionState {
            last_touched_ms: 1,
            sent_on_current_connection: true,
        },
    );

    assert_eq!(
        pending_subscription_symbols(&subscriptions),
        vec!["btcusdt".to_owned(), "ethusdt".to_owned()]
    );
}

#[test]
fn stream_symbol_normalizes_base_or_contract_symbol() {
    assert_eq!(stream_symbol("BTC"), "btcusdt");
    assert_eq!(stream_symbol("BTCUSDT"), "btcusdt");
    assert_eq!(stream_symbol("BTCUSDC"), "btcusdc");
    assert_eq!(depth_stream("ETH"), "ethusdt@depth20@100ms");
}

#[test]
fn parse_raw_partial_depth_extracts_book() {
    let parsed = parse_depth_snapshot(
        r#"{
            "e":"depthUpdate",
            "E":1700000000000,
            "T":1700000000001,
            "s":"BTCUSDT",
            "U":1,
            "u":2,
            "pu":0,
            "b":[["50000.5","1.5"],["50000.0","2.0"]],
            "a":[["50001.0","1.2"]]
        }"#,
    )
    .expect("depth");

    assert_eq!(parsed.stream_symbol, "btcusdt");
    assert_eq!(parsed.book.symbol, "BTC");
    assert_eq!(parsed.book.exchange, EXCHANGE);
    assert_eq!(parsed.book.timestamp, 1_700_000_000_000);
    assert_eq!(parsed.book.bids[0], [50_000.5, 1.5]);
    assert_eq!(parsed.book.asks[0], [50_001.0, 1.2]);
}

#[test]
fn parse_combined_partial_depth_extracts_data() {
    let parsed = parse_depth_snapshot(
        r#"{
            "stream":"ethusdt@depth20@100ms",
            "data":{
                "e":"depthUpdate",
                "E":1700000000100,
                "s":"ETHUSDT",
                "b":[["3000.0","4.0"]],
                "a":[["3001.0","5.0"]]
            }
        }"#,
    )
    .expect("depth");

    assert_eq!(parsed.stream_symbol, "ethusdt");
    assert_eq!(parsed.book.symbol, "ETH");
    assert_eq!(parsed.book.bids[0], [3_000.0, 4.0]);
}

#[test]
fn parse_depth_skips_invalid_levels() {
    let parsed = parse_depth_snapshot(
        r#"{
            "E":1700000000000,
            "s":"SOLUSDT",
            "b":[["bad","1.0"],["100.0","2.0"]],
            "a":[["101.0","x"],["101.5","3.0"]]
        }"#,
    )
    .expect("depth");

    assert_eq!(parsed.book.bids, vec![[100.0, 2.0]]);
    assert_eq!(parsed.book.asks, vec![[101.5, 3.0]]);
}

#[test]
fn stale_books_are_not_served() {
    // 10s 内新鲜，超过即视为冻结数据（断线/静默期间不得供给旧盘口）。
    assert!(book_is_fresh(1_000, 1_000 + BOOK_STALE_AFTER_MS));
    assert!(!book_is_fresh(1_000, 1_001 + BOOK_STALE_AFTER_MS));
}

#[test]
fn transport_failures_invalidate_depth_cache() {
    let (subscription_wakeup, _rx) = mpsc::channel(1);
    let stream = MarketStream {
        manager: test_manager(),
        books: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
        subscription_wakeup,
    };
    seed_book(&stream);
    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1))));
    assert!(stream.books.is_empty());
    seed_book(&stream);
    stream.handle_ws_event(WsEvent::CircuitOpened);
    assert!(stream.books.is_empty());
    seed_book(&stream);
    assert!(!stream.handle_dispatch_result(Err(RecvError::Closed)));
    assert!(stream.books.is_empty());
}

fn test_manager() -> Arc<WsManager> {
    Arc::new(WsManager::new(WsConfig {
        url: WS_URL.into(),
        exchange: EXCHANGE.into(),
        heartbeat_interval: Duration::from_secs(180),
        heartbeat: WsHeartbeat::PingFrame,
        inbound_codec: WsInboundCodec::Plain,
        server_ping: WsServerPing::None,
        initial_reconnect_delay: Duration::from_secs(1),
        max_reconnect_delay: Duration::from_secs(30),
        circuit_breaker_threshold: 10,
    }))
}

fn seed_book(stream: &MarketStream) {
    stream.on_text(r#"{"E":1,"s":"BTCUSDT","b":[["1","1"]],"a":[["2","1"]]}"#);
}
