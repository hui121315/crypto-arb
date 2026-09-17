use super::super::gate_ws_ticker_data::{
    channel_payload, order_book_payload, ORDER_BOOK_CHANNEL, TICKERS_CHANNEL,
};
use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_gate_ticker_schema() {
    let symbols = vec!["BTC".to_owned(), "ETH_USDT".to_owned()];
    let value: Value =
        serde_json::from_str(&channel_payload("subscribe", TICKERS_CHANNEL, &symbols)).unwrap();
    assert_eq!(value["channel"], TICKERS_CHANNEL);
    assert_eq!(value["event"], "subscribe");
    assert_eq!(value["payload"][0], "BTC_USDT");
    assert_eq!(value["payload"][1], "ETH_USDT");
}

#[test]
fn order_book_bootstrap_payload_requests_one_level() {
    let value: Value = serde_json::from_str(&order_book_payload("subscribe", "IONQ")).unwrap();
    assert_eq!(value["channel"], ORDER_BOOK_CHANNEL);
    assert_eq!(value["event"], "subscribe");
    assert_eq!(value["payload"][0], "IONQ_USDT");
    assert_eq!(value["payload"][1], "1");
    assert_eq!(value["payload"][2], "0");
}

#[test]
fn parse_market_update_extracts_last_and_volume() {
    let raw = r#"{
      "time": 1541659086,
      "channel": "futures.tickers",
      "event": "update",
      "result": [{
        "contract": "BTC_USDT",
        "last": "118.4",
        "funding_rate": "-0.000114",
        "funding_rate_indicative": "0.01875",
        "mark_price": "118.35",
        "index_price": "118.36",
        "total_size": "73648",
        "volume_24h_quote": "1665006",
        "volume_24h_settle": "178"
      }]
    }"#;
    let rows = parse_market_updates(raw);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item.contract, "BTC_USDT");
    assert_eq!(rows[0].item.last, "118.4");
    assert_eq!(rows[0].item.total_size, "73648");
    assert_eq!(rows[0].data_timestamp_ms, 1_541_659_086_000);
}

#[test]
fn parse_book_update_extracts_best_bid_and_ask() {
    let raw = r#"{
      "time": 1615366379,
      "channel": "futures.book_ticker",
      "event": "update",
      "result": {
        "t": 1615366379123,
        "u": 2517661076,
        "s": "BTC_USDT",
        "b": "54696.6",
        "B": 37000,
        "a": "54696.7",
        "A": 47061
      }
    }"#;
    let (symbol, row) = parse_book_update(raw).unwrap();
    assert_eq!(symbol, "BTC_USDT");
    assert_eq!(row.bid, 54696.6);
    assert_eq!(row.ask, 54696.7);
    assert_eq!(row.data_timestamp_ms, 1_615_366_379_123);
}

#[test]
fn to_ticker_combines_market_and_book_rows() {
    let market = CachedMarket {
        item: TickerItem {
            contract: "BTC_USDT".into(),
            last: "118.4".into(),
            highest_bid: String::new(),
            lowest_ask: String::new(),
            volume_24h_quote: "1665006".into(),
            volume_24h_settle: "178".into(),
            funding_rate: String::new(),
            funding_rate_indicative: String::new(),
            funding_next_apply: 0,
            mark_price: String::new(),
            index_price: String::new(),
            total_size: String::new(),
        },
        cached_at_ms: now_ms(),
        data_timestamp_ms: 1,
    };
    let book = CachedBookTicker {
        bid: 118.3,
        ask: 118.5,
        cached_at_ms: now_ms(),
        data_timestamp_ms: 2,
    };
    let ticker = to_ticker(&market, &book).expect("ws ticker parses");
    assert_eq!(ticker.symbol, "BTC");
    assert_eq!(ticker.bid, 118.3);
    assert_eq!(ticker.ask, 118.5);
    assert_eq!(ticker.last, 118.4);
    assert_eq!(ticker.timestamp, 2);
}

#[test]
fn parse_ignores_acks_and_other_channels() {
    assert!(
        parse_market_updates(r#"{"event":"subscribe","channel":"futures.tickers"}"#).is_empty()
    );
    assert!(parse_book_update(r#"{"channel":"futures.trades","event":"update"}"#).is_none());
}

#[test]
fn transport_failures_invalidate_cached_tickers() {
    let stream = test_stream();
    seed_rows(&stream);
    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1))));
    assert!(stream.markets.is_empty());
    assert!(stream.books.is_empty());

    seed_rows(&stream);
    stream.handle_ws_event(WsEvent::Disconnected("test".into()));
    assert!(stream.markets.is_empty());
    assert!(stream.books.is_empty());

    seed_rows(&stream);
    stream.handle_ws_event(WsEvent::CircuitOpened);
    assert!(stream.markets.is_empty());
    assert!(stream.books.is_empty());

    seed_rows(&stream);
    assert!(!stream.handle_dispatch_result(Err(RecvError::Closed)));
    assert!(stream.markets.is_empty());
    assert!(stream.books.is_empty());
}

fn test_stream() -> TickerStream {
    let manager = Arc::new(WsManager::new(WsConfig {
        url: WS_URL.into(),
        exchange: EXCHANGE.into(),
        heartbeat_interval: Duration::from_secs(20),
        heartbeat: WsHeartbeat::PingFrame,
        inbound_codec: WsInboundCodec::Plain,
        server_ping: WsServerPing::None,
        initial_reconnect_delay: Duration::from_secs(1),
        max_reconnect_delay: Duration::from_secs(30),
        circuit_breaker_threshold: 10,
    }));
    TickerStream {
        manager: Arc::clone(&manager),
        markets: Arc::new(DashMap::new()),
        books: Arc::new(DashMap::new()),
        runtime: GateTickerRuntime::new(manager),
    }
}

fn seed_rows(stream: &TickerStream) {
    stream.on_text(
        r#"{"time":1,"channel":"futures.tickers","event":"update","result":[{"contract":"BTC_USDT","last":"1"}]}"#,
    );
    stream.on_text(
        r#"{"channel":"futures.book_ticker","event":"update","result":{"t":1,"s":"BTC_USDT","b":"1","a":"2"}}"#,
    );
}
