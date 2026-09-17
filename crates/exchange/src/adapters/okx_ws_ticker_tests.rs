use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_okx_tickers_schema() {
    let symbols = vec!["BTC".to_owned(), "ETH-USDT-SWAP".to_owned()];
    let value: Value = serde_json::from_str(&channel_payload("subscribe", &symbols)).unwrap();
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0]["channel"], "tickers");
    assert_eq!(value["args"][0]["instId"], "BTC-USDT-SWAP");
    assert_eq!(value["args"][1]["instId"], "ETH-USDT-SWAP");
}

#[test]
fn unsubscribe_payload_matches_okx_tickers_schema() {
    let symbols = vec!["BTC-USDT-SWAP".to_owned()];
    let value: Value = serde_json::from_str(&channel_payload("unsubscribe", &symbols)).unwrap();
    assert_eq!(value["op"], "unsubscribe");
    assert_eq!(value["args"][0]["channel"], "tickers");
    assert_eq!(value["args"][0]["instId"], "BTC-USDT-SWAP");
}

#[test]
fn parse_ticker_update_extracts_data() {
    let raw = r#"{
      "arg":{"channel":"tickers","instId":"BTC-USDT-SWAP"},
      "data":[{
        "instType":"SWAP",
        "instId":"BTC-USDT-SWAP",
        "last":"75500.5",
        "lastSz":"0.01",
        "askPx":"75501.0",
        "askSz":"1.5",
        "bidPx":"75499.0",
        "bidSz":"2.0",
        "open24h":"74000",
        "high24h":"76000",
        "low24h":"73000",
        "volCcy24h":"3404219299.89529",
        "vol24h":"44545.8656",
        "ts":"1700724675402"
      }]
    }"#;
    let parsed = parse_ticker_update(raw).unwrap();
    let row = parse_ticker(&parsed.item).expect("ws ticker parses");
    assert_eq!(parsed.stream_symbol, "BTC-USDT-SWAP");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "okx");
    assert!((row.last - 75_500.5).abs() < 1e-6);
    assert!((row.bid - 75_499.0).abs() < 1e-6);
    assert!((row.ask - 75_501.0).abs() < 1e-6);
    assert!((row.volume_24h - 3_404_219_299.895_29).abs() < 1e-3);
    assert_eq!(row.timestamp, 1_700_724_675_402);
}

#[test]
fn parse_ticker_update_ignores_ack_and_other_channels() {
    assert!(parse_ticker_update(r#"{"event":"subscribe","arg":{"channel":"tickers"}}"#).is_none());
    let raw = r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[]}"#;
    assert!(parse_ticker_update(raw).is_none());
}

#[test]
fn stream_symbol_normalizes_base_or_contract_symbol() {
    assert_eq!(stream_symbol("BTC"), "BTC-USDT-SWAP");
    assert_eq!(stream_symbol("BTC-USDT"), "BTC-USDT-SWAP");
    assert_eq!(stream_symbol("BTC-USDT-SWAP"), "BTC-USDT-SWAP");
}

#[test]
fn transport_failures_invalidate_ticker_cache() {
    let stream = test_stream();
    seed_row(&stream);
    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1))));
    assert!(stream.rows.is_empty());

    seed_row(&stream);
    stream.handle_ws_event(WsEvent::Disconnected("test".to_owned()));
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
            heartbeat: WsHeartbeat::Text("ping".into()),
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
    stream.on_text(
        r#"{
      "arg":{"channel":"tickers","instId":"BTC-USDT-SWAP"},
      "data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP","last":"1",
      "askPx":"2","bidPx":"1","volCcy24h":"3","ts":"1"}]
    }"#,
    );
}
