//! PR-CY KuCoin non-idempotent write and read-side recovery contract.

use exchange::{Kucoin, KucoinConfig, KucoinCredentials, LiveTradingAdapter};
use serde_json::{json, Value};
use shared_types::{
    ExecutionMode, LiveOrderState, MarginMode, OrderIntent, OrderSide, OrderSource, OrderType,
    TimeInForce,
};
use std::error::Error;
use std::io;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

type TestResult = Result<(), Box<dyn Error>>;

fn kucoin(server: &MockServer) -> exchange::ExchangeResult<Kucoin> {
    Kucoin::new(KucoinConfig {
        credentials: Some(KucoinCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
            passphrase: "test-passphrase".into(),
        }),
        allow_live_writes: true,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server.uri()),
        margin_mode: exchange::adapters::KucoinMarginMode::Isolated,
        default_leverage: 1,
    })
}

fn limit_intent() -> OrderIntent {
    OrderIntent {
        id: "internal-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "kucoin".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(30_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

#[tokio::test]
async fn ambiguous_place_result_recovers_by_client_oid_without_replaying_post() -> TestResult {
    let server = MockServer::start().await;
    mount_order_context(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/orders"))
        .and(body_partial_json(json!({"clientOid": "client-1"})))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream timeout"))
        .expect(1)
        .mount(&server)
        .await;
    mount_order_query(&server, "client-1").await?;

    let ack = LiveTradingAdapter::place_order(&kucoin(&server)?, &limit_intent()).await?;

    assert_eq!(ack.exchange_order_id.as_deref(), Some("250444645610336256"));
    assert_eq!(ack.state, LiveOrderState::Accepted);
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("client-1")
    );
    assert!(ack
        .message
        .is_some_and(|message| message.contains("byClientOid")));
    assert_request_counts(&server, 1, 1).await
}

#[tokio::test]
async fn ambiguous_place_result_fails_closed_on_mismatched_query_identity() -> TestResult {
    let server = MockServer::start().await;
    mount_order_context(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/orders"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream timeout"))
        .expect(1)
        .mount(&server)
        .await;
    mount_order_query(&server, "different-client").await?;

    let Err(error) = LiveTradingAdapter::place_order(&kucoin(&server)?, &limit_intent()).await
    else {
        return Err(
            io::Error::other("mismatched query identity unexpectedly recovered the write").into(),
        );
    };

    assert!(matches!(
        error,
        exchange::ExchangeError::Http { status: 503, .. }
    ));
    assert_request_counts(&server, 1, 1).await
}

async fn mount_order_context(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v2/position/getPositionMode"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": {"positionMode": 0}
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/contracts/active"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": [{
                "symbol": "XBTUSDTM",
                "baseCurrency": "XBT",
                "quoteCurrency": "USDT",
                "settleCurrency": "USDT",
                "multiplier": "0.001",
                "lotSize": 1,
                "tickSize": "0.1",
                "maxOrderQty": 1000000,
                "marketMaxOrderQty": 1000000,
                "maxLeverage": 125,
                "status": "Open"
            }]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/position"))
        .and(query_param("symbol", "XBTUSDTM"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "200000",
            "data": []
        })))
        .mount(server)
        .await;
}

async fn mount_order_query(server: &MockServer, response_client_oid: &str) -> TestResult {
    let mut fixture: Value = serde_json::from_str(include_str!(
        "../fixtures/kucoin/get_order_by_client_oid_open.json"
    ))?;
    fixture["data"]["symbol"] = json!("XBTUSDTM");
    fixture["data"]["clientOid"] = json!(response_client_oid);
    Mock::given(method("GET"))
        .and(path("/api/v1/orders/byClientOid"))
        .and(query_param("clientOid", "client-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture))
        .expect(1)
        .mount(server)
        .await;
    Ok(())
}

async fn assert_request_counts(server: &MockServer, posts: usize, queries: usize) -> TestResult {
    let requests = server
        .received_requests()
        .await
        .ok_or_else(|| io::Error::other("wiremock request history is unavailable"))?;
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .count(),
        posts
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/api/v1/orders/byClientOid")
            .count(),
        queries
    );
    Ok(())
}
