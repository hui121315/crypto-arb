use super::*;
use crate::adapters::bybit_market_data::MarketTickerItem;
use crate::adapters::bybit_response::BybitResponse;
use pretty_assertions::assert_eq;
use std::sync::atomic::Ordering;

fn bybit() -> Bybit {
    Bybit::new(BybitConfig::default()).unwrap()
}

#[test]
fn name() {
    assert_eq!(ExchangeAdapter::name(&bybit()), "bybit");
}

#[test]
fn symbol_round_trip() {
    let b = bybit();
    assert_eq!(b.to_exchange_symbol("BTC"), "BTCUSDT");
    assert_eq!(b.to_exchange_symbol("BTCUSDC"), "BTCUSDC");
    assert_eq!(b.to_exchange_symbol("BTC-USDC-SWAP"), "BTCUSDC");
    assert_eq!(b.to_exchange_symbol("BTCPERP"), "BTCPERP");
    assert_eq!(b.to_exchange_symbol("AAPLUSDT"), "AAPLUSDT");
    assert_eq!(b.normalize_symbol("BTCUSDT"), "BTC");
}

#[test]
fn account_type_string_mapping() {
    // 修复 P1 3.1：4 种 accountType 都正确映射。
    assert_eq!(BybitAccountType::Unified.as_str(), "UNIFIED");
    assert_eq!(BybitAccountType::Contract.as_str(), "CONTRACT");
    assert_eq!(BybitAccountType::Spot.as_str(), "SPOT");
    assert_eq!(BybitAccountType::Fund.as_str(), "FUND");
}

#[test]
fn funding_interval_minutes_to_hours_handles_common_cases() {
    // 修复 P1 3.2：常见值正确。
    assert_eq!(funding_interval_minutes_to_hours(60), 1); // 1h
    assert_eq!(funding_interval_minutes_to_hours(240), 4); // 4h
    assert_eq!(funding_interval_minutes_to_hours(480), 8); // 8h
}

#[test]
fn funding_interval_minutes_to_hours_rounds_non_60_multiples() {
    // 30 / 60 = 0.5 → round = 1
    assert_eq!(funding_interval_minutes_to_hours(30), 1);
    // 90 / 60 = 1.5 → round = 2
    assert_eq!(funding_interval_minutes_to_hours(90), 2);
    // 150 / 60 = 2.5 → round = 3 (banker rounding 在 .5 时取偶数；f64 round 是 half-away)
    // 实际 (2.5).round() = 3.0
    assert_eq!(funding_interval_minutes_to_hours(150), 3);
}

#[test]
fn funding_interval_minutes_to_hours_handles_negative_and_zero() {
    // 修复 P1 3.2：负数 / 0 / 缺失字段都退回默认 480 → 8h，而非异常值。
    assert_eq!(funding_interval_minutes_to_hours(0), 8);
    assert_eq!(funding_interval_minutes_to_hours(-1), 8);
    assert_eq!(funding_interval_minutes_to_hours(-9999), 8);
}

#[test]
fn funding_interval_minutes_to_hours_clamps_to_24h() {
    // 1500min / 60 = 25 → clamp 24
    assert_eq!(funding_interval_minutes_to_hours(1500), 24);
}

#[test]
fn is_usdm_perp_accepts_usdt_and_usdc() {
    // 修复 P1 3.5：USDC pair 不应被丢弃。
    assert!(is_usdm_perp("BTCUSDT"));
    assert!(is_usdm_perp("ETHUSDC"));
    assert!(is_usdm_perp("BTCPERP"));
    assert!(!is_usdm_perp("BTCBUSD"));
    assert!(!is_usdm_perp("BTC"));
}

#[test]
fn discovery_defaults_to_usdt_but_exact_requests_keep_usdc_perps() {
    assert!(include_discovery_perp("BTCUSDT", None));
    assert!(!include_discovery_perp("BTCUSDC", None));
    assert!(!include_discovery_perp("BTCPERP", None));
    let requested = std::collections::HashSet::from(["BTCPERP".to_owned()]);
    assert!(include_discovery_perp("BTCPERP", Some(&requested)));
    assert!(!include_discovery_perp("BTCUSDT", Some(&requested)));
}

#[test]
fn clamp_orderbook_limit_respects_category_caps() {
    // Current V5 orderbook limits: spot/linear/inverse 1000, option 25.
    assert_eq!(clamp_orderbook_limit("linear", 500), 500);
    assert_eq!(clamp_orderbook_limit("linear", 1000), 1000);
    assert_eq!(clamp_orderbook_limit("spot", 1000), 1000);
    assert_eq!(clamp_orderbook_limit("option", 100), 25);
    assert_eq!(clamp_orderbook_limit("inverse", 1000), 1000);
    assert_eq!(clamp_orderbook_limit("unknown", 1000), 1000);
    // 下界
    assert_eq!(clamp_orderbook_limit("linear", 0), 1);
}

#[test]
fn parse_funding_8h_no_op() {
    let t = MarketTickerItem {
        symbol: "BTCUSDT".into(),
        last_price: "30000".into(),
        bid1_price: "29999".into(),
        bid1_size: "1.0".into(),
        ask1_price: "30001".into(),
        ask1_size: "1.2".into(),
        turnover24h: "0".into(),
        funding_rate: "0.0001".into(),
        next_funding_time: "1700028800000".into(),
        funding_interval_hour: "8".into(),
        mark_price: "30000".into(),
        index_price: "30001".into(),
        open_interest: "10".into(),
        open_interest_value: "300000".into(),
    };
    let f = parse_funding(&t, 8, 1_000_000.0, 1_700_000_000_000).expect("funding parses");
    assert_eq!(f.symbol, "BTC");
    assert_eq!(f.exchange, "bybit");
    assert!((f.rate - 0.0001).abs() < 1e-12);
    assert!((f.rate_8h - 0.0001).abs() < 1e-12);
    assert_eq!(f.funding_interval, 8);
    // 修复 P2 3.10：服务端时间戳应被使用
    assert_eq!(f.timestamp, 1_700_000_000_000);
}

#[test]
fn parse_funding_4h_normalized() {
    let t = MarketTickerItem {
        symbol: "ETHUSDT".into(),
        last_price: "0".into(),
        bid1_price: "0".into(),
        bid1_size: "0".into(),
        ask1_price: "0".into(),
        ask1_size: "0".into(),
        turnover24h: "0".into(),
        funding_rate: "0.0001".into(),
        next_funding_time: "1700014400000".into(),
        funding_interval_hour: "4".into(),
        mark_price: "0".into(),
        index_price: "0".into(),
        open_interest: "0".into(),
        open_interest_value: "0".into(),
    };
    let f = parse_funding(&t, 4, 0.0, 0).expect("funding parses");
    // 4h 0.01% → 8h 标准化 0.02%
    assert!((f.rate_8h - 0.0002).abs() < 1e-12);
    assert_eq!(f.funding_interval, 4);
    // 修复 P2 3.10：server_time_ms == 0 时 fallback 到 now_ms()
    assert!(f.timestamp > 0);
}

#[test]
fn parse_spot_tick_pair_symbol() {
    let t = MarketTickerItem {
        symbol: "SOLUSDT".into(),
        last_price: "150.5".into(),
        bid1_price: "150.0".into(),
        bid1_size: "2".into(),
        ask1_price: "151.0".into(),
        ask1_size: "3".into(),
        turnover24h: "1000000".into(),
        funding_rate: String::new(),
        next_funding_time: String::new(),
        funding_interval_hour: String::new(),
        mark_price: String::new(),
        index_price: String::new(),
        open_interest: String::new(),
        open_interest_value: String::new(),
    };
    let tick = parse_spot_tick(&t, 1_700_000_000_000).unwrap();
    assert_eq!(tick.venue, "bybit");
    assert_eq!(tick.symbol, "SOL/USDT");
    assert_eq!(tick.exchange_ts_ms, Some(1_700_000_000_000));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn bybit_response_propagates_api_error() {
    let body = r#"{"retCode":10001,"retMsg":"params error","result":{"list":[]}}"#;
    let wrap: BybitResponse<MarketTickerItem> = serde_json::from_str(body).unwrap();
    let err = wrap.into_list("tickers").unwrap_err();
    assert!(matches!(err, ExchangeError::Api { .. }));
}

#[test]
fn require_credentials_errors_when_missing() {
    let b = bybit();
    assert!(b.require_credentials().is_err());
}

#[tokio::test]
async fn funding_payments_fetches_two_signed_pages_and_deduplicates_boundary() {
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};

    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("GET"))
        .and(path("/v5/account/transaction-log"))
        .and(query_param_is_missing("cursor"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(bybit_funding_page(
                "page-2",
                &[("event-1", "0.1", 1_700_000_000_000)],
            )),
        )
        .mount(&server)
        .await;
    wiremock::Mock::given(method("GET"))
        .and(path("/v5/account/transaction-log"))
        .and(query_param("cursor", "page-2"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(bybit_funding_page(
                "",
                &[
                    ("event-1", "0.1", 1_700_000_000_000),
                    ("event-2", "-0.2", 1_700_000_000_001),
                ],
            )),
        )
        .mount(&server)
        .await;
    let adapter = Bybit::new(BybitConfig {
        credentials: Some(BybitCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
        }),
        base_url_override: Some(server.uri()),
        ..BybitConfig::default()
    })
    .expect("adapter");
    adapter
        .time_synced_at_ms
        .store(common::time::now_ms(), Ordering::Relaxed);

    let payments = ExchangeAdapter::get_funding_payments(&adapter, None, None, None)
        .await
        .expect("two funding pages");

    assert_eq!(payments.len(), 2);
    assert_eq!(
        payments[0].venue_event_id,
        "bybit_funding:event-1:1700000000000"
    );
    assert_eq!(
        payments[1].venue_event_id,
        "bybit_funding:event-2:1700000000001"
    );
    let requests = server.received_requests().await.expect("request history");
    let signatures = requests
        .iter()
        .filter(|request| request.url.path() == "/v5/account/transaction-log")
        .filter_map(|request| request.headers.get("x-bapi-sign"))
        .collect::<Vec<_>>();
    assert_eq!(signatures.len(), 2);
    assert_ne!(signatures[0], signatures[1]);
}

#[tokio::test]
async fn private_account_reads_fan_out_usdt_and_usdc_settles() {
    use std::collections::BTreeSet;
    use wiremock::matchers::{method, path, query_param};

    let server = wiremock::MockServer::start().await;
    let empty = serde_json::json!({
        "retCode": 0,
        "retMsg": "OK",
        "time": 1_700_000_000_000_i64,
        "result": { "list": [] }
    });
    for (endpoint, settle) in [
        ("/v5/position/list", "USDT"),
        ("/v5/position/list", "USDC"),
        ("/v5/order/realtime", "USDT"),
        ("/v5/order/realtime", "USDC"),
    ] {
        wiremock::Mock::given(method("GET"))
            .and(path(endpoint))
            .and(query_param("category", "linear"))
            .and(query_param("settleCoin", settle))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(empty.clone()))
            .expect(1)
            .mount(&server)
            .await;
    }
    let adapter = Bybit::new(BybitConfig {
        credentials: Some(BybitCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
        }),
        base_url_override: Some(server.uri()),
        ..BybitConfig::default()
    })
    .expect("adapter");
    adapter
        .time_synced_at_ms
        .store(common::time::now_ms(), Ordering::Relaxed);

    assert!(ExchangeAdapter::get_positions(&adapter, None)
        .await
        .expect("USDT and USDC positions")
        .is_empty());
    assert!(ExchangeAdapter::get_open_orders(&adapter, None)
        .await
        .expect("USDT and USDC open orders")
        .is_empty());

    let requests = server.received_requests().await.expect("request history");
    let observed = requests
        .iter()
        .filter_map(|request| {
            let settle = request
                .url
                .query_pairs()
                .find(|(key, _)| key == "settleCoin")?
                .1
                .into_owned();
            Some((request.url.path().to_owned(), settle))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        observed,
        BTreeSet::from([
            ("/v5/order/realtime".to_owned(), "USDC".to_owned()),
            ("/v5/order/realtime".to_owned(), "USDT".to_owned()),
            ("/v5/position/list".to_owned(), "USDC".to_owned()),
            ("/v5/position/list".to_owned(), "USDT".to_owned()),
        ])
    );
}

fn bybit_funding_page(cursor: &str, rows: &[(&str, &str, i64)]) -> serde_json::Value {
    serde_json::json!({
        "retCode": 0,
        "retMsg": "OK",
        "result": {
            "nextPageCursor": cursor,
            "list": rows.iter().map(|(id, amount, timestamp)| serde_json::json!({
                "id": id,
                "symbol": "BTCUSDT",
                "funding": amount,
                "currency": "USDT",
                "transactionTime": timestamp.to_string(),
                "type": "SETTLEMENT"
            })).collect::<Vec<_>>()
        }
    })
}
