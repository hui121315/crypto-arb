use super::*;
use pretty_assertions::assert_eq;

#[test]
fn subscribe_payload_contains_three_official_channels() {
    let payload = channel_payload("subscribe", &["BTC".to_owned()]);
    let value: Value = serde_json::from_str(&payload).expect("payload parses");
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0]["channel"], CHANNEL_MARK);
    assert_eq!(value["args"][0]["instId"], "BTC-USDT-SWAP");
    assert_eq!(value["args"][1]["channel"], CHANNEL_OI);
    assert_eq!(value["args"][1]["instId"], "BTC-USDT-SWAP");
    assert_eq!(value["args"][2]["channel"], CHANNEL_INDEX);
    assert_eq!(value["args"][2]["instId"], "BTC-USDT");
}

#[test]
fn parse_mark_price_update_uses_mark_px_and_ts() {
    let raw = r#"{"arg":{"channel":"mark-price","instId":"BTC-USDT-SWAP"},"data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP","markPx":"200","ts":"1597026383085"}]}"#;
    let parsed = parse_update(raw).expect("mark update parses");
    assert_eq!(parsed.stream_symbol, "BTC-USDT-SWAP");
    match parsed.kind {
        UpdateKind::Mark(value) => {
            assert_eq!(value.value, 200.0);
            assert_eq!(value.timestamp_ms, 1_597_026_383_085);
        }
        _ => panic!("expected mark update"),
    }
}

#[test]
fn parse_index_update_maps_index_id_to_swap_id() {
    let raw = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"199.5","ts":"1597026383085"}]}"#;
    let parsed = parse_update(raw).expect("index update parses");
    assert_eq!(parsed.stream_symbol, "BTC-USDT-SWAP");
    match parsed.kind {
        UpdateKind::Index(value) => assert_eq!(value, 199.5),
        _ => panic!("expected index update"),
    }
}

#[test]
fn parse_open_interest_update_uses_contracts_and_usd_value() {
    let raw = r#"{"arg":{"channel":"open-interest","instId":"BTC-USDT-SWAP"},"data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP","oi":"5000","oiCcy":"555.55","oiUsd":"50000","ts":"1743041250440"}]}"#;
    let parsed = parse_update(raw).expect("oi update parses");
    match parsed.kind {
        UpdateKind::OpenInterest { oi, oi_usd } => {
            assert_eq!(oi, Some(5000.0));
            assert_eq!(oi_usd, Some(50_000.0));
        }
        _ => panic!("expected open interest update"),
    }
}

#[test]
fn transport_failures_invalidate_mark_index_cache() {
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

fn test_stream() -> MarkIndexStream {
    MarkIndexStream {
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

fn seed_row(stream: &MarkIndexStream) {
    stream.on_text(
        r#"{"arg":{"channel":"mark-price","instId":"BTC-USDT-SWAP"},"data":[{"markPx":"200","ts":"1"}]}"#,
    );
}
