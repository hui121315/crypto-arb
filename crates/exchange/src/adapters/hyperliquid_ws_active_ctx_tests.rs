use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_hl_schema() {
    let value: Value = serde_json::from_str(&subscribe_payload("BTC")).unwrap();
    assert_eq!(value["method"], "subscribe");
    assert_eq!(value["subscription"]["type"], "activeAssetCtx");
    assert_eq!(value["subscription"]["coin"], "BTC");
}

#[test]
fn unsubscribe_payload_matches_hl_schema() {
    let value: Value = serde_json::from_str(&unsubscribe_payload("ETH")).unwrap();
    assert_eq!(value["method"], "unsubscribe");
    assert_eq!(value["subscription"]["type"], "activeAssetCtx");
    assert_eq!(value["subscription"]["coin"], "ETH");
}

#[test]
fn parse_active_asset_ctx_extracts_coin_and_ctx() {
    let raw = r#"{
        "channel": "activeAssetCtx",
        "data": {
            "coin": "BTC",
            "ctx": {
                "funding": "0.0000125",
                "openInterest": "12345.67",
                "prevDayPx": "70000.0",
                "dayNtlVlm": "987654321.5",
                "premium": "0.00005",
                "oraclePx": "75500.0",
                "markPx": "75510.5",
                "midPx": "75505.0"
            }
        }
    }"#;
    let (coin, item) = parse_active_asset_ctx(raw).expect("ctx parses");
    assert_eq!(coin, "BTC");
    assert_eq!(item.funding, "0.0000125");
    assert_eq!(item.oracle_px, "75500.0");
    assert_eq!(item.open_interest, "12345.67");
    assert_eq!(item.day_ntl_vlm, "987654321.5");
}

#[test]
fn parse_active_spot_asset_ctx_accepts_spot_shape() {
    let raw = r#"{
        "channel": "activeAssetCtx",
        "data": {
            "coin": "PURR/USDC",
            "ctx": {
                "prevDayPx": "0.20",
                "dayNtlVlm": "123456.7",
                "markPx": "0.21",
                "midPx": "0.2095",
                "circulatingSupply": "598000000"
            }
        }
    }"#;

    let (coin, item) = parse_active_asset_ctx(raw).expect("spot ctx parses");

    assert_eq!(coin, "PURR/USDC");
    assert_eq!(item.mark_px, "0.21");
    assert_eq!(item.mid_px.as_deref(), Some("0.2095"));
    assert_eq!(item.day_ntl_vlm, "123456.7");
    assert!(item.funding.is_empty());
}

#[test]
fn parse_active_asset_ctx_ignores_other_channels() {
    let raw = r#"{"channel":"l2Book","data":{"coin":"BTC"}}"#;
    assert!(parse_active_asset_ctx(raw).is_none());
}

#[test]
fn parse_funding_combines_ctx_with_hl_one_hour_interval() {
    let item = AssetCtx {
        coin: None,
        funding: "0.0001".into(),
        mark_px: "75510.5".into(),
        oracle_px: "75500.0".into(),
        open_interest: "12345.67".into(),
        mid_px: Some("75505.0".into()),
        day_ntl_vlm: "987654321.5".into(),
        impact_pxs: Some(["75509.0".into(), "75512.0".into()]),
    };
    let funding = parse_funding("hyperliquid", "BTC", &item).expect("funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "hyperliquid");
    assert!((funding.rate - 0.0001).abs() < 1e-12);
    // HL interval = 1h → rate_8h = rate * 8.
    assert_eq!(funding.funding_interval, 1);
    assert!((funding.rate_8h - 0.0008).abs() < 1e-12);
    assert!((funding.volume_24h - 987_654_321.5).abs() < 1e-3);
    // next_funding_time is the next UTC top-of-hour.
    assert!(funding.next_funding_time > 0);
    assert_eq!(funding.next_funding_time % 3_600_000, 0);
}

#[test]
fn parse_funding_returns_none_when_rate_unparsable() {
    let item = AssetCtx {
        coin: None,
        funding: String::new(),
        mark_px: String::new(),
        oracle_px: String::new(),
        open_interest: String::new(),
        mid_px: None,
        day_ntl_vlm: String::new(),
        impact_pxs: None,
    };
    assert!(parse_funding("hyperliquid", "BTC", &item).is_none());
}

#[test]
fn parse_active_asset_ctx_picks_impact_pxs_when_present() {
    let raw = r#"{
        "channel": "activeAssetCtx",
        "data": {
            "coin": "ETH",
            "ctx": {
                "funding": "0.0001",
                "markPx": "3500.5",
                "midPx": "3500.0",
                "dayNtlVlm": "123456789",
                "impactPxs": ["3499.0", "3501.0"]
            }
        }
    }"#;
    let (coin, ctx) = parse_active_asset_ctx(raw).expect("ctx parses");
    assert_eq!(coin, "ETH");
    assert_eq!(ctx.impact_pxs.as_ref().unwrap()[0], "3499.0");
    assert_eq!(ctx.impact_pxs.as_ref().unwrap()[1], "3501.0");
}

#[test]
fn next_funding_time_rounds_up_to_top_of_hour() {
    // 03:30:15 UTC on some day → next top-of-hour is 04:00:00.
    let now = 1_700_000_000_000_i64;
    let computed = next_funding_time_ms(now);
    let now_floor = (now / 3_600_000) * 3_600_000;
    assert!(computed > now);
    assert_eq!(computed, now_floor + 3_600_000);
}

#[test]
fn transport_failures_invalidate_cached_asset_contexts() {
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

fn test_stream() -> ActiveAssetCtxStream {
    ActiveAssetCtxStream {
        venue: "hyperliquid",
        manager: Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: "hyperliquid".into(),
            heartbeat_interval: Duration::from_secs(30),
            heartbeat: WsHeartbeat::Text(json!({"method": "ping"}).to_string()),
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

fn seed_row(stream: &ActiveAssetCtxStream) {
    stream.on_text(
        r#"{"channel":"activeAssetCtx","data":{"coin":"BTC","ctx":{"funding":"0.0001","dayNtlVlm":"1"}}}"#,
    );
}
