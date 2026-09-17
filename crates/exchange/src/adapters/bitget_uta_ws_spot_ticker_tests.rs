use super::*;
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

#[test]
fn channel_payload_uses_v3_spot_args_shape() {
    let payload = channel_payload("subscribe", &["BTCUSDT".to_owned(), "ETHUSDC".to_owned()]);
    let value: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["op"], "subscribe");
    let args = value["args"].as_array().expect("args is an array");
    assert_eq!(args.len(), 2);
    for arg in args {
        assert_eq!(arg["instType"], "spot");
        assert_eq!(arg["topic"], "ticker");
        assert!(matches!(
            arg["symbol"].as_str(),
            Some("BTCUSDT" | "ETHUSDC")
        ));
    }
}

#[test]
fn stream_symbols_normalize_pair_inputs() {
    let rows = stream_symbols(&[
        "BTC".to_owned(),
        "eth/usdt".to_owned(),
        "sol-usdc".to_owned(),
    ])
    .expect("symbols normalize");
    assert_eq!(rows, vec!["BTCUSDT", "ETHUSDT", "SOLUSDC"]);
    assert!(stream_symbols(&[]).is_none());
    assert!(stream_symbols(&["".to_owned()]).is_none());
}

#[test]
fn parse_ticker_update_extracts_official_spot_payload() {
    let raw = r#"{
      "data": [
        {
          "bid1Price": "99999",
          "lowPrice24h": "98200",
          "ask1Size": "188.312553",
          "volume24h": "37.722858",
          "price24hPcnt": "0.01833",
          "highPrice24h": "100000",
          "turnover24h": "3750302.979626",
          "bid1Size": "186.183209",
          "ask1Price": "100000",
          "openPrice24h": "0",
          "lastPrice": "100000"
        }
      ],
      "arg": {
        "instType": "spot",
        "symbol": "BTCUSDT",
        "topic": "ticker"
      },
      "action": "snapshot",
      "ts": 1736371332162
    }"#;
    let rows = parse_ticker_update(raw);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item.symbol, "BTCUSDT");
    assert_eq!(rows[0].item.bid1_price, "99999");
    assert_eq!(rows[0].item.ask1_price, "100000");
    assert_eq!(rows[0].item.bid1_size, "186.183209");
    assert_eq!(rows[0].item.ask1_size, "188.312553");
    assert_eq!(rows[0].data_timestamp_ms, 1_736_371_332_162);
}

#[test]
fn parse_spot_tick_uses_bid_ask_sizes_and_turnover() {
    let row = CachedTicker {
        item: SpotTickerItem {
            symbol: "BTCUSDT".into(),
            last_price: "100000".into(),
            bid1_price: "99999".into(),
            ask1_price: "100000".into(),
            bid1_size: "186.183209".into(),
            ask1_size: "188.312553".into(),
            volume_24h: "37.722858".into(),
            turnover_24h: "3750302.979626".into(),
            ts: String::new(),
        },
        cached_at_ms: now_ms(),
        data_timestamp_ms: 1_736_371_332_162,
    };
    let tick = parse_spot_tick(&row).expect("spot tick");
    assert_eq!(tick.venue, "bitget");
    assert_eq!(tick.symbol, "BTC/USDT");
    assert_eq!(tick.bid, dec("99999"));
    assert_eq!(tick.ask, dec("100000"));
    assert_eq!(tick.last, dec("100000"));
    assert_eq!(tick.bid_size, Some(dec("186.183209")));
    assert_eq!(tick.ask_size, Some(dec("188.312553")));
    assert_eq!(tick.volume_24h, dec("3750302.979626"));
    assert_eq!(tick.exchange_ts_ms, Some(1_736_371_332_162));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn parse_ticker_update_ignores_non_spot_topics() {
    assert!(parse_ticker_update(
        r#"{"event":"subscribe","arg":{"instType":"spot","topic":"ticker","symbol":"BTCUSDT"}}"#
    )
    .is_empty());
    assert!(parse_ticker_update(r#"{"arg":{"instType":"usdt-futures","topic":"ticker","symbol":"BTCUSDT"},"data":[{"lastPrice":"1"}]}"#).is_empty());
    assert!(parse_ticker_update(r#"{"arg":{"instType":"spot","topic":"books","symbol":"BTCUSDT"},"data":[{"lastPrice":"1"}]}"#).is_empty());
}

#[test]
fn quote_volume_falls_back_to_base_when_turnover_missing() {
    let item = SpotTickerItem {
        symbol: "ETHUSDT".into(),
        last_price: "1".into(),
        bid1_price: "1".into(),
        ask1_price: "1".into(),
        bid1_size: "1".into(),
        ask1_size: "1".into(),
        volume_24h: "42".into(),
        turnover_24h: String::new(),
        ts: String::new(),
    };
    assert_eq!(quote_volume(&item), "42");
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("valid decimal")
}

#[test]
fn lag_keeps_complete_snapshots_while_transport_failures_clear_cache() {
    let stream = test_stream();
    seed_row(&stream);
    assert!(stream.handle_dispatch_result(Err(RecvError::Lagged(1))));
    assert_eq!(stream.rows.len(), 1);
    stream.handle_ws_event(WsEvent::Disconnected("test".to_owned()));
    assert!(stream.rows.is_empty());
    seed_row(&stream);
    stream.handle_ws_event(WsEvent::CircuitOpened);
    assert!(stream.rows.is_empty());
    seed_row(&stream);
    assert!(!stream.handle_dispatch_result(Err(RecvError::Closed)));
    assert!(stream.rows.is_empty());
}

#[test]
fn orphan_frames_after_prune_do_not_repopulate_the_cache() {
    let stream = test_stream();
    ingest_seed_row(&stream);
    assert!(stream.rows.is_empty());
}

fn test_stream() -> SpotTickerStream {
    SpotTickerStream {
        manager: Arc::new(WsManager::new(WsConfig {
            url: PROD_WS_PUBLIC.into(),
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
    stream.subscriptions.insert(
        "BTCUSDT".to_owned(),
        SubscriptionState {
            last_touched_ms: now_ms(),
            sent_on_current_connection: true,
        },
    );
    ingest_seed_row(stream);
}

fn ingest_seed_row(stream: &SpotTickerStream) {
    stream.on_text(
        r#"{
      "data":[{"bid1Price":"1","ask1Size":"1","volume24h":"3",
      "turnover24h":"3","bid1Size":"1","ask1Price":"2","lastPrice":"1"}],
      "arg":{"instType":"spot","symbol":"BTCUSDT","topic":"ticker"},
      "action":"snapshot","ts":1
    }"#,
    );
}
