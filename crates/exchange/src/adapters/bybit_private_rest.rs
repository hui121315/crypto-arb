//! Bybit private REST request helpers.

use super::bybit_credential_data::{BybitAccountInfo, BybitApiKeyInfo};
use super::bybit_private_data::{
    order_position_idx_rows, parse_account_response, parse_balance_response, parse_open_order,
    parse_open_orders, parse_position_mode_rows, parse_positions, OpenOrderRow,
    PositionModeEvidence, PositionModeRow, PositionRow, WalletAccount,
};
use super::bybit_response::{parse_err, BybitObjectResponse, BybitResponse};
use super::funding_payments::{
    parse_bybit_funding_payments, BybitTransactionLogPage, FundingPaymentPage,
    BYBIT_TRANSACTION_LOG_PATH,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{venue_balance_rows, VenueAccountRead};
use reqwest::Method;
use shared_types::{BalanceInfo, OrderInfo, OrderSide, PositionInfo};
use std::collections::HashMap;

pub(super) type SignedHeaders = [(String, String); 4];
const SIGNED_GET_BODY_ATTEMPTS: usize = 2;

pub(super) async fn post_signed<T: serde::de::DeserializeOwned>(
    http: &HttpClient,
    base_url: &str,
    path: &str,
    body: String,
    headers: &SignedHeaders,
    context: &str,
) -> ExchangeResult<T> {
    let url = format!("{base_url}{path}");
    // 非幂等写：不重放（headers 内含一次性签名时间戳），结果由读侧对账。
    let resp = http
        .execute_once(|| {
            let mut req = http
                .request(Method::POST, &url)
                .header("Content-Type", "application/json")
                .body(body.clone());
            for (key, value) in headers {
                req = req.header(key, value);
            }
            req
        })
        .await?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if !status.is_success() {
        return Err(ExchangeError::Http {
            status: status.as_u16(),
            body: text,
        });
    }
    let wrap: BybitObjectResponse<T> = serde_json::from_str(&text)
        .map_err(|error| ExchangeError::Parse(format!("bybit {context}: {error}: {text}")))?;
    wrap.into_result(context)
}

pub(super) async fn pre_check_order(
    http: &HttpClient,
    base_url: &str,
    body: String,
    headers: &SignedHeaders,
) -> ExchangeResult<()> {
    let _: serde_json::Value = post_signed(
        http,
        base_url,
        "/v5/order/pre-check",
        body,
        headers,
        "pre-check order",
    )
    .await?;
    Ok(())
}

pub(super) async fn account_info(
    http: &HttpClient,
    base_url: &str,
    headers: &SignedHeaders,
) -> ExchangeResult<BybitAccountInfo> {
    let url = signed_url(base_url, "/v5/account/info", "");
    let wrap = signed_get_json::<BybitObjectResponse<BybitAccountInfo>>(
        http,
        &url,
        headers,
        "bybit account info",
    )
    .await?;
    wrap.into_result("account info")
}

pub(super) async fn api_key_info(
    http: &HttpClient,
    base_url: &str,
    headers: &SignedHeaders,
) -> ExchangeResult<BybitApiKeyInfo> {
    let url = signed_url(base_url, "/v5/user/query-api", "");
    let wrap = signed_get_json::<BybitObjectResponse<BybitApiKeyInfo>>(
        http,
        &url,
        headers,
        "bybit api key info",
    )
    .await?;
    wrap.into_result("api key info")
}

pub(super) async fn get_order(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
) -> ExchangeResult<Option<OrderInfo>> {
    let url = signed_url(base_url, "/v5/order/realtime", query);
    let resp =
        signed_get_json::<BybitResponse<OpenOrderRow>>(http, &url, headers, "bybit get order")
            .await?;
    let mut rows = resp.into_list("get order")?;
    rows.pop().map(parse_open_order).transpose()
}

pub(super) async fn balances(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let url = signed_url(base_url, "/v5/account/wallet-balance", query);
    let wrap =
        signed_get_json::<BybitResponse<WalletAccount>>(http, &url, headers, "bybit balance")
            .await?;
    parse_balance_response(wrap.into_list("wallet-balance")?, currency)
}

pub(super) async fn account_read(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
    currency: Option<&str>,
) -> ExchangeResult<VenueAccountRead> {
    let url = signed_url(base_url, "/v5/account/wallet-balance", query);
    let wrap =
        signed_get_json::<BybitResponse<WalletAccount>>(http, &url, headers, "bybit account read")
            .await?;
    let (accounts, observed_at_ms) = wrap.into_list_with_time("wallet-balance")?;
    let read = parse_account_response(accounts, currency, observed_at_ms)?;
    Ok(VenueAccountRead {
        balances: venue_balance_rows("bybit", read.balances),
        summaries: vec![read.summary],
        asset_valuations: Vec::new(),
        issues: Vec::new(),
    })
}

pub(super) async fn funding_payments(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
) -> ExchangeResult<FundingPaymentPage> {
    let url = signed_url(base_url, BYBIT_TRANSACTION_LOG_PATH, query);
    let wrap = signed_get_json::<BybitObjectResponse<BybitTransactionLogPage>>(
        http,
        &url,
        headers,
        "bybit funding payments",
    )
    .await?;
    parse_bybit_funding_payments(wrap.into_result("transaction-log")?)
}

pub(super) async fn positions(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
) -> ExchangeResult<Vec<PositionInfo>> {
    let url = signed_url(base_url, "/v5/position/list", query);
    let wrap =
        signed_get_json::<BybitResponse<PositionRow>>(http, &url, headers, "bybit positions")
            .await?;
    let rows = wrap.into_list("positions")?;
    parse_positions(&rows)
}

pub(super) async fn position_mode(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
) -> ExchangeResult<PositionModeEvidence> {
    let url = signed_url(base_url, "/v5/position/list", query);
    let wrap = signed_get_json::<BybitResponse<PositionModeRow>>(
        http,
        &url,
        headers,
        "bybit position mode",
    )
    .await?;
    let rows = wrap.into_list("position mode")?;
    parse_position_mode_rows(&rows)
}

pub(super) async fn position_idx_for_order(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
    side: OrderSide,
    reduce_only: bool,
) -> ExchangeResult<u8> {
    let url = signed_url(base_url, "/v5/position/list", query);
    let wrap = signed_get_json::<BybitResponse<PositionModeRow>>(
        http,
        &url,
        headers,
        "bybit position mode",
    )
    .await?;
    let rows = wrap.into_list("position mode")?;
    order_position_idx_rows(&rows, side, reduce_only)
}

pub(super) async fn open_orders(
    http: &HttpClient,
    base_url: &str,
    query: &str,
    headers: &SignedHeaders,
) -> ExchangeResult<Vec<OrderInfo>> {
    let url = signed_url(base_url, "/v5/order/realtime", query);
    let wrap =
        signed_get_json::<BybitResponse<OpenOrderRow>>(http, &url, headers, "bybit open orders")
            .await?;
    let rows = wrap.into_list("orders")?;
    parse_open_orders(rows)
}

async fn signed_get_json<T: serde::de::DeserializeOwned>(
    http: &HttpClient,
    url: &str,
    headers: &SignedHeaders,
    _context: &str,
) -> ExchangeResult<T> {
    for attempt in 0..SIGNED_GET_BODY_ATTEMPTS {
        let resp = http
            .execute_with_retry(|| signed_get(http, url, headers))
            .await?;
        match resp.bytes().await {
            Ok(body) => {
                return serde_json::from_slice(&body)
                    .map_err(|error| ExchangeError::Parse(format!("bybit json: {error}")));
            }
            Err(error)
                if retryable_signed_get_body_error(&error)
                    && attempt + 1 < SIGNED_GET_BODY_ATTEMPTS =>
            {
                continue;
            }
            Err(error) => return Err(parse_err(&error)),
        }
    }
    Err(ExchangeError::Network(
        "bybit signed GET response body attempts exhausted".into(),
    ))
}

fn retryable_signed_get_body_error(error: &reqwest::Error) -> bool {
    error.is_body() || error.is_decode() || error.is_timeout()
}

fn signed_get(http: &HttpClient, url: &str, headers: &SignedHeaders) -> reqwest::RequestBuilder {
    let mut req = http.request(Method::GET, url);
    for (key, value) in headers {
        req = req.header(key, value);
    }
    req
}

fn signed_url(base_url: &str, path: &str, query: &str) -> String {
    if query.is_empty() {
        format!("{base_url}{path}")
    } else {
        format!("{base_url}{path}?{query}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn bybit_http() -> HttpClient {
        HttpClient::builder("bybit")
            .timeout_secs(5)
            .build()
            .expect("http client")
    }

    fn headers() -> SignedHeaders {
        [
            ("X-BAPI-API-KEY".into(), "key".into()),
            ("X-BAPI-TIMESTAMP".into(), "1700000000000".into()),
            ("X-BAPI-RECV-WINDOW".into(), "5000".into()),
            ("X-BAPI-SIGN".into(), "sig".into()),
        ]
    }

    #[tokio::test]
    async fn pre_check_order_uses_official_pre_check_endpoint() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v5/order/pre-check"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"retCode":0,"retMsg":"OK","result":{"orderId":"pre","orderLinkId":"xline-precheck"}}"#,
            ))
            .mount(&server)
            .await;
        let http = bybit_http();
        let headers = headers();

        pre_check_order(
            &http,
            &server.uri(),
            r#"{"category":"linear","symbol":"BTCUSDT"}"#.to_owned(),
            &headers,
        )
        .await
        .expect("official Bybit pre-check endpoint accepts a signed create-order shaped body");
    }

    #[tokio::test]
    async fn account_and_api_key_info_use_official_read_only_endpoints() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/v5/account/info"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(format!(
                    r#"{{"retCode":0,"retMsg":"OK","result":{}}}"#,
                    include_str!("../../fixtures/bybit/account_info_uta2.json")
                )),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/v5/user/query-api"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(format!(
                    r#"{{"retCode":0,"retMsg":"OK","result":{}}}"#,
                    include_str!("../../fixtures/bybit/api_key_info_order.json")
                )),
            )
            .mount(&server)
            .await;
        let http = bybit_http();
        let headers = headers();

        account_info(&http, &server.uri(), &headers)
            .await
            .expect("official account info endpoint");
        api_key_info(&http, &server.uri(), &headers)
            .await
            .expect("official api key info endpoint")
            .validate_linear_order_permission()
            .expect("official contract order permission");
    }

    #[tokio::test]
    async fn signed_get_retries_a_truncated_response_body_once() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = listener.local_addr().expect("test listener address");
        let server = tokio::spawn(async move {
            for body in [r#"{}"#, r#"{"ok":true}"#] {
                let (mut socket, _) = listener.accept().await.expect("accept test request");
                let mut request = [0_u8; 2_048];
                let _ = socket.read(&mut request).await.expect("read test request");
                let declared_length = if body == "{}" { 64 } else { body.len() };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {declared_length}\r\nconnection: close\r\n\r\n{body}"
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write test response");
            }
        });
        let http = bybit_http();

        let value: serde_json::Value = signed_get_json(
            &http,
            &format!("http://{address}/v5/test"),
            &headers(),
            "truncated body test",
        )
        .await
        .expect("second complete response");

        server.await.expect("test server completed");
        assert_eq!(value, serde_json::json!({"ok": true}));
    }
}
