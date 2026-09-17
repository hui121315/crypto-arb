use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_okx_funding_schema() {
    let symbols = vec!["BTC".to_owned(), "ETH-USDT-SWAP".to_owned()];
    let value: Value = serde_json::from_str(&channel_payload("subscribe", &symbols)).unwrap();
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0]["channel"], "funding-rate");
    assert_eq!(value["args"][0]["instId"], "BTC-USDT-SWAP");
    assert_eq!(value["args"][1]["instId"], "ETH-USDT-SWAP");
}

#[test]
fn unsubscribe_payload_matches_okx_funding_schema() {
    let symbols = vec!["BTC-USDT-SWAP".to_owned()];
    let value: Value = serde_json::from_str(&channel_payload("unsubscribe", &symbols)).unwrap();
    assert_eq!(value["op"], "unsubscribe");
    assert_eq!(value["args"][0]["channel"], "funding-rate");
    assert_eq!(value["args"][0]["instId"], "BTC-USDT-SWAP");
}

#[test]
fn parse_funding_rate_update_extracts_data() {
    let raw = r#"{
      "arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},
      "data":[{
        "formulaType":"noRate",
        "fundingRate":"0.0001875391284828",
        "fundingTime":"1700726400000",
        "instId":"BTC-USDT-SWAP",
        "instType":"SWAP",
        "method":"current_period",
        "nextFundingRate":"",
        "nextFundingTime":"1700755200000",
        "ts":"1700724675402"
      }]
    }"#;
    let parsed = parse_funding_update(raw).unwrap();
    let row = parse_funding(&parsed.item, 1_000_000.0).expect("funding parses");
    assert_eq!(parsed.stream_symbol, "BTC-USDT-SWAP");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "okx");
    assert!((row.rate - 0.000_187_539_128_482_8).abs() < 1e-16);
}

#[test]
fn parse_funding_rate_update_ignores_ack_and_other_channels() {
    assert!(
        parse_funding_update(r#"{"event":"subscribe","arg":{"channel":"funding-rate"}}"#).is_none()
    );
    let raw = r#"{"arg":{"channel":"mark-price","instId":"BTC-USDT-SWAP"},"data":[]}"#;
    assert!(parse_funding_update(raw).is_none());
}

#[test]
fn stream_symbol_normalizes_base_or_contract_symbol() {
    assert_eq!(stream_symbol("BTC"), "BTC-USDT-SWAP");
    assert_eq!(stream_symbol("BTC-USDT"), "BTC-USDT-SWAP");
    assert_eq!(stream_symbol("BTC-USDT-SWAP"), "BTC-USDT-SWAP");
}

#[test]
fn transport_failures_invalidate_cached_funding() {
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

#[test]
fn fresh_count_excludes_rows_without_complete_settlement_evidence() {
    let stream = test_stream();
    stream.on_text(
        r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","fundingRate":"0.0001","fundingTime":"1700726400000","nextFundingTime":"","ts":"1700724675402"}]}"#,
    );

    assert_eq!(stream.fresh_count(&["BTC".into()]), 0);

    stream.on_text(
        r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","fundingRate":"0.0001","fundingTime":"1700726400000","nextFundingTime":"1700755200000","ts":"1700724675402"}]}"#,
    );
    assert_eq!(stream.fresh_count(&["BTC".into()]), 1);
}

fn test_stream() -> FundingStream {
    FundingStream {
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

fn seed_row(stream: &FundingStream) {
    stream.on_text(
        r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[{"fundingRate":"0.0001","fundingTime":"1","nextFundingTime":"2","ts":"1"}]}"#,
    );
}
