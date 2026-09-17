use super::*;
use crate::adapters::bitget_response::BitgetResponse;
use crate::adapters::bitget_uta_market_data::UtaTickerItem;
use pretty_assertions::assert_eq;
use std::sync::atomic::Ordering;

fn bitget() -> Bitget {
    Bitget::new(BitgetConfig::default()).unwrap()
}

#[test]
fn name() {
    assert_eq!(ExchangeAdapter::name(&bitget()), "bitget");
}

#[test]
fn symbol_round_trip() {
    let b = bitget();
    assert_eq!(b.to_exchange_symbol("BTC"), "BTCUSDT");
    assert_eq!(b.normalize_symbol("BTCUSDT"), "BTC");
}

#[test]
fn order_margin_modes_follow_configured_margin_mode() {
    assert_eq!(bitget().order_margin_modes(), vec![MarginMode::Cross]);
    let isolated = Bitget::new(BitgetConfig {
        margin_mode: BitgetMarginMode::Isolated,
        ..BitgetConfig::default()
    })
    .unwrap();
    assert_eq!(isolated.order_margin_modes(), vec![MarginMode::Isolated]);
}

#[test]
fn margin_mode_string_mapping() {
    // 修复 P1 4.2：margin_mode enum 正确转字符串。
    assert_eq!(BitgetMarginMode::Crossed.as_str(), "crossed");
    assert_eq!(BitgetMarginMode::Isolated.as_str(), "isolated");
}

#[test]
fn success_code_is_00000_not_0() {
    let body = r#"{"code":"00000","msg":"success","data":[]}"#;
    let wrap: BitgetResponse<UtaTickerItem> = serde_json::from_str(body).unwrap();
    assert!(wrap.into_data("tickers").is_ok());

    let bad = r#"{"code":"40001","msg":"bad","data":[]}"#;
    let wrap: BitgetResponse<UtaTickerItem> = serde_json::from_str(bad).unwrap();
    assert!(matches!(
        wrap.into_data("tickers").unwrap_err(),
        ExchangeError::Api { .. }
    ));
}

#[test]
fn require_credentials_errors_when_missing() {
    let b = bitget();
    assert!(b.require_credentials().is_err());
}

#[tokio::test]
async fn orderbook_preserves_requested_twenty_level_limit() {
    use wiremock::matchers::{method, path, query_param};

    let server = wiremock::MockServer::start().await;
    let fixture = include_str!("../../fixtures/bitget/uta_orderbook_usdt_futures_btcusdt.json");
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v3/market/orderbook"))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("symbol", "BTCUSDT"))
        .and(query_param("limit", "20"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(fixture))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Bitget::new(BitgetConfig {
        base_url_override: Some(server.uri()),
        ..BitgetConfig::default()
    })
    .expect("Bitget adapter");

    let book = ExchangeAdapter::get_orderbook(&adapter, "BTC", 20)
        .await
        .expect("Bitget orderbook");

    assert_eq!(book.bids.len(), 5);
    assert_eq!(book.asks.len(), 5);
    assert!((book.bids[0][1] - 0.4599).abs() < 1e-12);
}

#[tokio::test]
async fn exchange_order_id_lookup_uses_official_order_id_query() {
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};

    let server = wiremock::MockServer::start().await;
    let fixture = include_str!("../../fixtures/bitget/uta_order_info_filled.json");
    wiremock::Mock::given(method("GET"))
        .and(path(GET_ORDER_PATH))
        .and(query_param("category", "USDT-FUTURES"))
        .and(query_param("symbol", "ETHUSDT"))
        .and(query_param("orderId", "111111111111111111"))
        .and(query_param_is_missing("clientOid"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(fixture))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Bitget::new(BitgetConfig {
        credentials: Some(BitgetCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            passphrase: "pass".into(),
        }),
        allow_live_writes: true,
        base_url_override: Some(server.uri()),
        ..BitgetConfig::default()
    })
    .expect("Bitget adapter");

    let order =
        LiveTradingAdapter::get_order_by_exchange_order_id(&adapter, "ETH", "111111111111111111")
            .await
            .expect("orderId query")
            .expect("order row");

    assert_eq!(order.order_id, "111111111111111111");
}

#[tokio::test]
async fn funding_payments_pages_each_type_independently_and_resigns() {
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};

    let server = wiremock::MockServer::start().await;
    let paged_type = super::super::funding_payments::bitget_uta_funding_types()[0];
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v3/account/financial-records"))
        .and(query_param("type", paged_type))
        .and(query_param_is_missing("cursor"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(bitget_funding_page(
                "page-2",
                paged_type,
                &[("event-1", "0.1")],
            )),
        )
        .mount(&server)
        .await;
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v3/account/financial-records"))
        .and(query_param("type", paged_type))
        .and(query_param("cursor", "page-2"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(bitget_funding_page(
                "",
                paged_type,
                &[("event-1", "0.1"), ("event-2", "-0.2")],
            )),
        )
        .mount(&server)
        .await;
    for funding_type in &super::super::funding_payments::bitget_uta_funding_types()[1..] {
        wiremock::Mock::given(method("GET"))
            .and(path("/api/v3/account/financial-records"))
            .and(query_param("type", *funding_type))
            .and(query_param_is_missing("cursor"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(bitget_funding_page(
                    "",
                    funding_type,
                    &[],
                )),
            )
            .mount(&server)
            .await;
    }
    let adapter = Bitget::new(BitgetConfig {
        credentials: Some(BitgetCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            passphrase: "pass".into(),
        }),
        base_url_override: Some(server.uri()),
        ..BitgetConfig::default()
    })
    .expect("adapter");
    adapter
        .time_synced_at_ms
        .store(common::time::now_ms(), Ordering::Relaxed);

    let payments = ExchangeAdapter::get_funding_payments(&adapter, None, None, None)
        .await
        .expect("all funding-type pages");

    assert_eq!(payments.len(), 2);
    assert_eq!(payments[0].venue_event_id, "bitget_funding:event-1");
    assert_eq!(payments[1].venue_event_id, "bitget_funding:event-2");
    let requests = server.received_requests().await.expect("request history");
    assert_eq!(requests.len(), 5);
    let signatures = requests
        .iter()
        .filter(|request| {
            request
                .url
                .query_pairs()
                .any(|(key, value)| key == "type" && value == paged_type)
        })
        .filter_map(|request| request.headers.get("access-sign"))
        .collect::<Vec<_>>();
    assert_eq!(signatures.len(), 2);
    assert_ne!(signatures[0], signatures[1]);
}

fn bitget_funding_page(
    cursor: &str,
    funding_type: &str,
    rows: &[(&str, &str)],
) -> serde_json::Value {
    serde_json::json!({
        "code": "00000",
        "msg": "success",
        "data": {
            "cursor": cursor,
            "list": rows.iter().enumerate().map(|(index, (id, amount))| serde_json::json!({
                "id": id,
                "symbol": "BTCUSDT",
                "coin": "USDT",
                "type": funding_type,
                "amount": amount,
                "ts": (1_700_000_000_000_i64 + index as i64).to_string()
            })).collect::<Vec<_>>()
        }
    })
}
