#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Gate.io 适配器集成测试（wiremock）。

use exchange::{ExchangeAdapter, Gate, GateConfig, GateCredentials};
use serde_json::json;
use shared_types::IndexCompositionQuality;
use shared_types::{OrderStatus, OrderType};
use wiremock::matchers::{header_exists, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gate(server_uri: String, credentials: Option<GateCredentials>) -> Gate {
    Gate::new(GateConfig {
        credentials,
        allow_live_writes: false,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
        testnet: false,
    })
    .unwrap()
}

fn gate_with_writes(server_uri: String, credentials: Option<GateCredentials>) -> Gate {
    Gate::new(GateConfig {
        credentials,
        allow_live_writes: true,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
        testnet: false,
    })
    .unwrap()
}

async fn mount_btc_contract(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts/BTC_USDT"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(gate_contract("BTC_USDT", "0.0001", "0.1")),
        )
        .mount(server)
        .await;
}

async fn mount_position_contracts(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            gate_contract("BTC_USDT", "0.0001", "0.1"),
            gate_contract("ETH_USDT", "0.01", "0.01")
        ])))
        .mount(server)
        .await;
}

fn gate_contract(name: &str, multiplier: &str, price_tick: &str) -> serde_json::Value {
    json!({
        "name": name,
        "type": "direct",
        "quanto_multiplier": multiplier,
        "in_delisting": false,
        "status": "trading",
        "funding_interval": 28800,
        "order_price_round": price_tick,
        "order_size_min": 1,
        "order_size_max": 1000000,
        "market_order_size_max": 100000,
        "enable_decimal": false,
        "leverage_min": "1",
        "leverage_max": "100",
        "maintenance_rate": "0.005",
        "maker_fee_rate": "-0.0001",
        "taker_fee_rate": "0.0005"
    })
}

#[tokio::test]
async fn batch_funding_rates_reuse_contract_schedule_and_refresh_ticker_rates() {
    let server = MockServer::start().await;
    let next_apply = 2_000_016_000_i64;
    let contract = |name: &str, interval: i64, in_delisting: bool| {
        json!({
            "name": name,
            "type": "direct",
            "quanto_multiplier": "0.0001",
            "funding_next_apply": next_apply,
            "funding_interval": interval,
            "in_delisting": in_delisting,
            "status": if in_delisting { "delisted" } else { "trading" },
            "order_price_round": "0.1",
            "order_size_min": 1,
            "order_size_max": 1_000_000,
            "market_order_size_max": 100_000,
            "enable_decimal": false,
            "leverage_min": "1",
            "leverage_max": "100",
            "maintenance_rate": "0.005",
            "maker_fee_rate": "-0.0001",
            "taker_fee_rate": "0.0005"
        })
    };

    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            contract("BTC_USDT", 28_800, false),
            contract("ETH_USDT", 14_400, false),
            contract("DEAD_USDT", 28_800, true),
            contract("BTC_USDC", 28_800, false)
        ])))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/tickers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "contract": "BTC_USDT",
                "last": "30000",
                "highest_bid": "29999",
                "lowest_ask": "30001",
                "funding_rate": "0.0001",
                "funding_rate_indicative": "0.00011",
                "volume_24h_quote": "1500000000",
                "volume_24h_settle": "1500000000"
            },
            {
                "contract": "ETH_USDT",
                "last": "2000",
                "highest_bid": "1999",
                "lowest_ask": "2001",
                "funding_rate": "-0.00005",
                "funding_rate_indicative": "-0.00004",
                "volume_24h_quote": "500000000",
                "volume_24h_settle": "500000000"
            }
        ])))
        .expect(2)
        .mount(&server)
        .await;

    let g = gate(server.uri(), None);
    let rates = g.get_funding_rates(None).await.expect("ok");
    let second_rates = g.get_funding_rates(None).await.expect("cached schedule");

    assert_eq!(rates.len(), 2, "DEAD_USDT 退市 + BTC_USDC 非 USDT 应过滤");
    assert_eq!(second_rates.len(), 2);
    let btc = rates.iter().find(|r| r.symbol == "BTC").unwrap();
    assert_eq!(btc.exchange, "gate");
    assert!((btc.rate - 0.0001).abs() < 1e-12);
    assert_eq!(btc.funding_interval, 8); // 28800s = 8h
    assert!((btc.rate_8h - 0.0001).abs() < 1e-12);
    assert_eq!(btc.next_funding_time, next_apply * 1000); // 秒 → 毫秒
    assert!((btc.volume_24h - 1_500_000_000.0).abs() < 1.0);

    let eth = rates.iter().find(|r| r.symbol == "ETH").unwrap();
    assert_eq!(eth.funding_interval, 4); // 14400s = 4h
    assert!((eth.rate_8h - (-0.0001)).abs() < 1e-12); // 4h -0.005% → 8h -0.01%
}

#[tokio::test]
async fn get_funding_rate_single_via_contract_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts/BTC_USDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "BTC_USDT",
            "funding_rate": "0.0003",
            "funding_next_apply": 1_700_028_800_i64,
            "funding_interval": 28800,
            "in_delisting": false
        })))
        .mount(&server)
        .await;

    let g = gate(server.uri(), None);
    let r = g.get_funding_rate("BTC").await.expect("ok");
    assert_eq!(r.symbol, "BTC");
    assert!((r.rate - 0.0003).abs() < 1e-12);
}

#[tokio::test]
async fn index_composition_constituents_follow_official_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/index_constituents/BTC_USDT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "index": "BTC_USDT",
            "constituents": [
                {"exchange": "Binance", "symbols": ["BTC_USDT"]},
                {"exchange": "Gate.com", "symbols": ["BTC_USDT", "BTC_USDC"]},
                {"exchange": "Coinbase", "symbols": []}
            ]
        })))
        .mount(&server)
        .await;

    let g = gate(server.uri(), None);
    let snapshot = g
        .get_index_composition("BTC")
        .await
        .expect("index composition");

    assert_eq!(snapshot.venue, "gate");
    assert_eq!(snapshot.symbol, "BTC");
    assert_eq!(snapshot.index_id, "BTC_USDT");
    assert_eq!(snapshot.quality, IndexCompositionQuality::Verified);
    assert_eq!(
        snapshot.source,
        "GET /api/v4/futures/usdt/index_constituents/{index}"
    );
    assert_eq!(snapshot.components.len(), 3);
    assert_eq!(snapshot.components[0].name, "Binance");
    assert_eq!(snapshot.components[0].symbol, "BTC_USDT");
    assert_eq!(snapshot.components[0].weight, 0.0);
    assert_eq!(snapshot.components[0].price, None);
    assert_eq!(snapshot.components[2].name, "Gate.com");
    assert_eq!(snapshot.components[2].symbol, "BTC_USDC");
}

#[tokio::test]
async fn balance_signed_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/accounts"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "currency": "USDT",
            "total": "10000",
            "available": "9000",
            "position_margin": "500",
            "order_margin": "500",
            "unrealised_pnl": "5"
        })))
        .mount(&server)
        .await;

    let g = gate(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let balances = g.get_balance(None).await.expect("ok");
    let usdt = balances.get("USDT").unwrap();
    assert!((usdt.total - 10000.0).abs() < 1e-9);
    assert!((usdt.available - 9000.0).abs() < 1e-9);
    assert!((usdt.frozen - 1000.0).abs() < 1e-9); // position_margin + order_margin
    assert!((usdt.unrealized_pnl - 5.0).abs() < 1e-9);
}

#[tokio::test]
async fn positions_signed_size_to_long_short() {
    let server = MockServer::start().await;
    mount_position_contracts(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/positions"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "contract": "BTC_USDT",
                "size": 5,
                "entry_price": "30000",
                "mark_price": "30100",
                "unrealised_pnl": "0.5",
                "leverage": "10",
                "liq_price": "27000",
                "margin": "10"
            },
            {
                "contract": "ETH_USDT",
                "size": -3,
                "entry_price": "2000",
                "mark_price": "1990",
                "unrealised_pnl": "0.3",
                "leverage": "10",
                "liq_price": "2200",
                "margin": "5"
            },
            // 平仓状态（size=0）应被过滤
            {
                "contract": "SOL_USDT",
                "size": 0,
                "entry_price": "0",
                "mark_price": "0",
                "unrealised_pnl": "0",
                "leverage": "1",
                "liq_price": "0",
                "margin": "0"
            }
        ])))
        .mount(&server)
        .await;

    let g = gate(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let positions = g.get_positions(None).await.expect("ok");
    assert_eq!(positions.len(), 2);
    let btc = positions.iter().find(|p| p.symbol == "BTC").unwrap();
    assert_eq!(btc.side, "long");
    assert!((btc.quantity - 0.0005).abs() < 1e-12);
    let eth = positions.iter().find(|p| p.symbol == "ETH").unwrap();
    assert_eq!(eth.side, "short");
    assert!((eth.quantity - 0.03).abs() < 1e-12);
}

#[tokio::test]
async fn safe_cancel_no_match_probe_uses_official_single_cancel_without_live_writes() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path_regex(
            r"^/api/v4/futures/usdt/orders/t-[A-Za-z0-9_.-]{1,28}$",
        ))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "label": "ORDER_NOT_FOUND",
            "message": "order not found"
        })))
        .mount(&server)
        .await;

    let g = gate(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );

    g.validate_safe_order_cancel_no_match_permission()
        .await
        .expect("save-time no-match cancel probe does not require live writes");
}

#[tokio::test]
async fn safe_cancel_no_match_probe_rejects_unexpected_match() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path_regex(
            r"^/api/v4/futures/usdt/orders/t-[A-Za-z0-9_.-]{1,28}$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7,
            "text": "t-xline-collision"
        })))
        .mount(&server)
        .await;

    let g = gate(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let error = g
        .validate_safe_order_cancel_no_match_permission()
        .await
        .expect_err("matching a real order must fail the no-match probe");

    assert!(matches!(
        error,
        exchange::ExchangeError::Api { code, .. } if code == "safe_cancel_collision"
    ));
}

#[tokio::test]
async fn get_order_uses_official_order_detail_by_gate_text() {
    let server = MockServer::start().await;
    mount_btc_contract(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/orders/t-cid-1"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7,
            "contract": "BTC_USDT",
            "status": "finished",
            "finish_as": "filled",
            "size": 5,
            "left": 0,
            "price": "0",
            "fill_price": "30000",
            "create_time_ms": 1,
            "text": "t-cid-1",
            "tif": "ioc"
        })))
        .mount(&server)
        .await;

    let g = gate_with_writes(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let order = exchange::LiveTradingAdapter::get_order(&g, "BTC", "cid-1")
        .await
        .expect("query succeeds")
        .expect("order found");

    assert_eq!(order.order_id, "7");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.order_type, OrderType::Market);
    assert_eq!(order.status, OrderStatus::Filled);
}

#[tokio::test]
async fn get_order_rejects_malformed_order_detail() {
    let server = MockServer::start().await;
    mount_btc_contract(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/orders/t-cid-1"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7,
            "contract": "BTC_USDT",
            "status": "finished",
            "finish_as": "filled",
            "size": 5,
            "left": 0,
            "price": "bad-price",
            "fill_price": "30000",
            "create_time_ms": 1,
            "text": "t-cid-1",
            "tif": "ioc"
        })))
        .mount(&server)
        .await;

    let g = gate_with_writes(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let err = exchange::LiveTradingAdapter::get_order(&g, "BTC", "cid-1")
        .await
        .expect_err("bad price must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("invalid price")
    ));
}

#[tokio::test]
async fn get_order_rejects_malformed_order_detail_json_before_fallback() {
    let server = MockServer::start().await;
    mount_btc_contract(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/orders/t-cid-1"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7,
            "contract": "BTC_USDT",
            "status": "finished",
            "finish_as": "filled",
            "size": 5,
            "left": 0,
            "price": "0",
            "fill_price": "30000",
            "text": "t-cid-1",
            "tif": "ioc"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/orders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let g = gate_with_writes(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let err = exchange::LiveTradingAdapter::get_order(&g, "BTC", "cid-1")
        .await
        .expect_err("detail schema drift must fail before fallback");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message)
            if message.contains("missing a valid create_time_ms/create_time")
    ));
}

#[tokio::test]
async fn get_order_falls_back_to_open_orders_only_on_detail_404() {
    let server = MockServer::start().await;
    mount_btc_contract(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/orders/t-cid-1"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "label": "ORDER_NOT_FOUND",
            "message": "not found"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/orders"))
        .and(query_param("status", "open"))
        .and(query_param("contract", "BTC_USDT"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 7,
                "contract": "BTC_USDT",
                "status": "open",
                "size": 5,
                "left": 5,
                "price": "30000",
                "fill_price": "0",
                "create_time_ms": 1,
                "text": "t-cid-1",
                "tif": "gtc"
            }
        ])))
        .mount(&server)
        .await;

    let g = gate_with_writes(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let order = exchange::LiveTradingAdapter::get_order(&g, "BTC", "cid-1")
        .await
        .expect("404 detail may fallback")
        .expect("fallback order found");

    assert_eq!(order.order_id, "7");
    assert_eq!(order.status, OrderStatus::Open);
}

#[tokio::test]
async fn open_orders_rejects_malformed_order_row() {
    let server = MockServer::start().await;
    mount_btc_contract(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/orders"))
        .and(query_param("status", "open"))
        .and(query_param("contract", "BTC_USDT"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 7,
                "contract": "BTC_USDT",
                "status": "open",
                "size": 5,
                "left": 5,
                "price": "30000",
                "fill_price": "bad-fill",
                "create_time_ms": 1,
                "text": "t-cid-1",
                "tif": "gtc"
            }
        ])))
        .mount(&server)
        .await;

    let g = gate(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let err = g
        .get_open_orders(Some("BTC"))
        .await
        .expect_err("bad fill_price must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("invalid fill_price")
    ));
}

#[tokio::test]
async fn balance_rejects_malformed_numeric() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/accounts"))
        .and(header_exists("KEY"))
        .and(header_exists("Timestamp"))
        .and(header_exists("SIGN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "currency": "USDT",
            "total": "10000",
            "available": "bad-available",
            "position_margin": "500",
            "order_margin": "500",
            "unrealised_pnl": "5"
        })))
        .mount(&server)
        .await;

    let g = gate(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let err = g
        .get_balance(None)
        .await
        .expect_err("bad available must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("invalid available")
    ));
}

#[tokio::test]
async fn positions_reject_malformed_numeric() {
    let server = MockServer::start().await;
    mount_position_contracts(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/positions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "contract": "BTC_USDT",
                "size": 5,
                "entry_price": "30000",
                "mark_price": "bad-mark",
                "unrealised_pnl": "0.5",
                "leverage": "10",
                "liq_price": "27000",
                "margin": "10"
            }
        ])))
        .mount(&server)
        .await;

    let g = gate(
        server.uri(),
        Some(GateCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
        }),
    );
    let err = g
        .get_positions(None)
        .await
        .expect_err("bad mark_price must fail closed");

    assert!(matches!(
        err,
        exchange::ExchangeError::Parse(message) if message.contains("invalid mark_price")
    ));
}

#[tokio::test]
async fn missing_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let g = gate(server.uri(), None);
    let err = g.get_balance(None).await.expect_err("auth");
    assert!(matches!(err, exchange::ExchangeError::Auth(_)));
}
