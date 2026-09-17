use super::*;
use crate::adapters::bybit_market_data::MarketTickerItem;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn failed_subscribe_releases_the_symbol_for_one_bounded_retry() {
    let subscriptions = DashMap::new();
    let symbol = "BTCUSDT".to_owned();
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
fn subscribe_payload_matches_bybit_ticker_schema() {
    let symbols = vec![
        "BTC".to_owned(),
        "ETHUSDT".to_owned(),
        "SOLUSDC".to_owned(),
        "BTCPERP".to_owned(),
    ];
    let value: Value = serde_json::from_str(&channel_payload("subscribe", &symbols)).unwrap();
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0], "tickers.BTCUSDT");
    assert_eq!(value["args"][1], "tickers.ETHUSDT");
    assert_eq!(value["args"][2], "tickers.SOLUSDC");
    assert_eq!(value["args"][3], "tickers.BTCPERP");
}

#[test]
fn parse_snapshot_extracts_ticker_and_funding() {
    let raw = r#"{
      "topic":"tickers.BTCUSDT",
      "type":"snapshot",
      "ts":1760340000000,
      "data":{
        "symbol":"BTCUSDT",
        "lastPrice":"66666.60",
        "markPrice":"66666.60",
        "indexPrice":"66500.00",
        "openInterest":"492373.72",
        "openInterestValue":"32824881841.75",
        "turnover24h":"4936790807.6521",
        "volume24h":"73191.3870",
        "fundingIntervalHour":"8",
        "nextFundingTime":"1760342400000",
        "fundingRate":"-0.005",
        "bid1Price":"66666.60",
        "bid1Size":"23789.165",
        "ask1Price":"66666.70",
        "ask1Size":"23775.469"
      }
    }"#;
    let parsed = parse_ticker_update(raw).unwrap();
    let cached = CachedTicker {
        item: parsed.item,
        cached_at_ms: now_ms(),
        data_timestamp_ms: parsed.timestamp_ms,
    };

    let ticker = to_ticker(&cached).expect("ws ticker parses");
    let funding = to_funding(&cached).expect("complete funding evidence parses");
    assert_eq!(parsed.stream_symbol, "BTCUSDT");
    assert_eq!(ticker.symbol, "BTC");
    assert_eq!(ticker.bid, 66666.60);
    assert_eq!(ticker.ask, 66666.70);
    assert_eq!(ticker.timestamp, 1_760_340_000_000);
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.funding_interval, 8);
    assert_eq!(funding.next_funding_time, 1_760_342_400_000);
    assert!((funding.rate + 0.005).abs() < 1e-12);
}

#[test]
fn funding_row_requires_interval_and_next_settlement_evidence() {
    let cached = CachedTicker {
        item: MarketTickerItem {
            symbol: "BTCUSDT".into(),
            funding_rate: "0.0001".into(),
            ..MarketTickerItem::default()
        },
        cached_at_ms: now_ms(),
        data_timestamp_ms: now_ms(),
    };

    assert!(to_funding(&cached).is_none());
}

#[test]
fn delta_merges_only_present_fields() {
    let mut base = MarketTickerItem {
        symbol: "BTCUSDT".into(),
        last_price: "100".into(),
        bid1_price: "99".into(),
        bid1_size: "1".into(),
        ask1_price: "101".into(),
        ask1_size: "2".into(),
        turnover24h: "1000".into(),
        funding_rate: "0.0001".into(),
        next_funding_time: "1700000000000".into(),
        funding_interval_hour: "8".into(),
        mark_price: "100".into(),
        index_price: "99".into(),
        open_interest: "10".into(),
        open_interest_value: "1000".into(),
    };
    let delta = MarketTickerItem {
        symbol: "BTCUSDT".into(),
        last_price: "102".into(),
        bid1_price: String::new(),
        bid1_size: String::new(),
        ask1_price: "103".into(),
        ask1_size: String::new(),
        turnover24h: String::new(),
        funding_rate: String::new(),
        next_funding_time: String::new(),
        funding_interval_hour: String::new(),
        mark_price: String::new(),
        index_price: String::new(),
        open_interest: String::new(),
        open_interest_value: String::new(),
    };

    merge_item(&mut base, delta);
    assert_eq!(base.last_price, "102");
    assert_eq!(base.bid1_price, "99");
    assert_eq!(base.ask1_price, "103");
    assert_eq!(base.funding_rate, "0.0001");
}

#[test]
fn parse_ignores_acks_and_other_topics() {
    assert!(parse_ticker_update(r#"{"op":"subscribe","success":true}"#).is_none());
    let raw = r#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","data":{}}"#;
    assert!(parse_ticker_update(raw).is_none());
}

#[test]
fn transport_failures_invalidate_cached_tickers() {
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

fn test_stream() -> TickerStream {
    TickerStream {
        manager: Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: EXCHANGE.into(),
            heartbeat_interval: Duration::from_secs(20),
            heartbeat: WsHeartbeat::Text(r#"{"op":"ping"}"#.into()),
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        })),
        rows: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
    }
}

fn seed_row(stream: &TickerStream) {
    let parsed = parse_ticker_update(
        r#"{"topic":"tickers.BTCUSDT","type":"snapshot","ts":1,"data":{"symbol":"BTCUSDT"}}"#,
    )
    .expect("ticker frame parses");
    stream.rows.insert(
        parsed.stream_symbol,
        CachedTicker {
            item: parsed.item,
            cached_at_ms: now_ms(),
            data_timestamp_ms: parsed.timestamp_ms,
        },
    );
}
