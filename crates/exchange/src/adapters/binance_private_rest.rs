//! Binance private REST request helpers.
//!
//! This module keeps signed private HTTP calls out of the main adapter. Signing
//! remains in `binance.rs` so time sync and credential ownership stay in one place.

use super::binance_fee_evidence::{
    parse_commission_rate, BinanceCommissionRateEvidence, BinanceCommissionRateResponse,
    BINANCE_COMMISSION_RATE_PATH,
};
use super::binance_private_data::{
    parse_account_info_v3, parse_balances, parse_open_order, parse_positions_with_mode,
    AccountInfoV3, BalanceItem, OpenOrderItem, ParsedAccountRead, ParsedPositions, PositionItem,
};
use super::binance_response::{checked_json, ListenKeyResponse};
use super::binance_trade_data::ack_from_order_item;
use super::funding_payments::{
    parse_binance_funding_payments, BinanceIncomeRow, BINANCE_FUNDING_INCOME_PATH,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{venue_balance_rows, VenueAccountRead};
use reqwest::Method;
use shared_types::{BalanceInfo, FundingPaymentData, OrderAck, OrderInfo};
use std::collections::HashMap;

pub(super) const ORDER_PATH: &str = "/fapi/v1/order";
pub(super) const TEST_ORDER_PATH: &str = "/fapi/v1/order/test";

pub(super) async fn user_stream_listen_key(
    http: &HttpClient,
    base_url: &str,
    method: Method,
    api_key: &str,
    context: &str,
) -> ExchangeResult<String> {
    let url = user_stream_url(base_url);
    let resp = http
        .execute_with_retry(|| {
            http.request(method.clone(), &url)
                .header("X-MBX-APIKEY", api_key)
        })
        .await?;
    let body: ListenKeyResponse = checked_json(resp, context).await?;
    if body.listen_key.is_empty() {
        return Err(ExchangeError::Parse(format!(
            "binance {context}: empty listenKey"
        )));
    }
    Ok(body.listen_key)
}

pub(super) async fn close_user_data_stream(
    http: &HttpClient,
    base_url: &str,
    api_key: &str,
) -> ExchangeResult<()> {
    let url = user_stream_url(base_url);
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::DELETE, &url)
                .header("X-MBX-APIKEY", api_key)
        })
        .await?;
    let _: serde_json::Value = checked_json(resp, "binance close user data stream").await?;
    Ok(())
}

pub(super) async fn place_order(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
    internal_order_id: String,
    client_order_id: String,
) -> ExchangeResult<OrderAck> {
    let url = signed_url(base_url, ORDER_PATH, signed_query, api_key)?;
    let item =
        signed_json_once::<OpenOrderItem>(http, Method::POST, &url, api_key, "binance place order")
            .await?;
    Ok(ack_from_order_item(
        internal_order_id,
        client_order_id,
        &item,
    ))
}

pub(super) async fn test_order<F>(
    http: &HttpClient,
    base_url: &str,
    mut sign: F,
) -> ExchangeResult<()>
where
    F: FnMut() -> ExchangeResult<(String, String)>,
{
    let endpoint = format!("{base_url}{TEST_ORDER_PATH}");
    let response = http
        .execute_with_retry_fresh(Method::POST, &endpoint, || {
            let (signed_query, api_key) = sign()?;
            let url = signed_url(base_url, TEST_ORDER_PATH, &signed_query, &api_key)?;
            Ok(http
                .request(Method::POST, url)
                .header("X-MBX-APIKEY", api_key))
        })
        .await?;
    let _: serde_json::Value = checked_json(response, "binance test order").await?;
    Ok(())
}

pub(super) async fn safe_cancel_probe<F>(
    http: &HttpClient,
    base_url: &str,
    mut sign: F,
) -> ExchangeResult<()>
where
    F: FnMut() -> ExchangeResult<(String, String)>,
{
    let endpoint = format!("{base_url}{ORDER_PATH}");
    let resp = http
        .execute_with_retry_fresh(Method::DELETE, &endpoint, || {
            let (signed_query, api_key) = sign()?;
            let url = signed_url(base_url, ORDER_PATH, &signed_query, &api_key)?;
            Ok(http
                .request(Method::DELETE, url)
                .header("X-MBX-APIKEY", api_key))
        })
        .await?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if binance_safe_cancel_nonmatch(&text) {
        return Ok(());
    }
    if status.is_success() {
        return Err(ExchangeError::Api {
            exchange: "binance".into(),
            code: "safe_cancel_collision".into(),
            message:
                "binance safe cancel probe unexpectedly matched an order; no-match evidence rejected"
                    .into(),
        });
    }
    Err(ExchangeError::Http {
        status: status.as_u16(),
        body: text,
    })
}

pub(super) async fn cancel_order(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
    internal_order_id: String,
    client_order_id: String,
) -> ExchangeResult<OrderAck> {
    let url = signed_url(base_url, ORDER_PATH, signed_query, api_key)?;
    let item = signed_json_once::<OpenOrderItem>(
        http,
        Method::DELETE,
        &url,
        api_key,
        "binance cancel order",
    )
    .await?;
    Ok(ack_from_order_item(
        internal_order_id,
        client_order_id,
        &item,
    ))
}

pub(super) async fn get_order(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
) -> ExchangeResult<Option<OrderInfo>> {
    let url = signed_url(base_url, ORDER_PATH, signed_query, api_key)?;
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .header("X-MBX-APIKEY", api_key)
        })
        .await?;
    let item: OpenOrderItem = checked_json(resp, "binance get order").await?;
    parse_open_order(&item).map(Some)
}

pub(super) async fn balances(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let url = signed_url(base_url, "/fapi/v3/balance", signed_query, api_key)?;
    let items =
        signed_json::<Vec<BalanceItem>>(http, Method::GET, &url, api_key, "binance balance")
            .await?;
    parse_balances(items, currency)
}

pub(super) async fn account_read(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
    currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountRead> {
    let url = signed_url(base_url, "/fapi/v3/account", signed_query, api_key)?;
    let item = signed_json::<AccountInfoV3>(
        http,
        Method::GET,
        &url,
        api_key,
        "binance account information",
    )
    .await?;
    let ParsedAccountRead { balances, summary } =
        parse_account_info_v3(item, currency, observed_at_ms)?;
    Ok(VenueAccountRead {
        balances: venue_balance_rows("binance", balances),
        summaries: vec![summary],
        asset_valuations: Vec::new(),
        issues: Vec::new(),
    })
}

pub(super) async fn funding_payments(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
) -> ExchangeResult<Vec<FundingPaymentData>> {
    let url = signed_url(base_url, BINANCE_FUNDING_INCOME_PATH, signed_query, api_key)?;
    let rows = signed_json::<Vec<BinanceIncomeRow>>(
        http,
        Method::GET,
        &url,
        api_key,
        "binance funding payments",
    )
    .await?;
    parse_binance_funding_payments(rows)
}

pub(super) async fn positions(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
    target_exchange_symbol: Option<&str>,
) -> ExchangeResult<ParsedPositions> {
    let url = signed_url(base_url, "/fapi/v3/positionRisk", signed_query, api_key)?;
    let items =
        signed_json::<Vec<PositionItem>>(http, Method::GET, &url, api_key, "binance positions")
            .await?;
    parse_positions_with_mode(items, target_exchange_symbol)
}

pub(super) async fn open_orders(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
) -> ExchangeResult<Vec<OrderInfo>> {
    let url = signed_url(base_url, "/fapi/v1/openOrders", signed_query, api_key)?;
    let items =
        signed_json::<Vec<OpenOrderItem>>(http, Method::GET, &url, api_key, "binance open orders")
            .await?;
    items.iter().map(parse_open_order).collect()
}

#[allow(dead_code)]
pub(super) async fn commission_rate(
    http: &HttpClient,
    base_url: &str,
    signed_query: &str,
    api_key: &str,
    expected_symbol: &str,
    fetched_at_ms: i64,
) -> ExchangeResult<BinanceCommissionRateEvidence> {
    let url = signed_url(
        base_url,
        BINANCE_COMMISSION_RATE_PATH,
        signed_query,
        api_key,
    )?;
    let row = signed_json::<BinanceCommissionRateResponse>(
        http,
        Method::GET,
        &url,
        api_key,
        "binance commission rate",
    )
    .await?;
    parse_commission_rate(&row, expected_symbol, fetched_at_ms)
}

async fn signed_json<T: serde::de::DeserializeOwned>(
    http: &HttpClient,
    method: Method,
    url: &str,
    api_key: &str,
    context: &str,
) -> ExchangeResult<T> {
    let resp = http
        .execute_with_retry(|| {
            http.request(method.clone(), url)
                .header("X-MBX-APIKEY", api_key)
        })
        .await?;
    checked_json(resp, context).await
}

/// 非幂等写（`place`/`cancel`）：单次尝试，不重放（`signed_query` 内含一次性
/// 时间戳），结果由读侧 `get_order` 按 `client_order_id` 对账。
async fn signed_json_once<T: serde::de::DeserializeOwned>(
    http: &HttpClient,
    method: Method,
    url: &str,
    api_key: &str,
    context: &str,
) -> ExchangeResult<T> {
    let resp = http
        .execute_once(|| {
            http.request(method.clone(), url)
                .header("X-MBX-APIKEY", api_key)
        })
        .await?;
    checked_json(resp, context).await
}

fn user_stream_url(base_url: &str) -> String {
    format!("{base_url}/fapi/v1/listenKey")
}

fn signed_url(
    base_url: &str,
    path: &str,
    signed_query: &str,
    api_key: &str,
) -> ExchangeResult<String> {
    if api_key.trim().is_empty() {
        return Err(ExchangeError::Auth(
            "binance private request missing API key".into(),
        ));
    }
    validate_signed_query(signed_query)?;
    Ok(format!("{base_url}{path}?{signed_query}"))
}

fn validate_signed_query(query: &str) -> ExchangeResult<()> {
    let pairs: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
    let timestamp = exactly_one_param(&pairs, "timestamp")?;
    let signature = exactly_one_param(&pairs, "signature")?;
    let timestamp = timestamp.parse::<i64>().map_err(|_| {
        ExchangeError::Auth("binance signed request timestamp must be a positive integer".into())
    })?;
    if timestamp <= 0 {
        return Err(ExchangeError::Auth(
            "binance signed request timestamp must be a positive integer".into(),
        ));
    }
    if signature.len() != 64 || !signature.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ExchangeError::Auth(
            "binance signed request signature must be 64 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn exactly_one_param<'a>(
    pairs: &'a [(std::borrow::Cow<'a, str>, std::borrow::Cow<'a, str>)],
    key: &str,
) -> ExchangeResult<&'a str> {
    let mut values = pairs
        .iter()
        .filter_map(|(candidate, value)| (candidate == key).then_some(value.as_ref()));
    let value = values
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ExchangeError::Auth(format!("binance signed request missing {key}")))?;
    if values.next().is_some() {
        return Err(ExchangeError::Auth(format!(
            "binance signed request has duplicate {key}"
        )));
    }
    Ok(value)
}

fn binance_safe_cancel_nonmatch(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("-2011")
        || body.contains("unknown order")
        || body.contains("order does not exist")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Binance documents unknown orders as a non-success API error envelope.
    // Every non-success status, including an undocumented 404, stays fail-closed.
    fn binance_http() -> HttpClient {
        HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client")
    }

    fn test_signed_query(params: &str) -> String {
        let suffix = "timestamp=1700000000000&signature=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        if params.is_empty() {
            suffix.to_owned()
        } else {
            format!("{params}&{suffix}")
        }
    }

    #[tokio::test]
    async fn get_order_fails_closed_on_non_official_404() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(wiremock::ResponseTemplate::new(404).set_body_string("{}"))
            .mount(&server)
            .await;
        let http = binance_http();
        let error = get_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT&orderId=777"),
            "key",
        )
        .await
        .expect_err("Binance documents an API error envelope, not a 404 not-found response");
        assert!(matches!(error, ExchangeError::Http { status: 404, .. }));
    }

    #[tokio::test]
    async fn get_order_fails_closed_on_unknown_order_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(
                wiremock::ResponseTemplate::new(400)
                    .set_body_string(r#"{"code":-2013,"msg":"Order does not exist."}"#),
            )
            .mount(&server)
            .await;
        let http = binance_http();
        let err = get_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT&orderId=777"),
            "key",
        )
        .await
        .expect_err("unknown-order 400/-2013 must fail-closed, not become None");
        assert!(matches!(err, ExchangeError::Http { status: 400, .. }));
    }

    #[tokio::test]
    async fn get_order_parses_official_filled_order() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .and(wiremock::matchers::query_param("symbol", "BTCUSDT"))
            .and(wiremock::matchers::query_param(
                "timestamp",
                "1700000000000",
            ))
            .and(wiremock::matchers::query_param(
                "signature",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ))
            .and(wiremock::matchers::header("X-MBX-APIKEY", "key"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(include_str!(
                    "../../fixtures/binance/usdm_get_order_filled.json"
                )),
            )
            .mount(&server)
            .await;
        let http = binance_http();
        let info = get_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT&orderId=283194212"),
            "key",
        )
        .await
        .expect("200 yields a parsed order")
        .expect("order present");
        assert_eq!(info.order_id, "283194212");
        assert!(matches!(info.status, shared_types::OrderStatus::Filled));
        assert!(matches!(info.side, shared_types::OrderSide::Buy));
        assert_eq!(info.filled_price, 50000.0);
    }

    #[tokio::test]
    async fn get_order_fails_closed_on_invalid_body() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("not-json"))
            .mount(&server)
            .await;
        let http = binance_http();
        assert!(get_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT&orderId=777"),
            "key",
        )
        .await
        .is_err());
    }

    // PR-CV (write path): official Binance USD-M place/cancel HTTP-status
    // finality fixtures. place_order/cancel_order both decode the order item via
    // signed_json -> checked_json, so any non-success status must fail closed as
    // a typed Http error rather than fabricate an ack, and the success ack state
    // must reflect the official order status (NEW -> Accepted, CANCELED ->
    // Cancelled).
    #[tokio::test]
    async fn place_order_official_envelope_builds_accepted_ack() {
        let server = wiremock::MockServer::start().await;
        let body = r#"{"orderId":778,"symbol":"BTCUSDT","status":"NEW","type":"LIMIT","side":"BUY","price":"30000","origQty":"1","executedQty":"0","avgPrice":"0","time":1700000000000,"timeInForce":"GTC","clientOrderId":"x-place","reduceOnly":false}"#;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let http = binance_http();
        let ack = place_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT"),
            "key",
            "internal-1".to_owned(),
            "x-place".to_owned(),
        )
        .await
        .expect("place order ack");
        assert_eq!(ack.exchange_order_id.as_deref(), Some("778"));
        assert!(matches!(ack.state, shared_types::LiveOrderState::Accepted));
    }

    #[tokio::test]
    async fn place_order_fails_closed_on_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(
                wiremock::ResponseTemplate::new(400)
                    .set_body_string(r#"{"code":-2010,"msg":"Order would immediately trigger."}"#),
            )
            .mount(&server)
            .await;
        let http = binance_http();
        let err = place_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT"),
            "key",
            "internal-1".to_owned(),
            "x-place".to_owned(),
        )
        .await
        .expect_err("rejected place must fail-closed");
        assert!(matches!(err, ExchangeError::Http { status: 400, .. }));
    }

    #[tokio::test]
    async fn test_order_uses_official_non_matching_endpoint() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/fapi/v1/order/test"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        let http = binance_http();

        test_order(&http, &server.uri(), || {
            Ok((test_signed_query("symbol=BTCUSDT"), "key".to_owned()))
        })
        .await
        .expect("official test order endpoint accepts an empty success body");
    }

    #[tokio::test]
    async fn safe_cancel_probe_rejects_unexpected_2xx_collision() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        let http = binance_http();

        let error = safe_cancel_probe(&http, &server.uri(), || {
            Ok((
                test_signed_query("symbol=BTCUSDT&origClientOrderId=x"),
                "key".to_owned(),
            ))
        })
        .await
        .expect_err("2xx means the no-match cancel probe unexpectedly matched an order");

        assert!(matches!(
            error,
            ExchangeError::Api {
                code,
                ..
            } if code == "safe_cancel_collision"
        ));
    }

    #[tokio::test]
    async fn safe_cancel_probe_treats_unknown_order_as_nonmatching_evidence() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(
                wiremock::ResponseTemplate::new(400)
                    .set_body_string(r#"{"code":-2011,"msg":"Unknown order sent."}"#),
            )
            .mount(&server)
            .await;
        let http = binance_http();

        safe_cancel_probe(&http, &server.uri(), || {
            Ok((
                test_signed_query("symbol=BTCUSDT&origClientOrderId=x"),
                "key".to_owned(),
            ))
        })
        .await
        .expect("unknown order proves the nonmatching probe did not cancel a live order");
    }

    #[tokio::test]
    async fn cancel_order_official_envelope_builds_cancelled_ack() {
        let server = wiremock::MockServer::start().await;
        let body = r#"{"orderId":778,"symbol":"BTCUSDT","status":"CANCELED","type":"LIMIT","side":"BUY","price":"30000","origQty":"1","executedQty":"0","avgPrice":"0","time":1700000000000,"timeInForce":"GTC","clientOrderId":"x-place","reduceOnly":false}"#;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let http = binance_http();
        let ack = cancel_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT&orderId=778"),
            "key",
            "internal-1".to_owned(),
            "x-place".to_owned(),
        )
        .await
        .expect("cancel order ack");
        assert_eq!(ack.exchange_order_id.as_deref(), Some("778"));
        assert!(matches!(ack.state, shared_types::LiveOrderState::Cancelled));
    }

    #[tokio::test]
    async fn cancel_order_fails_closed_on_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path("/fapi/v1/order"))
            .respond_with(
                wiremock::ResponseTemplate::new(400)
                    .set_body_string(r#"{"code":-2011,"msg":"Unknown order sent."}"#),
            )
            .mount(&server)
            .await;
        let http = binance_http();
        let err = cancel_order(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT&orderId=778"),
            "key",
            "internal-1".to_owned(),
            "x-place".to_owned(),
        )
        .await
        .expect_err("unknown-order cancel must fail-closed");
        assert!(matches!(err, ExchangeError::Http { status: 400, .. }));
    }

    #[tokio::test]
    async fn signed_private_reads_use_official_paths_and_preserve_commission_rates() {
        let server = wiremock::MockServer::start().await;
        let signature = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for (path, fixture) in [
            (
                "/fapi/v3/balance",
                include_str!("../../fixtures/binance/usdm_balance_v3.json"),
            ),
            (
                "/fapi/v3/positionRisk",
                include_str!("../../fixtures/binance/usdm_position_risk_btcusdt.json"),
            ),
            (
                "/fapi/v1/openOrders",
                include_str!("../../fixtures/binance/usdm_open_orders.json"),
            ),
        ] {
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .and(wiremock::matchers::path(path))
                .and(wiremock::matchers::query_param(
                    "timestamp",
                    "1700000000000",
                ))
                .and(wiremock::matchers::query_param("signature", signature))
                .and(wiremock::matchers::header("X-MBX-APIKEY", "key"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(fixture))
                .expect(1)
                .mount(&server)
                .await;
        }
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(BINANCE_COMMISSION_RATE_PATH))
            .and(wiremock::matchers::query_param("symbol", "BTCUSDT"))
            .and(wiremock::matchers::query_param(
                "timestamp",
                "1700000000000",
            ))
            .and(wiremock::matchers::query_param("signature", signature))
            .and(wiremock::matchers::header("X-MBX-APIKEY", "key"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(include_str!(
                    "../../fixtures/binance/usdm_account_commission_rate_btcusdt.json"
                )),
            )
            .expect(1)
            .mount(&server)
            .await;

        let http = binance_http();
        let account_query = test_signed_query("");
        assert!(
            balances(&http, &server.uri(), &account_query, "key", Some("USDT"))
                .await
                .expect("official balance response")
                .contains_key("USDT")
        );
        assert_eq!(
            positions(&http, &server.uri(), &account_query, "key", Some("BTCUSDT"),)
                .await
                .expect("official position response")
                .rows
                .len(),
            2
        );
        assert_eq!(
            open_orders(&http, &server.uri(), &account_query, "key")
                .await
                .expect("official open-orders response")
                .len(),
            1
        );
        let fee = commission_rate(
            &http,
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT"),
            "key",
            "BTCUSDT",
            1_700_000_000_000,
        )
        .await
        .expect("official commission-rate response");
        assert_eq!(fee.maker_commission_rate, 0.0002);
        assert_eq!(fee.taker_commission_rate, 0.0004);
        assert_eq!(fee.rpi_commission_rate, 0.00005);
    }

    #[tokio::test]
    async fn account_read_uses_official_v3_path_and_returns_summary() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/fapi/v3/account"))
            .and(wiremock::matchers::query_param(
                "timestamp",
                "1700000000000",
            ))
            .and(wiremock::matchers::header("X-MBX-APIKEY", "key"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_string(include_str!("../../fixtures/binance/usdm_account_v3.json")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let read = account_read(
            &binance_http(),
            &server.uri(),
            &test_signed_query(""),
            "key",
            Some("USDT"),
            1_700_000_000_000,
        )
        .await
        .expect("official account information response");

        assert_eq!(read.balances.len(), 1);
        assert_eq!(read.summaries.len(), 1);
        assert_eq!(read.summaries[0].venue, "binance");
        assert_eq!(read.summaries[0].total_equity_usd, 126.724_692_06);
    }

    #[tokio::test]
    async fn commission_rate_signed_request_uses_official_path() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/fapi/v1/commissionRate"))
            .and(wiremock::matchers::query_param("symbol", "BTCUSDT"))
            .and(wiremock::matchers::query_param(
                "timestamp",
                "1700000000000",
            ))
            .and(wiremock::matchers::query_param(
                "signature",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ))
            .and(wiremock::matchers::header("X-MBX-APIKEY", "key"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(include_str!(
                    "../../fixtures/binance/usdm_account_commission_rate_btcusdt.json"
                )),
            )
            .expect(1)
            .mount(&server)
            .await;

        let fee = commission_rate(
            &binance_http(),
            &server.uri(),
            &test_signed_query("symbol=BTCUSDT"),
            "key",
            "BTCUSDT",
            1_700_000_000_000,
        )
        .await
        .expect("signed commission-rate request");

        assert_eq!(fee.symbol, "BTCUSDT");
        assert_eq!(fee.maker_commission_rate, 0.0002);
    }

    #[tokio::test]
    async fn private_reads_reject_unsigned_or_malformed_auth_before_io() {
        let http = binance_http();
        let base_url = "http://127.0.0.1:1";

        let missing_signature = balances(&http, base_url, "timestamp=1700000000000", "key", None)
            .await
            .expect_err("missing signature must fail before I/O");
        assert!(matches!(missing_signature, ExchangeError::Auth(_)));

        let duplicate_timestamp = balances(
            &http,
            base_url,
            &format!("timestamp=1&{}", test_signed_query("")),
            "key",
            None,
        )
        .await
        .expect_err("duplicate timestamp must fail before I/O");
        assert!(matches!(duplicate_timestamp, ExchangeError::Auth(_)));

        let missing_key = balances(&http, base_url, &test_signed_query(""), " ", None)
            .await
            .expect_err("missing API key must fail before I/O");
        assert!(matches!(missing_key, ExchangeError::Auth(_)));
    }
}
