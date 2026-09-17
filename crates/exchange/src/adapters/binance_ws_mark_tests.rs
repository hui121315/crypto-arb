use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscription_payload_matches_binance_schema() {
    let payload = subscription_payload(
        "SUBSCRIBE",
        &["btcusdt".to_owned(), "ethusdt".to_owned()],
        7,
    );
    let value: Value = serde_json::from_str(&payload).expect("json parses");
    assert_eq!(value["method"], "SUBSCRIBE");
    let params = value["params"].as_array().expect("params is array");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], "btcusdt@markPrice@1s");
    assert_eq!(params[1], "ethusdt@markPrice@1s");
    assert_eq!(value["id"], 7);
}

#[test]
fn unsubscribe_payload_uses_uppercase_method() {
    let payload = subscription_payload("UNSUBSCRIBE", &["btcusdt".to_owned()], 9);
    let value: Value = serde_json::from_str(&payload).expect("json parses");
    assert_eq!(value["method"], "UNSUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@markPrice@1s");
}

#[test]
fn stream_symbol_normalises_inputs() {
    assert_eq!(stream_symbol("BTC"), "btcusdt");
    assert_eq!(stream_symbol("BTCUSDT"), "btcusdt");
    assert_eq!(stream_symbol("BTC-USDC-SWAP"), "btcusdc");
    assert_eq!(stream_symbol("eth"), "ethusdt");
}

#[test]
fn parse_mark_price_update_extracts_funding_payload() {
    // Sample payload mirrors the docs:
    // <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Mark-Price-Stream>
    let raw = r#"{
        "e": "markPriceUpdate",
        "E": 1779511245529,
        "s": "BTCUSDT",
        "p": "75492.10",
        "ap": "75491.50",
        "i": "75525.13",
        "P": "75500.00",
        "r": "0.00007800",
        "T": 1779537600000
    }"#;
    let item = parse_mark_price_update(raw).expect("mark price parses");
    assert_eq!(item.symbol, "BTCUSDT");
    assert_eq!(item.event_time, 1_779_511_245_529);
    assert_eq!(item.mark_price, "75492.10");
    assert_eq!(item.index_price, "75525.13");
    assert_eq!(item.funding_rate, "0.00007800");
    assert_eq!(item.next_funding_time, 1_779_537_600_000);
}

#[test]
fn parse_mark_price_update_ignores_other_event_types() {
    // `kline` frames must not feed the mark-price cache.
    let raw = r#"{"e":"kline","E":1,"s":"BTCUSDT"}"#;
    assert!(parse_mark_price_update(raw).is_none());
}

#[test]
fn parse_funding_combines_ws_row_with_interval_cache() {
    let item = MarkPriceItem {
        event_type: "markPriceUpdate".into(),
        event_time: 1_779_511_245_529,
        symbol: "BTCUSDT".into(),
        mark_price: "75492.10".into(),
        index_price: "75525.13".into(),
        funding_rate: "0.000078".into(),
        next_funding_time: 1_779_537_600_000,
    };
    let funding = parse_funding(&item, 8).expect("funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "binance");
    assert!((funding.rate - 0.000_078).abs() < 1e-9);
    assert!((funding.rate_8h - 0.000_078).abs() < 1e-9);
    assert_eq!(funding.next_funding_time, 1_779_537_600_000);
    assert_eq!(funding.funding_interval, 8);
    assert_eq!(funding.timestamp, 1_779_511_245_529);
}

#[test]
fn parse_funding_normalises_4h_interval_to_rate_8h() {
    let item = MarkPriceItem {
        event_type: "markPriceUpdate".into(),
        event_time: 0,
        symbol: "ALTUSDT".into(),
        mark_price: "0".into(),
        index_price: String::new(),
        funding_rate: "0.0001".into(),
        next_funding_time: 1_779_537_600_000,
    };
    let funding = parse_funding(&item, 4).expect("funding parses");
    assert_eq!(funding.funding_interval, 4);
    assert!((funding.rate_8h - 0.0002).abs() < 1e-12);
}

#[test]
fn parse_funding_returns_none_when_rate_missing() {
    let item = MarkPriceItem {
        event_type: "markPriceUpdate".into(),
        event_time: 0,
        symbol: "BTCUSDT".into(),
        mark_price: "0".into(),
        index_price: String::new(),
        funding_rate: String::new(),
        next_funding_time: 0,
    };
    assert!(parse_funding(&item, 8).is_none());
}

#[test]
fn parse_funding_returns_none_when_next_settlement_is_missing() {
    let item = MarkPriceItem {
        event_type: "markPriceUpdate".into(),
        event_time: 1_779_511_245_529,
        symbol: "BTCUSDT".into(),
        mark_price: "75492.10".into(),
        index_price: "75525.13".into(),
        funding_rate: "0.000078".into(),
        next_funding_time: 0,
    };
    assert!(parse_funding(&item, 8).is_none());
}

#[test]
fn parse_mark_index_uses_documented_mark_and_index_fields() {
    let item = MarkPriceItem {
        event_type: "markPriceUpdate".into(),
        event_time: 1_779_511_245_529,
        symbol: "BTCUSDT".into(),
        mark_price: "75492.10".into(),
        index_price: "75525.13".into(),
        funding_rate: "0.000078".into(),
        next_funding_time: 1_779_537_600_000,
    };
    let row = parse_mark_index(&item).expect("mark/index parses");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "binance");
    assert!((row.mark_price - 75_492.10).abs() < 1e-9);
    assert_eq!(row.index_price, Some(75_525.13));
    assert_eq!(row.open_interest, None);
    assert_eq!(row.open_interest_value, None);
    assert_eq!(row.timestamp, 1_779_511_245_529);
}

#[test]
fn parse_mark_index_rejects_missing_mark_price() {
    let item = MarkPriceItem {
        event_type: "markPriceUpdate".into(),
        event_time: 0,
        symbol: "BTCUSDT".into(),
        mark_price: String::new(),
        index_price: "75525.13".into(),
        funding_rate: "0.000078".into(),
        next_funding_time: 0,
    };
    assert!(parse_mark_index(&item).is_none());
}

#[test]
fn transport_failures_invalidate_cached_mark_prices() {
    let stream = test_stream();
    seed_row(&stream);
    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1))));
    assert!(stream.rows.is_empty());

    seed_row(&stream);
    stream.handle_ws_event(WsEvent::Disconnected("test".into()));
    assert!(stream.rows.is_empty());

    seed_row(&stream);
    stream.handle_ws_event(WsEvent::CircuitOpened);
    assert!(stream.rows.is_empty());

    seed_row(&stream);
    assert!(!stream.handle_dispatch_result(Err(RecvError::Closed)));
    assert!(stream.rows.is_empty());
}

fn test_stream() -> MarkPriceStream {
    MarkPriceStream {
        manager: Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: EXCHANGE.into(),
            heartbeat_interval: Duration::from_secs(30),
            heartbeat: WsHeartbeat::PingFrame,
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        })),
        rows: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
        next_request_id: AtomicU64::new(1),
    }
}

fn seed_row(stream: &MarkPriceStream) {
    stream.on_text(
        r#"{"e":"markPriceUpdate","E":1,"s":"BTCUSDT","p":"1","i":"1","r":"0.0001","T":2}"#,
    );
}
