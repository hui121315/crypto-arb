use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn lag_reports_are_aggregated_without_hiding_the_total() {
    let report = LagReportState::default();

    assert_eq!(report.observe(3, 10_000), Some(3));
    assert_eq!(report.observe(4, 11_000), None);
    assert_eq!(report.observe(5, 14_999), None);
    assert_eq!(report.observe(6, 15_000), Some(15));
}

#[test]
fn book_payload_uses_only_requested_symbol_streams() {
    let payload = book_subscription_payload("SUBSCRIBE", &["btcusdt".to_owned()], 17);
    let value: Value = serde_json::from_str(&payload).expect("json parses");
    assert_eq!(value["method"], "SUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@bookTicker");
    assert_eq!(value["params"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["id"], 17);
}

#[test]
fn stats_unsubscribe_payload_uses_only_requested_symbol_streams() {
    let payload = stats_subscription_payload("UNSUBSCRIBE", &["btcusdt".to_owned()], 18);
    let value: Value = serde_json::from_str(&payload).expect("json parses");
    assert_eq!(value["method"], "UNSUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@ticker");
    assert_eq!(value["params"].as_array().map(Vec::len), Some(1));
}

#[test]
fn stream_symbol_normalises_inputs() {
    assert_eq!(stream_symbol("BTC"), "btcusdt");
    assert_eq!(stream_symbol("BTCUSDT"), "btcusdt");
    assert_eq!(stream_symbol("BTCUSDC"), "btcusdc");
    assert_eq!(stream_symbol("eth"), "ethusdt");
}

#[test]
fn parse_ticker_update_extracts_24h_ticker() {
    let raw = r#"{
        "e": "24hrTicker",
        "E": 1779511245529,
        "s": "BTCUSDT",
        "c": "75492.10",
        "b": "75491.90",
        "a": "75492.20",
        "q": "3404219299.89529",
        "C": 1779511245000
    }"#;
    let parsed = parse_ticker_update(raw).expect("ticker parses");
    let BinanceTickerUpdate::Ticker(item) = parsed else {
        panic!("expected 24h ticker");
    };
    assert_eq!(item.symbol, "BTCUSDT");
    assert_eq!(item.last_price, "75492.10");
    assert_eq!(item.bid_price, "75491.90");
    assert_eq!(item.ask_price, "75492.20");
    assert_eq!(item.quote_volume, "3404219299.89529");
}

#[test]
fn parse_ticker_update_extracts_book_ticker() {
    let raw = r#"{
        "e": "bookTicker",
        "E": 1568014460893,
        "T": 1568014460891,
        "s": "BNBUSDT",
        "b": "25.35190000",
        "B": "31.21000000",
        "a": "25.36520000",
        "A": "40.66000000"
    }"#;
    let parsed = parse_ticker_update(raw).expect("book ticker parses");
    let BinanceTickerUpdate::Book(item) = parsed else {
        panic!("expected book ticker");
    };
    assert_eq!(item.symbol, "BNBUSDT");
    assert_eq!(item.bid_price, "25.35190000");
    assert_eq!(item.ask_price, "25.36520000");
}

#[test]
fn book_stream_caches_only_touched_symbols() {
    let stream = test_stream();
    stream.subscriptions.insert(
        "btcusdt".to_owned(),
        SubscriptionState {
            last_touched_ms: now_ms(),
        },
    );

    stream.on_text(r#"{"e":"bookTicker","E":1,"T":1,"s":"ETHUSDT","b":"1","a":"2"}"#);
    assert!(stream.books.is_empty());

    stream.on_text(r#"{"e":"bookTicker","E":2,"T":2,"s":"BTCUSDT","b":"3","a":"4"}"#);
    assert!(stream.books.contains_key("btcusdt"));
    assert!(!stream.books.contains_key("ethusdt"));
}

#[test]
fn parse_ticker_combines_24h_and_book_rows() {
    let ticker = Ticker24hUpdate {
        event_time: 1_779_511_245_529,
        symbol: "BTCUSDT".into(),
        last_price: "75492.10".into(),
        quote_volume: "3404219299.89529".into(),
        bid_price: "75491.8".into(),
        ask_price: "75492.3".into(),
        close_time: 1_779_511_245_500,
    };
    let book = BookTickerUpdate {
        event_time: 1_779_511_245_600,
        transaction_time: 1_779_511_245_599,
        symbol: "BTCUSDT".into(),
        bid_price: "75491.9".into(),
        ask_price: "75492.2".into(),
    };
    let row = parse_ticker(&ticker, Some(&book)).expect("ws ticker parses");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "binance");
    assert!((row.bid - 75_491.9).abs() < 1e-6);
    assert!((row.ask - 75_492.2).abs() < 1e-6);
    assert!((row.last - 75_492.10).abs() < 1e-6);
    assert!((row.volume_24h - 3_404_219_299.895_29).abs() < 1e-3);
    assert_eq!(row.timestamp, 1_779_511_245_600);
}

#[test]
fn parse_ticker_update_ignores_ack_and_other_events() {
    assert!(parse_ticker_update(r#"{"result":null,"id":17}"#).is_none());
    assert!(parse_ticker_update(r#"{"e":"kline","s":"BTCUSDT"}"#).is_none());
}

#[test]
fn parse_ticker_falls_back_to_ticker_bbo_when_book_is_incomplete() {
    let ticker = Ticker24hUpdate {
        event_time: 1_779_511_245_529,
        symbol: "BTCUSDT".into(),
        last_price: "75492.10".into(),
        quote_volume: "3404219299.89529".into(),
        bid_price: "75491.8".into(),
        ask_price: "75492.3".into(),
        close_time: 1_779_511_245_500,
    };
    let book = BookTickerUpdate {
        event_time: 1_779_511_245_600,
        transaction_time: 1_779_511_245_599,
        symbol: "BTCUSDT".into(),
        bid_price: String::new(),
        ask_price: "75492.2".into(),
    };
    let row = parse_ticker(&ticker, Some(&book)).expect("ticker BBO remains usable");
    assert!((row.bid - 75_491.8).abs() < 1e-6);
    assert!((row.ask - 75_492.3).abs() < 1e-6);
}

#[test]
fn parse_ticker_uses_complete_ticker_before_book_first_frame() {
    let ticker = Ticker24hUpdate {
        event_time: 1_779_511_245_529,
        symbol: "BTCUSDT".into(),
        last_price: "75492.10".into(),
        quote_volume: "3404219299.89529".into(),
        bid_price: "75491.9".into(),
        ask_price: "75492.2".into(),
        close_time: 1_779_511_245_500,
    };
    let row = parse_ticker(&ticker, None).expect("ticker is a complete baseline");
    assert!((row.bid - 75_491.9).abs() < 1e-6);
    assert!((row.ask - 75_492.2).abs() < 1e-6);
    assert!((row.volume_24h - 3_404_219_299.895_29).abs() < 1e-3);
}

#[test]
fn assemble_from_book_uses_mid_before_ticker_first_frame() {
    let book = BookTickerUpdate {
        event_time: 1_785_088_000_000,
        transaction_time: 0,
        symbol: "ETHUSDT".into(),
        bid_price: "2400.0".into(),
        ask_price: "2400.2".into(),
    };
    let row = assemble_from_book(&book).expect("assembles before ticker first frame");
    // last 退化为 mid，24h 量为 0（流动性过滤中的保守方向）。
    assert!((row.last - 2400.1).abs() < 1e-9);
    assert!((row.volume_24h - 0.0).abs() < f64::EPSILON);
}

#[test]
fn assemble_from_book_drops_blank_bid() {
    let book = BookTickerUpdate {
        event_time: 0,
        transaction_time: 0,
        symbol: "BTCUSDT".into(),
        bid_price: String::new(),
        ask_price: "1".into(),
    };
    assert!(assemble_from_book(&book).is_none());
}

#[test]
fn transport_failures_invalidate_only_the_affected_socket_cache() {
    let stream = test_stream();
    seed_transport_rows(&stream);

    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1)), TickerSocket::Book));
    assert!(stream.books.is_empty());
    assert!(!stream.tickers.is_empty());

    seed_transport_rows(&stream);
    stream.handle_ws_event(WsEvent::CircuitOpened, TickerSocket::Stats);
    assert!(stream.tickers.is_empty());
    assert!(!stream.books.is_empty());

    assert!(!stream.handle_dispatch_result(Err(RecvError::Closed), TickerSocket::Book));
    assert!(stream.books.is_empty());
}

fn test_stream() -> TickerStream {
    let manager = || {
        Arc::new(WsManager::new(WsConfig {
            url: WS_PUBLIC_URL.into(),
            exchange: EXCHANGE.into(),
            heartbeat_interval: Duration::from_secs(30),
            heartbeat: WsHeartbeat::PingFrame,
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        }))
    };
    TickerStream {
        book_manager: manager(),
        stats_manager: manager(),
        tickers: Arc::new(DashMap::new()),
        books: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
        next_request_id: AtomicU64::new(1),
        lag_report: LagReportState::default(),
    }
}

fn seed_transport_rows(stream: &TickerStream) {
    stream.tickers.insert(
        "btcusdt".to_owned(),
        CachedTicker {
            item: Ticker24hUpdate {
                event_time: 1,
                symbol: "BTCUSDT".to_owned(),
                last_price: "1".to_owned(),
                quote_volume: "1".to_owned(),
                bid_price: "1".to_owned(),
                ask_price: "2".to_owned(),
                close_time: 1,
            },
            cached_at_ms: 1,
        },
    );
    stream.books.insert(
        "btcusdt".to_owned(),
        CachedBook {
            item: BookTickerUpdate {
                event_time: 1,
                transaction_time: 1,
                symbol: "BTCUSDT".to_owned(),
                bid_price: "1".to_owned(),
                ask_price: "2".to_owned(),
            },
            cached_at_ms: 1,
        },
    );
}
