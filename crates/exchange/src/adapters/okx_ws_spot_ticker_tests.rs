use super::*;
use crate::adapters::okx_market_data::parse_spot_tick;
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

#[test]
fn subscribe_payload_matches_okx_spot_tickers_schema() {
    let symbols = vec!["BTC-USDT".to_owned(), "ETH-USDC".to_owned()];
    let value: Value = serde_json::from_str(&channel_payload("subscribe", &symbols)).unwrap();
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0]["channel"], "tickers");
    assert_eq!(value["args"][0]["instType"], "SPOT");
    assert_eq!(value["args"][0]["instId"], "BTC-USDT");
    assert_eq!(value["args"][1]["instId"], "ETH-USDC");
}

#[test]
fn unsubscribe_payload_matches_okx_spot_tickers_schema() {
    let symbols = vec!["BTC-USDT".to_owned()];
    let value: Value = serde_json::from_str(&channel_payload("unsubscribe", &symbols)).unwrap();
    assert_eq!(value["op"], "unsubscribe");
    assert_eq!(value["args"][0]["channel"], "tickers");
    assert_eq!(value["args"][0]["instType"], "SPOT");
    assert_eq!(value["args"][0]["instId"], "BTC-USDT");
}

#[test]
fn spot_inst_ids_normalize_pair_inputs() {
    let symbols = vec![
        "BTC".to_owned(),
        "eth/usdt".to_owned(),
        "SOL-USDC".to_owned(),
    ];
    assert_eq!(
        spot_inst_ids(&symbols),
        Some(vec![
            "BTC-USDT".to_owned(),
            "ETH-USDT".to_owned(),
            "SOL-USDC".to_owned()
        ])
    );
}

#[test]
fn parse_ticker_update_extracts_spot_payload() {
    let raw = r#"{
      "arg":{"channel":"tickers","instId":"BTC-USDT"},
      "data":[{
        "instType":"SPOT",
        "instId":"BTC-USDT",
        "last":"9999.99",
        "lastSz":"0.1",
        "askPx":"10000.01",
        "askSz":"11",
        "bidPx":"9999.98",
        "bidSz":"5",
        "open24h":"9000",
        "high24h":"10000",
        "low24h":"8888.88",
        "volCcy24h":"2222",
        "vol24h":"0.2222",
        "ts":"1597026383085"
      }]
    }"#;
    let parsed = parse_ticker_update(raw).expect("ticker parses");
    let row = parse_spot_tick(&parsed.item).expect("spot tick");

    assert_eq!(parsed.stream_symbol, "BTC-USDT");
    assert_eq!(row.venue, "okx");
    assert_eq!(row.symbol, "BTC/USDT");
    assert_eq!(row.bid, dec("9999.98"));
    assert_eq!(row.ask, dec("10000.01"));
    assert_eq!(row.bid_size, Some(dec("5")));
    assert_eq!(row.ask_size, Some(dec("11")));
    assert_eq!(row.volume_24h, dec("2222"));
    assert_eq!(row.exchange_ts_ms, Some(1_597_026_383_085));
    assert!(row.received_at_ms > 0);
}

#[test]
fn parse_ticker_update_ignores_ack_perp_and_other_channels() {
    assert!(parse_ticker_update(r#"{"event":"subscribe","arg":{"channel":"tickers"}}"#).is_none());
    let swap = r#"{
      "arg":{"channel":"tickers","instId":"BTC-USDT-SWAP"},
      "data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP"}]
    }"#;
    assert!(parse_ticker_update(swap).is_none());
    let funding = r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT"},"data":[]}"#;
    assert!(parse_ticker_update(funding).is_none());
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("valid decimal")
}

#[test]
fn transport_failures_invalidate_spot_ticker_cache() {
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

fn test_stream() -> SpotTickerStream {
    SpotTickerStream {
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

fn seed_row(stream: &SpotTickerStream) {
    stream.on_text(
        r#"{
      "arg":{"channel":"tickers","instId":"BTC-USDT"},
      "data":[{"instType":"SPOT","instId":"BTC-USDT","last":"1",
      "askPx":"2","askSz":"1","bidPx":"1","bidSz":"1",
      "volCcy24h":"3","ts":"1"}]
    }"#,
    );
}
