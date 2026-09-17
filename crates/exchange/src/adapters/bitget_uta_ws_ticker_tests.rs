use super::super::bitget_uta_ws_ticker_data::{
    parse_funding, parse_quote_volume, CachedTicker, TickerItem,
};
use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

fn cached(item: TickerItem) -> CachedTicker {
    CachedTicker {
        symbol: "BTCUSDT".into(),
        item,
        ts_ms: 1_785_088_069_374,
        cached_at_ms: 0,
    }
}

#[test]
fn channel_payload_uses_v3_args_shape() {
    let payload = channel_payload("subscribe", &["BTCUSDT".to_owned(), "ETHUSDT".to_owned()]);
    let value: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["op"], "subscribe");
    let args = value["args"].as_array().expect("args is an array");
    assert_eq!(args.len(), 2);
    for arg in args {
        assert_eq!(arg["instType"], "usdt-futures");
        assert_eq!(arg["topic"], "ticker");
        assert!(matches!(
            arg["symbol"].as_str(),
            Some("BTCUSDT" | "ETHUSDT")
        ));
    }
}

#[test]
fn parse_ticker_update_takes_symbol_and_ts_from_envelope() {
    // 2026-07-27 生产实抓帧：`data[]` 项不带 symbol/ts，信封 `ts` 是数字。
    // 旧实现从 `data[].symbol`（serde default 空串）取缓存键，导致缓存永远
    // 查不到、bitget ticker 长期 0 覆盖回落 REST。
    let raw = r#"{
        "action":"snapshot",
        "arg":{"instType":"usdt-futures","topic":"ticker","symbol":"BTCUSDT"},
        "data":[{
            "highPrice24h":"64915.3","lowPrice24h":"64216.3","openPrice24h":"64274.5",
            "lastPrice":"64652","turnover24h":"924214758.208","volume24h":"14324.8359",
            "bid1Price":"64652","ask1Price":"64652.1","bid1Size":"3.4742","ask1Size":"2.3549",
            "price24hPcnt":"0.00587","indexPrice":"64687.8815","markPrice":"64655.5",
            "fundingRate":"-0.000023","openInterest":"36201.7054","deliveryTime":"",
            "deliveryStartTime":"","deliveryStatus":"","nextFundingTime":"1785110400000"
        }],
        "ts":1785088069374
    }"#;
    let rows = parse_ticker_update(raw);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "BTCUSDT");
    assert_eq!(rows[0].ts_ms, 1_785_088_069_374);

    let ticker = parse_ticker(&rows[0]).expect("ws ticker parses");
    assert_eq!(ticker.symbol, "BTC");
    assert_eq!(ticker.exchange, "bitget");
    assert!((ticker.bid - 64_652.0).abs() < 1e-6);
    assert!((ticker.ask - 64_652.1).abs() < 1e-6);
    assert!((ticker.last - 64_652.0).abs() < 1e-6);
    // turnover24h (quote) wins over base volume24h.
    assert!((ticker.volume_24h - 924_214_758.208).abs() < 1e-3);
    assert_eq!(ticker.timestamp, 1_785_088_069_374);
}

#[test]
fn parse_mark_index_extracts_mark_index_and_open_interest() {
    let row = cached(TickerItem {
        last_price: "75492.1".into(),
        bid1_price: "75492.0".into(),
        ask1_price: "75492.2".into(),
        volume_24h: "44545.8656".into(),
        turnover_24h: "3404219299.89529".into(),
        mark_price: "75492.1".into(),
        index_price: "75525.13".into(),
        open_interest: "31601.0065".into(),
        funding_rate: "-0.000023".into(),
        next_funding_time: "1785110400000".into(),
    });
    let info = parse_mark_index(&row).expect("mark/index row parses");
    assert_eq!(info.symbol, "BTC");
    assert_eq!(info.exchange, "bitget");
    assert!((info.mark_price - 75_492.1).abs() < 1e-6);
    assert_eq!(info.index_price, Some(75_525.13));
    assert_eq!(info.open_interest, Some(31_601.006_5));
    assert_eq!(info.open_interest_value, None);
    assert_eq!(info.timestamp, 1_785_088_069_374);
}

#[test]
fn parse_ticker_update_ignores_non_ticker_topics() {
    // Books frames must not feed the ticker cache.
    let raw = r#"{"arg":{"topic":"books","symbol":"BTCUSDT"},"data":[{"a":[],"b":[]}]}"#;
    assert!(parse_ticker_update(raw).is_empty());
    // Subscribe ack carries no `data`.
    let ack = r#"{"event":"subscribe","arg":{"instType":"usdt-futures","topic":"ticker","symbol":"BTCUSDT"}}"#;
    assert!(parse_ticker_update(ack).is_empty());
    // Frames without an arg symbol cannot be keyed and must be dropped.
    let no_symbol = r#"{"arg":{"topic":"ticker"},"data":[{"lastPrice":"1"}],"ts":1}"#;
    assert!(parse_ticker_update(no_symbol).is_empty());
}

#[test]
fn parse_quote_volume_falls_back_to_base() {
    let row = TickerItem {
        last_price: "0".into(),
        bid1_price: "0".into(),
        ask1_price: "0".into(),
        volume_24h: "100.0".into(),
        turnover_24h: String::new(),
        mark_price: String::new(),
        index_price: String::new(),
        open_interest: String::new(),
        funding_rate: String::new(),
        next_funding_time: String::new(),
    };
    assert!((parse_quote_volume(&row) - 100.0).abs() < 1e-9);
}

#[test]
fn parse_funding_requires_next_settlement_evidence() {
    let row = cached(TickerItem {
        funding_rate: "0.0001".into(),
        next_funding_time: String::new(),
        ..TickerItem::default()
    });

    assert!(parse_funding(&row, 8).is_none());
}

#[test]
fn parse_ticker_drops_tick_when_bid_missing() {
    let envelope = r#"{"action":"snapshot","arg":{"instType":"usdt-futures","topic":"ticker","symbol":"BTCUSDT"},"data":[{"lastPrice":"75492.1","ask1Price":"75492.2"}],"ts":1779511245529}"#;
    let rows = parse_ticker_update(envelope);
    assert_eq!(rows.len(), 1);
    assert!(parse_ticker(&rows[0]).is_none());
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
        intervals: Arc::new(DashMap::new()),
    }
}

fn seed_row(stream: &TickerStream) {
    let mut rows = parse_ticker_update(
        r#"{"arg":{"instType":"usdt-futures","topic":"ticker","symbol":"BTCUSDT"},"data":[{"lastPrice":"1"}],"ts":1}"#,
    );
    let row = rows.pop().expect("ticker frame parses");
    stream.rows.insert(row.symbol.clone(), row);
}
