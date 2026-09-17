use super::*;
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

#[test]
fn failed_subscribe_releases_ticker_or_depth_for_one_bounded_retry() {
    let subscriptions = DashMap::new();
    let symbol = "BTC_USDT".to_owned();
    subscriptions.insert(
        symbol.clone(),
        SubscriptionState {
            last_touched_ms: 1,
            sent_on_current_connection: true,
        },
    );

    release_subscriptions(&subscriptions, std::slice::from_ref(&symbol), "subscribe");
    let mut state = subscriptions.get_mut(&symbol).expect("subscription exists");
    assert!(claim_subscription(&mut state));
    assert!(!claim_subscription(&mut state));
}

#[test]
fn subscribe_payload_matches_gate_spot_schema() {
    let symbols = vec!["BTC_USDT".to_owned(), "ETH_USDC".to_owned()];
    let value: Value =
        serde_json::from_str(&channel_payload("subscribe", TICKERS_CHANNEL, &symbols)).unwrap();
    assert_eq!(value["channel"], TICKERS_CHANNEL);
    assert_eq!(value["event"], "subscribe");
    assert_eq!(value["payload"][0], "BTC_USDT");
    assert_eq!(value["payload"][1], "ETH_USDC");
}

#[test]
fn depth_payload_requests_fifty_level_snapshots() {
    let value: Value =
        serde_json::from_str(&depth_channel_payload("subscribe", "BTC_USDT")).unwrap();
    assert_eq!(value["channel"], ORDER_BOOK_CHANNEL);
    assert_eq!(
        value["payload"],
        serde_json::json!(["BTC_USDT", "50", "100ms"])
    );
}

#[test]
fn stream_symbol_normalizes_pair_inputs() {
    assert_eq!(stream_symbol("BTC"), Some("BTC_USDT".to_owned()));
    assert_eq!(stream_symbol("btc/usdt"), Some("BTC_USDT".to_owned()));
    assert_eq!(stream_symbol("eth-usdc"), Some("ETH_USDC".to_owned()));
    assert_eq!(stream_symbol(""), None);
}

#[test]
fn parse_ticker_update_extracts_last_and_quote_volume() {
    let raw = r#"{
      "time": 1669107766,
      "time_ms": 1669107766406,
      "channel": "spot.tickers",
      "event": "update",
      "result": {
        "currency_pair": "BTC_USDT",
        "last": "15743.4",
        "lowest_ask": "15744.4",
        "highest_bid": "15743.5",
        "quote_volume": "145082083.2535"
      }
    }"#;
    let row = parse_ticker_update(raw).expect("ticker parses");
    assert_eq!(row.item.currency_pair, "BTC_USDT");
    assert_eq!(row.item.last_price, "15743.4");
    assert_eq!(row.item.lowest_ask, "15744.4");
    assert_eq!(row.item.highest_bid, "15743.5");
    assert_eq!(row.item.quote_volume, "145082083.2535");
    assert_eq!(row.data_timestamp_ms, 1_669_107_766_406);
}

#[test]
fn parse_depth_update_preserves_multiple_levels() {
    let raw = r#"{
      "time_ms":1606295412213,
      "channel":"spot.order_book",
      "event":"update",
      "result":{"t":1606295412123,"s":"BTC_USDT",
        "bids":[["10","2"],["9","3"]],
        "asks":[["11","4"],["12","5"]]}
    }"#;
    let (symbol, cached) = parse_depth_update(raw).expect("depth parses");
    assert_eq!(symbol, "BTC_USDT");
    assert_eq!(cached.book.symbol, "BTC/USDT");
    assert_eq!(cached.book.bids, vec![[10.0, 2.0], [9.0, 3.0]]);
    assert_eq!(cached.book.asks, vec![[11.0, 4.0], [12.0, 5.0]]);
}

#[test]
fn parse_spot_tick_uses_ticker_bbo_without_claiming_sizes() {
    let ticker = CachedTicker {
        item: SpotTickerUpdate {
            currency_pair: "BTC_USDT".into(),
            last_price: "15743.4".into(),
            lowest_ask: "15744.4".into(),
            highest_bid: "15743.5".into(),
            quote_volume: "145082083.2535".into(),
        },
        cached_at_ms: now_ms(),
        data_timestamp_ms: 1_669_107_766_406,
    };
    let row = parse_spot_tick(&ticker).expect("spot tick");

    assert_eq!(row.venue, "gate");
    assert_eq!(row.symbol, "BTC/USDT");
    assert_eq!(row.bid, dec("15743.5"));
    assert_eq!(row.ask, dec("15744.4"));
    assert_eq!(row.last, dec("15743.4"));
    assert_eq!(row.bid_size, None);
    assert_eq!(row.ask_size, None);
    assert_eq!(row.volume_24h, dec("145082083.2535"));
    assert_eq!(row.exchange_ts_ms, Some(1_669_107_766_406));
    assert!(row.received_at_ms > 0);
}

#[test]
fn parse_ignores_acks_and_other_channels() {
    assert!(parse_ticker_update(r#"{"event":"subscribe","channel":"spot.tickers"}"#).is_none());
    assert!(parse_ticker_update(r#"{"channel":"spot.trades","event":"update"}"#).is_none());
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("valid decimal")
}

#[test]
fn transport_failures_invalidate_all_spot_market_caches() {
    let stream = test_stream();
    seed_rows(&stream);
    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1))));
    assert_cache_empty(&stream);
    seed_rows(&stream);
    stream.handle_ws_event(WsEvent::Disconnected("test".to_owned()));
    assert_cache_empty(&stream);
    seed_rows(&stream);
    stream.handle_ws_event(WsEvent::CircuitOpened);
    assert_cache_empty(&stream);
    seed_rows(&stream);
    assert!(!stream.handle_dispatch_result(Err(RecvError::Closed)));
    assert_cache_empty(&stream);
}

fn test_stream() -> SpotTickerStream {
    SpotTickerStream {
        manager: Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: EXCHANGE.into(),
            heartbeat_interval: Duration::from_secs(20),
            heartbeat: WsHeartbeat::PingFrame,
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        })),
        tickers: Arc::new(DashMap::new()),
        depth_books: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
        depth_subscriptions: Arc::new(DashMap::new()),
    }
}

fn seed_rows(stream: &SpotTickerStream) {
    stream.on_text(r#"{"time_ms":1,"channel":"spot.tickers","event":"update","result":{"currency_pair":"BTC_USDT","last":"1","lowest_ask":"2","highest_bid":"1","quote_volume":"3"}}"#);
    stream.on_text(r#"{"time_ms":1,"channel":"spot.order_book","event":"update","result":{"t":1,"s":"BTC_USDT","bids":[["1","1"]],"asks":[["2","1"]]}}"#);
}

fn assert_cache_empty(stream: &SpotTickerStream) {
    assert!(stream.tickers.is_empty());
    assert!(stream.depth_books.is_empty());
}
