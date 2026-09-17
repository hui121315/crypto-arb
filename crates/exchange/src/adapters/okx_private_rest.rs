//! OKX private REST request helpers.

use crate::adapters::funding_payments::{parse_okx_funding_payments, OkxBillRow};
use crate::adapters::okx_private_data::{
    parse_balance_response, parse_open_orders, parse_positions, AccountBalanceItem, OpenOrderItem,
    PositionRow,
};
use crate::adapters::okx_response::{parse_err, OkxResponse};
use crate::adapters::okx_trade_data::{parse_position_mode, AccountConfigRow, OkxPositionMode};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;
use shared_types::{BalanceInfo, FundingPaymentData, OrderInfo, PositionInfo};
use std::collections::HashMap;

pub(super) type SignedHeaders = [(String, String); 4];

pub(super) struct SignedRequest<'a> {
    pub(super) http: &'a HttpClient,
    pub(super) base_url: &'a str,
    pub(super) path: &'a str,
    pub(super) headers: &'a SignedHeaders,
}

pub(super) async fn balances(
    request: &SignedRequest<'_>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let wrap: OkxResponse<AccountBalanceItem> = signed_get_json(request).await?;
    parse_balance_response(wrap.into_data("balance")?, currency)
}

pub(super) async fn funding_payments(
    request: &SignedRequest<'_>,
) -> ExchangeResult<Vec<FundingPaymentData>> {
    let wrap: OkxResponse<OkxBillRow> = signed_get_json(request).await?;
    parse_okx_funding_payments(wrap.into_data("funding payments")?)
}

pub(super) async fn positions(
    request: &SignedRequest<'_>,
    target: Option<&str>,
) -> ExchangeResult<Vec<PositionInfo>> {
    let wrap: OkxResponse<PositionRow> = signed_get_json(request).await?;
    let items = wrap.into_data("positions")?;
    parse_positions(&items, target)
}

pub(super) async fn open_orders(request: &SignedRequest<'_>) -> ExchangeResult<Vec<OrderInfo>> {
    let wrap: OkxResponse<OpenOrderItem> = signed_get_json(request).await?;
    parse_open_orders(wrap.into_data("orders-pending")?)
}

pub(super) async fn account_position_mode(
    request: &SignedRequest<'_>,
) -> ExchangeResult<OkxPositionMode> {
    let wrap: OkxResponse<AccountConfigRow> = signed_get_json(request).await?;
    let mut rows = wrap.into_data("account config")?;
    let row = rows
        .pop()
        .ok_or_else(|| ExchangeError::Parse("okx account config empty".into()))?;
    parse_position_mode(&row)
}

pub(super) async fn pre_check_order(
    request: &SignedRequest<'_>,
    body: String,
) -> ExchangeResult<()> {
    let wrap: OkxResponse<serde_json::Value> =
        signed_json(request, Method::POST, Some(body)).await?;
    wrap.into_data("order pre-check")?;
    Ok(())
}

async fn signed_get_json<T: serde::de::DeserializeOwned>(
    request: &SignedRequest<'_>,
) -> ExchangeResult<T> {
    signed_json(request, Method::GET, None).await
}

async fn signed_json<T: serde::de::DeserializeOwned>(
    request: &SignedRequest<'_>,
    method: Method,
    body: Option<String>,
) -> ExchangeResult<T> {
    let url = format!("{}{}", request.base_url, request.path);
    let resp = request
        .http
        .execute_with_retry(|| signed_builder(request, method.clone(), &url, body.clone()))
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

fn signed_builder(
    request: &SignedRequest<'_>,
    method: Method,
    url: &str,
    body: Option<String>,
) -> reqwest::RequestBuilder {
    let mut req = request.http.request(method, url);
    for (key, value) in request.headers {
        req = req.header(key, value);
    }
    if let Some(body) = body {
        req = req.header("Content-Type", "application/json").body(body);
    }
    req
}

#[cfg(test)]
mod tests {
    use super::*;

    fn okx_http() -> HttpClient {
        HttpClient::builder("okx")
            .timeout_secs(5)
            .build()
            .expect("http client")
    }

    fn headers() -> SignedHeaders {
        [
            ("OK-ACCESS-KEY".into(), "key".into()),
            ("OK-ACCESS-SIGN".into(), "sig".into()),
            (
                "OK-ACCESS-TIMESTAMP".into(),
                "2026-07-03T00:00:00.000Z".into(),
            ),
            ("OK-ACCESS-PASSPHRASE".into(), "pass".into()),
        ]
    }

    #[tokio::test]
    async fn pre_check_order_uses_official_pre_check_endpoint_and_body() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/v5/trade/order-precheck"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "instId": "BTC-USDT-SWAP",
                "tdMode": "cross",
                "ordType": "limit",
                "clOrdId": "xlineprecheck1"
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"code":"0","msg":"","data":[{"acctStpMode":"cancel_maker"}]}"#,
            ))
            .mount(&server)
            .await;
        let http = okx_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v5/trade/order-precheck",
            headers: &headers,
        };

        pre_check_order(
            &request,
            r#"{"instId":"BTC-USDT-SWAP","tdMode":"cross","side":"buy","posSide":"net","ordType":"limit","sz":"1","clOrdId":"xlineprecheck1","px":"100000"}"#.to_owned(),
        )
        .await
        .expect("official OKX order-precheck endpoint accepts a signed create-order shaped body");
    }
}
