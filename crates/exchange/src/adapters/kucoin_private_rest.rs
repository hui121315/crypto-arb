//! KuCoin private REST request helpers.
//!
//! Authentication and endpoint contracts:
//! - <https://www.kucoin.com/docs-new/authentication>
//! - <https://www.kucoin.com/docs-new/rest/futures-trading/orders/cancel-order-by-orderld>
//! - <https://www.kucoin.com/docs-new/rest/futures-trading/orders/cancel-order-by-clientoid>
//! - <https://www.kucoin.com/docs-new/rest/futures-trading/orders/get-trade-history>
//! - <https://www.kucoin.com/docs-new/rest/account-info/trade-fee/get-actual-fee-futures>

use crate::adapters::funding_payments::{
    parse_kucoin_funding_payments, FundingPaymentPage, KucoinFundingHistoryPage,
};
use crate::adapters::kucoin_private_data::{
    parse_account_read, parse_balance_response, parse_fee_rate, parse_fill_page, parse_open_orders,
    parse_order_with_fills, parse_position_mode, parse_positions_with_native, AccountOverview,
    FeeRateEvidence, FeeRateRow, FillEvidence, FillPage, KucoinPositionMode, NativePositionInfo,
    OpenOrderItem, PaginatedOrders, PositionModeRow, PositionRow,
};
use crate::adapters::kucoin_response::{parse_err, KucoinResponse};
use crate::adapters::kucoin_trade_data::{
    ack_from_cancel_row, ack_from_order_row, KucoinCancelRow, KucoinOrderAckRow,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::VenueAccountRead;
use reqwest::Method;
use shared_types::{BalanceInfo, CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo};
use std::collections::HashMap;

pub(super) type SignedHeaders = [(String, String); 5];

pub(super) const KUCOIN_AUTH_DOC_URL: &str = "https://www.kucoin.com/docs-new/authentication";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CancelIdentity {
    ExchangeOrderId,
    ClientOrderId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrivateRequestTarget {
    pub(super) method: Method,
    pub(super) signing_path: String,
    pub(super) wire_path: String,
    pub(super) body: String,
    pub(super) source_url: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CancelRequestTarget {
    pub(super) identity: CancelIdentity,
    pub(super) request: PrivateRequestTarget,
}

pub(super) struct SignedRequest<'a> {
    pub(super) http: &'a HttpClient,
    pub(super) base_url: &'a str,
    pub(super) path: &'a str,
    pub(super) headers: &'a SignedHeaders,
}

pub(super) async fn place_order(
    request: &SignedRequest<'_>,
    body: String,
    internal_order_id: String,
    client_order_id: String,
) -> ExchangeResult<OrderAck> {
    let wrap: KucoinResponse<KucoinOrderAckRow> =
        signed_write_json(request, Method::POST, Some(body)).await?;
    Ok(ack_from_order_row(
        internal_order_id,
        client_order_id,
        wrap.into_data("place order")?,
        LiveOrderState::Accepted,
        None,
    ))
}

pub(super) async fn test_order(request: &SignedRequest<'_>, body: String) -> ExchangeResult<()> {
    let wrap: KucoinResponse<serde_json::Value> =
        signed_write_json(request, Method::POST, Some(body)).await?;
    let _ = wrap.into_data("test order")?;
    Ok(())
}

pub(super) async fn safe_cancel_probe(signed: &SignedRequest<'_>) -> ExchangeResult<()> {
    let wrap: KucoinResponse<serde_json::Value> =
        signed_write_json(signed, Method::DELETE, None).await?;
    match wrap.into_data("safe cancel probe") {
        Ok(_) => Err(ExchangeError::Api {
            exchange: "kucoin".into(),
            code: "safe_cancel_collision".into(),
            message:
                "kucoin safe cancel probe unexpectedly matched an order; no-match evidence rejected"
                    .into(),
        }),
        Err(error) if kucoin_safe_cancel_nonmatch(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) async fn cancel_order(
    signed: &SignedRequest<'_>,
    request: &CancelOrderRequest,
) -> ExchangeResult<OrderAck> {
    let wrap: KucoinResponse<KucoinCancelRow> =
        signed_write_json(signed, Method::DELETE, None).await?;
    Ok(ack_from_cancel_row(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        request.exchange_order_id.clone(),
        wrap.into_data("cancel order")?,
    ))
}

pub(super) async fn get_order_row(request: &SignedRequest<'_>) -> ExchangeResult<OpenOrderItem> {
    let wrap: KucoinResponse<OpenOrderItem> = signed_json(request, Method::GET, None).await?;
    wrap.into_data("get order")
}

pub(super) fn fill_lookup_order_id(order: &OpenOrderItem) -> Option<&str> {
    (order.filled_size > 0.0).then_some(order.id.as_str())
}

pub(super) fn order_with_fills(
    order: &OpenOrderItem,
    fills: &[FillEvidence],
) -> ExchangeResult<OrderInfo> {
    parse_order_with_fills(order, fills)
}

pub(super) fn ensure_order_client_oid(
    order: &OpenOrderItem,
    expected_client_oid: &str,
) -> ExchangeResult<()> {
    if order.client_oid == expected_client_oid {
        Ok(())
    } else {
        Err(ExchangeError::Parse(format!(
            "kucoin get order clientOid mismatch: requested={expected_client_oid:?} response={:?}",
            order.client_oid
        )))
    }
}

pub(super) fn is_ambiguous_place_result(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::Timeout { .. }
            | ExchangeError::Network(_)
            | ExchangeError::RateLimited { .. }
            | ExchangeError::Parse(_)
            | ExchangeError::Http {
                status: 500..=599,
                ..
            }
    )
}

pub(super) async fn balances(
    request: &SignedRequest<'_>,
    currency: &str,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let wrap: KucoinResponse<AccountOverview> = signed_json(request, Method::GET, None).await?;
    let acc = wrap.into_data("account-overview")?;
    parse_balance_response(&acc, currency)
}

pub(super) async fn account_read(
    request: &SignedRequest<'_>,
    currency: &str,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountRead> {
    let wrap: KucoinResponse<AccountOverview> = signed_json(request, Method::GET, None).await?;
    let account = wrap.into_data("account-overview")?;
    parse_account_read(&account, currency, observed_at_ms)
}

pub(super) async fn funding_payments(
    request: &SignedRequest<'_>,
) -> ExchangeResult<FundingPaymentPage> {
    let wrap: KucoinResponse<KucoinFundingHistoryPage> =
        signed_json(request, Method::GET, None).await?;
    parse_kucoin_funding_payments(wrap.into_data("funding-history")?)
}

pub(super) async fn positions(
    request: &SignedRequest<'_>,
    target: Option<&str>,
) -> ExchangeResult<Vec<NativePositionInfo>> {
    let wrap: KucoinResponse<Vec<PositionRow>> = signed_json(request, Method::GET, None).await?;
    let rows = wrap.into_data("positions")?;
    parse_positions_with_native(&rows, target)
}

pub(super) async fn position_mode(
    request: &SignedRequest<'_>,
) -> ExchangeResult<KucoinPositionMode> {
    let wrap: KucoinResponse<PositionModeRow> = signed_json(request, Method::GET, None).await?;
    parse_position_mode(&wrap.into_data("position/getPositionMode")?)
}

pub(super) async fn open_orders(request: &SignedRequest<'_>) -> ExchangeResult<Vec<OrderInfo>> {
    let wrap: KucoinResponse<PaginatedOrders> = signed_json(request, Method::GET, None).await?;
    let page = wrap.into_data("orders")?;
    parse_open_orders(&page)
}

pub(super) async fn fills(
    request: &SignedRequest<'_>,
    order_id: &str,
) -> ExchangeResult<Vec<FillEvidence>> {
    let target = fills_request_target(order_id)?;
    ensure_wire_path(request, &target)?;
    let wrap: KucoinResponse<FillPage> = signed_json(request, Method::GET, None).await?;
    parse_fill_page(&wrap.into_data("fills")?, order_id)
}

pub(super) async fn fee_rate(
    request: &SignedRequest<'_>,
    symbol: &str,
    fetched_at_ms: i64,
) -> ExchangeResult<FeeRateEvidence> {
    let target = fee_rate_request_target(symbol)?;
    ensure_wire_path(request, &target)?;
    let wrap: KucoinResponse<FeeRateRow> = signed_json(request, Method::GET, None).await?;
    parse_fee_rate(&wrap.into_data("trade-fees")?, symbol, fetched_at_ms)
}

pub(super) fn cancel_request_target(
    request: &CancelOrderRequest,
    native_symbol: &str,
) -> ExchangeResult<CancelRequestTarget> {
    if let Some(order_id) = request
        .exchange_order_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let order_id = kucoin_numeric_order_id(order_id)?;
        return Ok(CancelRequestTarget {
            identity: CancelIdentity::ExchangeOrderId,
            request: private_get_or_delete_target(
                Method::DELETE,
                format!("/api/v1/orders/{order_id}"),
                "https://www.kucoin.com/docs-new/rest/futures-trading/orders/cancel-order-by-orderld",
            ),
        });
    }
    let client_oid = required_path_value("clientOid", &request.client_order_id)?;
    let symbol = kucoin_native_symbol(native_symbol)?;
    let signing_path = format!("/api/v1/orders/client-order/{client_oid}?symbol={symbol}");
    let wire_path = format!(
        "/api/v1/orders/client-order/{}?symbol={symbol}",
        encoded(client_oid)
    );
    Ok(CancelRequestTarget {
        identity: CancelIdentity::ClientOrderId,
        request: PrivateRequestTarget {
            method: Method::DELETE,
            signing_path,
            wire_path,
            body: String::new(),
            source_url: "https://www.kucoin.com/docs-new/rest/futures-trading/orders/cancel-order-by-clientoid",
        },
    })
}

pub(super) fn fills_request_target(order_id: &str) -> ExchangeResult<PrivateRequestTarget> {
    let order_id = kucoin_numeric_order_id(order_id)?;
    Ok(private_get_or_delete_target(
        Method::GET,
        format!("/api/v1/fills?orderId={order_id}"),
        crate::adapters::kucoin_private_data::KUCOIN_FILLS_DOC_URL,
    ))
}

pub(super) fn fee_rate_request_target(symbol: &str) -> ExchangeResult<PrivateRequestTarget> {
    let symbol = kucoin_native_symbol(symbol)?;
    Ok(private_get_or_delete_target(
        Method::GET,
        format!("/api/v1/trade-fees?symbol={symbol}"),
        crate::adapters::kucoin_private_data::KUCOIN_FEE_RATE_DOC_URL,
    ))
}

async fn signed_json<T: serde::de::DeserializeOwned>(
    request: &SignedRequest<'_>,
    method: Method,
    body: Option<String>,
) -> ExchangeResult<T> {
    validate_signed_request(request, &method, body.as_deref())?;
    let url = request.url();
    let resp = request
        .http
        .execute_with_retry(|| signed_builder(request, method.clone(), &url, body.clone()))
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

pub(super) async fn signed_get<T: serde::de::DeserializeOwned>(
    request: &SignedRequest<'_>,
) -> ExchangeResult<T> {
    signed_json(request, Method::GET, None).await
}

async fn signed_write_json<T: serde::de::DeserializeOwned>(
    request: &SignedRequest<'_>,
    method: Method,
    body: Option<String>,
) -> ExchangeResult<T> {
    validate_signed_request(request, &method, body.as_deref())?;
    if !matches!(method, Method::POST | Method::DELETE) {
        return Err(kucoin_request_error(format!(
            "single-attempt private write requires POST or DELETE, got {}",
            method.as_str()
        )));
    }
    let url = request.url();
    let resp = request
        .http
        .execute_once(|| signed_builder(request, method.clone(), &url, body.clone()))
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

fn signed_builder(
    request: &SignedRequest<'_>,
    method: Method,
    url: &str,
    body: Option<String>,
) -> reqwest::RequestBuilder {
    let mut req = request
        .http
        .request(method, url)
        .header("Content-Type", "application/json");
    if let Some(body) = body {
        req = req.header("Content-Type", "application/json").body(body);
    }
    for (key, value) in request.headers {
        req = req.header(key, value);
    }
    req
}

impl SignedRequest<'_> {
    fn url(&self) -> String {
        format!("{}{}", self.base_url, self.path)
    }
}

fn validate_signed_request(
    request: &SignedRequest<'_>,
    method: &Method,
    body: Option<&str>,
) -> ExchangeResult<()> {
    if !request.path.starts_with('/') || request.path.contains('#') {
        return Err(kucoin_request_error(format!(
            "private request path must be an absolute API path without fragment: {:?}",
            request.path
        )));
    }
    if request.base_url.ends_with('/') {
        return Err(kucoin_request_error(
            "private request base_url must not end with '/'",
        ));
    }
    if matches!(*method, Method::GET | Method::DELETE) && body.is_some() {
        return Err(kucoin_request_error(format!(
            "{} private request must sign and send an empty body",
            method.as_str()
        )));
    }
    if *method == Method::POST {
        let body = body.ok_or_else(|| {
            kucoin_request_error("POST private request requires the exact signed JSON body")
        })?;
        serde_json::from_str::<serde_json::Value>(body).map_err(|error| {
            kucoin_request_error(format!(
                "POST private request body is not valid JSON: {error}"
            ))
        })?;
    }
    validate_auth_headers(request.headers)
}

fn ensure_wire_path(
    request: &SignedRequest<'_>,
    target: &PrivateRequestTarget,
) -> ExchangeResult<()> {
    if request.path == target.wire_path {
        Ok(())
    } else {
        Err(kucoin_request_error(format!(
            "signed private read path mismatch: expected={:?} actual={:?}",
            target.wire_path, request.path
        )))
    }
}

fn validate_auth_headers(headers: &SignedHeaders) -> ExchangeResult<()> {
    for required in [
        "KC-API-KEY",
        "KC-API-SIGN",
        "KC-API-TIMESTAMP",
        "KC-API-PASSPHRASE",
        "KC-API-KEY-VERSION",
    ] {
        let mut values = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(required))
            .map(|(_, value)| value.trim());
        let value = values
            .next()
            .ok_or_else(|| kucoin_request_error(format!("private request missing {required}")))?;
        if value.is_empty() || values.next().is_some() {
            return Err(kucoin_request_error(format!(
                "private request requires exactly one non-empty {required}"
            )));
        }
        if required == "KC-API-KEY-VERSION" && value != "2" {
            return Err(kucoin_request_error(format!(
                "private request expected KC-API-KEY-VERSION 2, got {value:?}"
            )));
        }
    }
    Ok(())
}

fn private_get_or_delete_target(
    method: Method,
    path: String,
    source_url: &'static str,
) -> PrivateRequestTarget {
    PrivateRequestTarget {
        method,
        signing_path: path.clone(),
        wire_path: path,
        body: String::new(),
        source_url,
    }
}

fn kucoin_numeric_order_id(value: &str) -> ExchangeResult<&str> {
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(value)
    } else {
        Err(kucoin_request_error(format!(
            "exchange order id must be a non-empty decimal KuCoin orderId: {value:?}"
        )))
    }
}

fn kucoin_native_symbol(value: &str) -> ExchangeResult<String> {
    let value = value.trim().to_ascii_uppercase();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        Ok(value)
    } else {
        Err(kucoin_request_error(format!(
            "native symbol must be non-empty ASCII alphanumeric: {value:?}"
        )))
    }
}

fn required_path_value<'a>(field: &str, value: &'a str) -> ExchangeResult<&'a str> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        Err(kucoin_request_error(format!(
            "{field} must be non-empty and contain no control characters"
        )))
    } else {
        Ok(value)
    }
}

fn encoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn kucoin_request_error(message: impl Into<String>) -> ExchangeError {
    let message = message.into();
    ExchangeError::Api {
        exchange: "kucoin".into(),
        code: "private_request_invariant".into(),
        message: format!("{message}; auth contract: {KUCOIN_AUTH_DOC_URL}"),
    }
}

fn kucoin_safe_cancel_nonmatch(error: &crate::error::ExchangeError) -> bool {
    match error {
        crate::error::ExchangeError::Api { message, .. } => {
            let message = message.to_ascii_lowercase();
            message.contains("not exist")
                || message.contains("not found")
                || message.contains("not active")
                || message.contains("already filled")
                || message.contains("previously canceled")
                || message.contains("cannot be canceled")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kucoin_http() -> HttpClient {
        HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client")
    }

    fn headers() -> SignedHeaders {
        [
            ("KC-API-KEY".into(), "key".into()),
            ("KC-API-SIGN".into(), "sig".into()),
            ("KC-API-TIMESTAMP".into(), "1700000000000".into()),
            ("KC-API-PASSPHRASE".into(), "pass".into()),
            ("KC-API-KEY-VERSION".into(), "2".into()),
        ]
    }

    fn cancel_request(
        exchange_order_id: Option<&str>,
        client_order_id: &str,
    ) -> CancelOrderRequest {
        CancelOrderRequest {
            exchange: "kucoin".into(),
            symbol: "BTC".into(),
            internal_order_id: "internal-1".into(),
            exchange_order_id: exchange_order_id.map(str::to_owned),
            client_order_id: client_order_id.into(),
        }
    }

    #[test]
    fn cancel_target_prefers_exchange_order_id_then_client_oid_fallback() {
        let by_order = cancel_request_target(
            &cancel_request(Some("235303670076489728"), "client/ignored"),
            "XBTUSDTM",
        )
        .expect("exchange order-id target");
        assert_eq!(by_order.identity, CancelIdentity::ExchangeOrderId);
        assert_eq!(
            by_order.request.signing_path,
            "/api/v1/orders/235303670076489728"
        );
        assert_eq!(by_order.request.wire_path, by_order.request.signing_path);

        let by_client =
            cancel_request_target(&cancel_request(None, "client/with space"), "XBTUSDTM")
                .expect("clientOid fallback target");
        assert_eq!(by_client.identity, CancelIdentity::ClientOrderId);
        assert_eq!(
            by_client.request.signing_path,
            "/api/v1/orders/client-order/client/with space?symbol=XBTUSDTM"
        );
        assert_eq!(
            by_client.request.wire_path,
            "/api/v1/orders/client-order/client%2Fwith+space?symbol=XBTUSDTM"
        );
    }

    #[test]
    fn private_read_targets_pin_official_paths_and_metadata() {
        let fills = fills_request_target("284486580251463680").expect("fills target");
        assert_eq!(fills.method, Method::GET);
        assert_eq!(fills.wire_path, "/api/v1/fills?orderId=284486580251463680");
        assert_eq!(
            fills.source_url,
            crate::adapters::kucoin_private_data::KUCOIN_FILLS_DOC_URL
        );

        let fee = fee_rate_request_target("xbtusdtm").expect("fee target");
        assert_eq!(fee.wire_path, "/api/v1/trade-fees?symbol=XBTUSDTM");
        assert_eq!(
            fee.source_url,
            crate::adapters::kucoin_private_data::KUCOIN_FEE_RATE_DOC_URL
        );
    }

    #[tokio::test]
    async fn private_request_rejects_missing_auth_header_before_io() {
        let http = kucoin_http();
        let mut headers = headers();
        headers[0] = ("X-NOT-AUTH-HEADER".into(), "value".into());
        let request = SignedRequest {
            http: &http,
            base_url: "http://127.0.0.1:1",
            path: "/api/v1/fills?orderId=284486580251463680",
            headers: &headers,
        };

        let error = fills(&request, "284486580251463680")
            .await
            .expect_err("missing KC-API-KEY must fail before network IO");
        assert!(error.to_string().contains("missing KC-API-KEY"));
        assert!(error.to_string().contains(KUCOIN_AUTH_DOC_URL));
    }

    #[tokio::test]
    async fn fills_read_uses_signed_order_query_and_official_fixture() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v1/fills"))
            .and(wiremock::matchers::query_param(
                "orderId",
                "284486580251463680",
            ))
            .and(wiremock::matchers::header("KC-API-KEY", "key"))
            .and(wiremock::matchers::header(
                "Content-Type",
                "application/json",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_string(include_str!("../../fixtures/kucoin/fills_by_order_id.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let http = kucoin_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v1/fills?orderId=284486580251463680",
            headers: &headers,
        };

        let evidence = fills(&request, "284486580251463680")
            .await
            .expect("signed fill evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].fee, 0.05176506);
    }

    #[tokio::test]
    async fn fee_rate_read_uses_signed_symbol_query_and_official_fixture() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v1/trade-fees"))
            .and(wiremock::matchers::query_param("symbol", "XBTUSDTM"))
            .and(wiremock::matchers::header("KC-API-SIGN", "sig"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(include_str!(
                    "../../fixtures/kucoin/futures_actual_fee_xbtusdtm.json"
                )),
            )
            .expect(1)
            .mount(&server)
            .await;
        let http = kucoin_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v1/trade-fees?symbol=XBTUSDTM",
            headers: &headers,
        };

        let evidence = fee_rate(&request, "XBTUSDTM", 1_700_000_000_000)
            .await
            .expect("signed fee-rate evidence");
        assert_eq!(evidence.maker_fee_rate, 0.0002);
        assert_eq!(evidence.taker_fee_rate, 0.0006);
    }

    #[tokio::test]
    async fn account_read_reuses_overview_for_equity_summary() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v1/account-overview"))
            .and(wiremock::matchers::query_param("currency", "USDT"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"code":"200000","data":{"accountEquity":198.733127406,"unrealisedPNL":0.0,"marginBalance":198.733127406,"positionMargin":3.0,"orderMargin":1.0,"frozenFunds":0.0,"availableBalance":194.733127406,"availableMargin":194.733127406,"currency":"USDT","riskRatio":0.01,"maxWithdrawAmount":194.733127406}}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        let http = kucoin_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v1/account-overview?currency=USDT",
            headers: &headers,
        };

        let read = account_read(&request, "USDT", 1_700_000_000_000)
            .await
            .expect("account read");
        assert_eq!(read.balances.len(), 1);
        assert_eq!(read.summaries[0].total_equity_usd, 198.733127406);
        assert_eq!(read.summaries[0].total_initial_margin_usd, 4.0);
        assert_eq!(
            read.summaries[0].withdrawable_balance_usd,
            Some(194.733127406)
        );
    }

    #[tokio::test]
    async fn test_order_uses_official_non_matching_endpoint() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/v1/orders/test"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"code":"200000","data":{"orderId":"test","clientOid":"xline-test"}}"#,
            ))
            .mount(&server)
            .await;
        let http = kucoin_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v1/orders/test",
            headers: &headers,
        };

        test_order(&request, r#"{"clientOid":"xline-test"}"#.to_owned())
            .await
            .expect("official KuCoin test order endpoint accepts a signed body");
    }

    #[tokio::test]
    async fn safe_cancel_probe_rejects_unexpected_2xx_collision() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path(
                "/api/v1/orders/client-order/xline-cancel-probe",
            ))
            .and(wiremock::matchers::query_param("symbol", "XBTUSDTM"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(
                    r#"{"code":"200000","data":{"clientOid":"xline-cancel-probe"}}"#,
                ),
            )
            .mount(&server)
            .await;
        let http = kucoin_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v1/orders/client-order/xline-cancel-probe?symbol=XBTUSDTM",
            headers: &headers,
        };

        let error = safe_cancel_probe(&request)
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
    async fn safe_cancel_probe_treats_unknown_client_oid_as_nonmatching_evidence() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path(
                "/api/v1/orders/client-order/xline-cancel-probe",
            ))
            .and(wiremock::matchers::query_param("symbol", "XBTUSDTM"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(
                    r#"{"code":"400100","msg":"order does not exist","data":null}"#,
                ),
            )
            .mount(&server)
            .await;
        let http = kucoin_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v1/orders/client-order/xline-cancel-probe?symbol=XBTUSDTM",
            headers: &headers,
        };

        safe_cancel_probe(&request)
            .await
            .expect("unknown clientOid proves the nonmatching probe did not cancel a live order");
    }

    #[test]
    fn ambiguous_place_result_excludes_deterministic_api_rejection() {
        assert!(is_ambiguous_place_result(&ExchangeError::Timeout {
            seconds: 5
        }));
        assert!(is_ambiguous_place_result(&ExchangeError::Http {
            status: 503,
            body: "upstream timeout".into(),
        }));
        assert!(is_ambiguous_place_result(&ExchangeError::Parse(
            "unreadable success envelope".into()
        )));
        assert!(!is_ambiguous_place_result(&ExchangeError::Api {
            exchange: "kucoin".into(),
            code: "300018".into(),
            message: "clientOid parameter repeated".into(),
        }));
    }
}
