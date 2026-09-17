use super::*;
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

#[test]
fn subscription_payload_matches_binance_spot_schema() {
    let payload = subscription_payload(
        "SUBSCRIBE",
        &["btcusdt".to_owned(), "ethusdc".to_owned()],
        17,
    );
    let value: Value = serde_json::from_str(&payload).expect("json parses");
    assert_eq!(value["method"], "SUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@ticker");
    assert_eq!(value["params"][1], "btcusdt@bookTicker");
    assert_eq!(value["params"][2], "ethusdc@ticker");
    assert_eq!(value["params"][3], "ethusdc@bookTicker");
    assert_eq!(value["params"].as_array().unwrap().len(), 4);
    assert_eq!(value["id"], 17);
}

#[test]
fn unsubscribe_payload_uses_same_stream_names() {
    let payload = subscription_payload("UNSUBSCRIBE", &["btcusdt".to_owned()], 18);
    let value: Value = serde_json::from_str(&payload).expect("json parses");
    assert_eq!(value["method"], "UNSUBSCRIBE");
    assert_eq!(value["params"][0], "btcusdt@ticker");
    assert_eq!(value["params"][1], "btcusdt@bookTicker");
    assert_eq!(value["params"].as_array().unwrap().len(), 2);
}

#[test]
fn stream_symbol_normalizes_pair_inputs() {
    assert_eq!(stream_symbol("BTC"), Some("btcusdt".to_owned()));
    assert_eq!(stream_symbol("BTC/USDT"), Some("btcusdt".to_owned()));
    assert_eq!(stream_symbol("eth-usdc"), Some("ethusdc".to_owned()));
    assert_eq!(stream_symbol(""), None);
}

#[test]
fn parse_ticker_update_extracts_24h_ticker() {
    let raw = r#"{
        "e": "24hrTicker",
        "E": 1672515782136,
        "s": "BNBBTC",
        "c": "0.0025",
        "q": "18",
        "b": "0.0024",
        "B": "10",
        "a": "0.0026",
        "A": "100",
        "C": 1672515782000
    }"#;
    let parsed = parse_ticker_update(raw).expect("ticker parses");
    let BinanceSpotTickerUpdate::Ticker(parsed) = parsed else {
        panic!("expected 24h ticker");
    };
    assert_eq!(parsed.symbol, "BNBBTC");
    assert_eq!(parsed.last_price, "0.0025");
    assert_eq!(parsed.quote_volume, "18");
    assert_eq!(parsed.bid_price, "0.0024");
    assert_eq!(parsed.ask_price, "0.0026");
}

#[test]
fn parse_all_market_array_extracts_complete_tickers() {
    let raw = r#"[{"e":"24hrTicker","E":1720000000000,"s":"BTCUSDT","c":"60000","q":"1000000","b":"59999","B":"2","a":"60001","A":"3","C":1720000000001},{"e":"24hrTicker","E":1720000000002,"s":"ETHUSDT","c":"3000","q":"500000","b":"2999","B":"4","a":"3001","A":"5","C":1720000000003}]"#;
    let rows = parse_ticker_updates(raw);
    assert_eq!(rows.len(), 2);
    assert!(matches!(
        &rows[0],
        BinanceSpotTickerUpdate::Ticker(row) if row.symbol == "BTCUSDT"
    ));
    assert!(matches!(
        &rows[1],
        BinanceSpotTickerUpdate::Ticker(row) if row.symbol == "ETHUSDT"
    ));
}

#[test]
fn parse_combined_all_market_array_extracts_complete_tickers() {
    let raw = r#"{"stream":"!ticker@arr","data":[{"e":"24hrTicker","E":1720000000000,"s":"BTCUSDT","c":"60000","q":"1000000","b":"59999","B":"2","a":"60001","A":"3","C":1720000000001}]}"#;
    let rows = parse_ticker_updates(raw);
    assert!(matches!(
        rows.as_slice(),
        [BinanceSpotTickerUpdate::Ticker(row)] if row.symbol == "BTCUSDT"
    ));
}

#[test]
fn parse_ticker_update_extracts_realtime_book_ticker() {
    let raw = r#"{
        "u": 400900217,
        "s": "BNBUSDT",
        "b": "25.35190000",
        "B": "31.21000000",
        "a": "25.36520000",
        "A": "40.66000000"
    }"#;
    let parsed = parse_ticker_update(raw).expect("book ticker parses");
    let BinanceSpotTickerUpdate::Book(parsed) = parsed else {
        panic!("expected book ticker");
    };
    assert_eq!(parsed.symbol, "BNBUSDT");
    assert_eq!(parsed.bid_price, "25.35190000");
    assert_eq!(parsed.ask_qty, "40.66000000");
}

#[test]
fn parse_combined_realtime_book_ticker() {
    let raw = r#"{"stream":"solusdt@bookTicker","data":{"u":400900217,"s":"SOLUSDT","b":"150.10","B":"31.21","a":"150.11","A":"40.66"}}"#;
    let parsed = parse_ticker_update(raw).expect("combined book ticker parses");
    assert!(matches!(
        parsed,
        BinanceSpotTickerUpdate::Book(row)
            if row.symbol == "SOLUSDT" && row.bid_price == "150.10"
    ));
}

#[test]
fn parse_spot_tick_prefers_realtime_book_and_preserves_frame_time() {
    let ticker = SpotTicker24hUpdate {
        event_type: EVENT_24H_TICKER.to_owned(),
        event_time: 1_672_515_782_136,
        symbol: "BNBUSDT".into(),
        last_price: "25.36000000".into(),
        quote_volume: "1800000.5".into(),
        bid_price: "25.35190000".into(),
        bid_qty: "31.21000000".into(),
        ask_price: "25.36520000".into(),
        ask_qty: "40.66000000".into(),
        close_time: 1_672_515_782_000,
    };
    let book = SpotBookTickerUpdate {
        symbol: "BNBUSDT".into(),
        bid_price: "25.35500000".into(),
        bid_qty: "50.00000000".into(),
        ask_price: "25.35600000".into(),
        ask_qty: "60.00000000".into(),
    };
    let row = parse_spot_tick(&ticker, Some(&book), 1_672_515_782_200).expect("spot tick");

    assert_eq!(row.venue, "binance");
    assert_eq!(row.symbol, "BNB/USDT");
    assert_eq!(row.bid, dec("25.35500000"));
    assert_eq!(row.ask, dec("25.35600000"));
    assert_eq!(row.last, dec("25.36000000"));
    assert_eq!(row.bid_size, Some(dec("50.00000000")));
    assert_eq!(row.ask_size, Some(dec("60.00000000")));
    assert_eq!(row.volume_24h, dec("1800000.5"));
    assert_eq!(row.exchange_ts_ms, Some(1_672_515_782_136));
    assert_eq!(row.received_at_ms, 1_672_515_782_200);
}

#[test]
fn parse_spot_tick_uses_ticker_bbo_until_book_arrives() {
    let ticker = SpotTicker24hUpdate {
        event_type: EVENT_24H_TICKER.to_owned(),
        event_time: 10,
        symbol: "SOLUSDT".into(),
        last_price: "150".into(),
        quote_volume: "1000000".into(),
        bid_price: "149.9".into(),
        bid_qty: "12".into(),
        ask_price: "150.1".into(),
        ask_qty: "13".into(),
        close_time: 9,
    };
    let row = parse_spot_tick(&ticker, None, 1234).expect("ticker BBO remains usable");
    assert_eq!(row.bid, dec("149.9"));
    assert_eq!(row.ask, dec("150.1"));
    assert_eq!(row.received_at_ms, 1234);
}

#[test]
fn parse_ticker_update_ignores_ack_and_other_events() {
    assert!(parse_ticker_update(r#"{"result":null,"id":17}"#).is_none());
    assert!(parse_ticker_update(r#"{"e":"aggTrade","s":"BTCUSDT"}"#).is_none());
}

#[test]
fn parses_official_subscription_ack_and_error_shapes() {
    assert_eq!(
        parse_control_response(r#"{"result":null,"id":17}"#),
        Some(ControlResponse {
            id: 17,
            code: None,
            message: None,
        })
    );
    assert_eq!(
        parse_control_response(
            r#"{"code":2,"msg":"Invalid request: too many parameters","id":18}"#
        ),
        Some(ControlResponse {
            id: 18,
            code: Some(2),
            message: Some("Invalid request: too many parameters".to_owned()),
        })
    );
}

#[test]
fn subscription_ack_and_rejection_are_visible_per_symbol() {
    let stream = test_stream();
    seed_subscription(&stream, "btcusdt", 17);
    stream.on_text(r#"{"result":null,"id":17}"#);
    assert!(stream
        .subscription_problem("BTC/USDT")
        .is_some_and(|problem| problem.contains("订阅已确认")));

    seed_subscription(&stream, "ethusdc", 18);
    stream.on_text(r#"{"code":2,"msg":"Invalid request: too many parameters","id":18}"#);
    assert!(stream
        .subscription_problem("ETH/USDC")
        .is_some_and(|problem| problem.contains("code 2")));
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("valid decimal")
}

#[test]
fn lag_keeps_complete_snapshots_while_transport_failures_clear_cache() {
    let stream = test_stream();
    seed_transport_rows(&stream);

    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1))));
    assert_eq!(stream.tickers.len(), 1);
    assert_eq!(stream.books.len(), 1);

    seed_transport_rows(&stream);
    stream.handle_ws_event(WsEvent::Disconnected("test".to_owned()));
    assert!(stream.tickers.is_empty());
    assert!(stream.books.is_empty());

    seed_transport_rows(&stream);
    stream.handle_ws_event(WsEvent::CircuitOpened);
    assert!(stream.tickers.is_empty());
    assert!(stream.books.is_empty());

    seed_transport_rows(&stream);
    assert!(!stream.handle_dispatch_result(Err(RecvError::Closed)));
    assert!(stream.tickers.is_empty());
    assert!(stream.books.is_empty());
}

#[test]
fn all_market_frames_only_write_requested_symbols() {
    let stream = test_stream();
    let raw = r#"[{"e":"24hrTicker","E":1720000000000,"s":"BTCUSDT","c":"60000","q":"1000000","b":"59999","B":"2","a":"60001","A":"3","C":1720000000001}]"#;

    stream.on_text(raw);
    assert!(stream.tickers.is_empty());

    stream.subscriptions.insert(
        "btcusdt".to_owned(),
        SubscriptionState {
            last_touched_ms: now_ms(),
            sent_on_current_connection: false,
            sent_at_ms: None,
            acknowledged_on_current_connection: false,
        },
    );
    stream.on_text(raw);
    assert_eq!(stream.tickers.len(), 1);
}

fn test_stream() -> SpotTickerStream {
    SpotTickerStream {
        manager: Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: WS_NAME.into(),
            heartbeat_interval: Duration::from_secs(20),
            heartbeat: WsHeartbeat::PingFrame,
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        })),
        tickers: Arc::new(DashMap::new()),
        books: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
        pending_subscriptions: Arc::new(DashMap::new()),
        subscription_problems: Arc::new(DashMap::new()),
        pending_unsubscribes: Arc::new(DashMap::new()),
        next_request_id: AtomicU64::new(1),
    }
}

fn seed_subscription(stream: &SpotTickerStream, symbol: &str, request_id: u64) {
    stream.subscriptions.insert(
        symbol.to_owned(),
        SubscriptionState {
            last_touched_ms: now_ms(),
            sent_on_current_connection: true,
            sent_at_ms: Some(now_ms()),
            acknowledged_on_current_connection: false,
        },
    );
    stream
        .pending_subscriptions
        .insert(request_id, vec![symbol.to_owned()]);
}

fn seed_transport_rows(stream: &SpotTickerStream) {
    stream.tickers.insert(
        "btcusdt".to_owned(),
        CachedTicker {
            item: SpotTicker24hUpdate {
                event_type: EVENT_24H_TICKER.to_owned(),
                event_time: 1,
                symbol: "BTCUSDT".to_owned(),
                last_price: "1".to_owned(),
                quote_volume: "1".to_owned(),
                bid_price: "1".to_owned(),
                bid_qty: "1".to_owned(),
                ask_price: "2".to_owned(),
                ask_qty: "1".to_owned(),
                close_time: 1,
            },
            cached_at_ms: 1,
        },
    );
    stream.books.insert(
        "btcusdt".to_owned(),
        CachedBook {
            item: SpotBookTickerUpdate {
                symbol: "BTCUSDT".to_owned(),
                bid_price: "1".to_owned(),
                bid_qty: "1".to_owned(),
                ask_price: "2".to_owned(),
                ask_qty: "1".to_owned(),
            },
            cached_at_ms: 1,
        },
    );
}
