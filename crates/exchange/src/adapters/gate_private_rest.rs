//! Gate private REST request helpers.

use crate::adapters::funding_payments::{parse_gate_funding_payments, GateAccountBookRow};
use crate::adapters::gate_fee_evidence::{
    parse_futures_fee_evidence, GateFuturesFeeEvidence, GateFuturesFeeResponse,
};
use crate::adapters::gate_fill_evidence::{
    parse_my_trades, GateFuturesFillEvidence, GateMyTradeRow,
};
use crate::adapters::gate_private_data::{
    parse_account_position_mode, parse_account_read, parse_balance_response, AccountItem,
    OpenOrderItem, PositionRow,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::VenueAccountRead;
use reqwest::header::CONTENT_TYPE;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use shared_types::{BalanceInfo, FundingPaymentData};
use std::collections::HashMap;

const BODY_EXCERPT_LIMIT: usize = 240;

pub(super) type SignedHeaders = [(String, String); 3];

pub(super) struct SignedRequest<'a> {
    pub(super) http: &'a HttpClient,
    pub(super) base_url: &'a str,
    pub(super) path: &'a str,
    pub(super) query: &'a str,
    pub(super) headers: &'a SignedHeaders,
}

pub(super) async fn open_order_rows(
    request: &SignedRequest<'_>,
) -> ExchangeResult<Vec<OpenOrderItem>> {
    let resp = signed_get(request).await?;
    json_with_context(resp, "open_orders", request.path).await
}

pub(super) async fn order_row(
    request: &SignedRequest<'_>,
) -> ExchangeResult<Option<OpenOrderItem>> {
    let resp = signed_get(request).await?;
    optional_json_with_context(resp, "order", request.path).await
}

pub(super) async fn balance(
    request: &SignedRequest<'_>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let resp = signed_get(request).await?;
    let item: AccountItem = json_with_context(resp, "balance", request.path).await?;
    parse_balance_response(
        &item,
        currency,
        futures_settle_from_account_path(request.path)?,
    )
}

pub(super) async fn account_read(
    request: &SignedRequest<'_>,
    currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountRead> {
    let resp = signed_get(request).await?;
    let item: AccountItem = json_with_context(resp, "account_read", request.path).await?;
    parse_account_read(
        &item,
        currency,
        futures_settle_from_account_path(request.path)?,
        observed_at_ms,
    )
}

pub(super) async fn account_position_mode(request: &SignedRequest<'_>) -> ExchangeResult<String> {
    let resp = signed_get(request).await?;
    let item: AccountItem = json_with_context(resp, "account_mode", request.path).await?;
    parse_account_position_mode(&item)
}

pub(super) async fn funding_payments(
    request: &SignedRequest<'_>,
    settle_currency: &str,
) -> ExchangeResult<Vec<FundingPaymentData>> {
    let resp = signed_get(request).await?;
    let rows: Vec<GateAccountBookRow> =
        json_with_context(resp, "funding_payments", request.path).await?;
    parse_gate_funding_payments(&rows, settle_currency)
}

pub(super) fn my_trades_query(order_id: &str) -> ExchangeResult<String> {
    let order_id = numeric_order_id(order_id)?;
    Ok(format!("order={order_id}"))
}

pub(super) async fn my_trades(
    request: &SignedRequest<'_>,
    settle: &str,
    order_id: &str,
) -> ExchangeResult<Vec<GateFuturesFillEvidence>> {
    validate_private_read_request(request, settle, "my_trades", &my_trades_query(order_id)?)?;
    let resp = signed_get(request).await?;
    let rows: Vec<GateMyTradeRow> = json_with_context(resp, "my_trades", request.path).await?;
    parse_my_trades(rows, settle, order_id)
}

pub(super) async fn futures_fee_evidence(
    request: &SignedRequest<'_>,
    settle: &str,
    fetched_at_ms: i64,
) -> ExchangeResult<Vec<GateFuturesFeeEvidence>> {
    validate_private_read_request(request, settle, "fee", "")?;
    let resp = signed_get(request).await?;
    let rows: GateFuturesFeeResponse = json_with_context(resp, "fee", request.path).await?;
    parse_futures_fee_evidence(rows, settle, fetched_at_ms)
}

pub(super) async fn position_rows(request: &SignedRequest<'_>) -> ExchangeResult<Vec<PositionRow>> {
    let resp = signed_get(request).await?;
    let rows: Vec<Value> = json_with_context(resp, "positions", request.path).await?;
    rows.into_iter()
        .map(|row| {
            let row = normalize_position_row(row)?;
            serde_json::from_value(row).map_err(|error| {
                ExchangeError::Parse(format!(
                    "gate positions normalized row: path={} error={error}",
                    request.path
                ))
            })
        })
        .collect()
}

fn normalize_position_row(mut row: Value) -> ExchangeResult<Value> {
    let object = row
        .as_object_mut()
        .ok_or_else(|| ExchangeError::Parse("gate position row must be an object".to_owned()))?;
    normalize_maintenance_rate(object)?;
    normalize_leverage_and_margin_mode(object)?;
    Ok(row)
}

fn normalize_maintenance_rate(row: &mut Map<String, Value>) -> ExchangeResult<()> {
    let effective = optional_positive_decimal(
        row.get("average_maintenance_rate"),
        "average_maintenance_rate",
    )?;
    row.insert(
        "maintenance_rate".to_owned(),
        Value::String(effective.map_or_else(String::new, decimal_text)),
    );
    Ok(())
}

fn normalize_leverage_and_margin_mode(row: &mut Map<String, Value>) -> ExchangeResult<()> {
    let current = optional_positive_decimal(row.get("lever"), "lever")?;
    let margin_mode = optional_text(row.get("pos_margin_mode"), "pos_margin_mode")?;
    match margin_mode.as_deref() {
        Some("isolated") => {
            let leverage = current.ok_or_else(|| {
                ExchangeError::Parse(
                    "gate isolated position missing positive current leverage in lever".to_owned(),
                )
            })?;
            row.insert("leverage".to_owned(), Value::String(decimal_text(leverage)));
        }
        Some("cross") => {
            // The legacy PositionInfo shape cannot carry cross mode and its
            // independent current leverage simultaneously. Preserve the
            // authoritative mode and leave leverage unknown instead of lying.
            row.insert("leverage".to_owned(), Value::String("0".to_owned()));
        }
        Some(other) => {
            return Err(ExchangeError::Parse(format!(
                "gate position has invalid pos_margin_mode {other:?}"
            )));
        }
        None => {}
    }
    Ok(())
}

fn optional_positive_decimal(value: Option<&Value>, field: &str) -> ExchangeResult<Option<f64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let text = match value {
        Value::String(value) => value.trim(),
        Value::Number(value) => return positive_decimal(value.as_f64(), field),
        Value::Null => return Ok(None),
        _ => {
            return Err(ExchangeError::Parse(format!(
                "gate position {field} must be numeric"
            )));
        }
    };
    if text.is_empty() {
        return Ok(None);
    }
    let parsed = text.parse::<f64>().map_err(|error| {
        ExchangeError::Parse(format!(
            "gate position has invalid {field} {text:?}: {error}"
        ))
    })?;
    positive_decimal(Some(parsed), field)
}

fn positive_decimal(value: Option<f64>, field: &str) -> ExchangeResult<Option<f64>> {
    let Some(value) = value else {
        return Err(ExchangeError::Parse(format!(
            "gate position has non-finite {field}"
        )));
    };
    if !value.is_finite() || value < 0.0 {
        return Err(ExchangeError::Parse(format!(
            "gate position has invalid {field} {value}"
        )));
    }
    Ok((value > 0.0).then_some(value))
}

fn optional_text(value: Option<&Value>, field: &str) -> ExchangeResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::String(value) = value else {
        return Err(ExchangeError::Parse(format!(
            "gate position {field} must be text"
        )));
    };
    Ok((!value.trim().is_empty()).then(|| value.trim().to_ascii_lowercase()))
}

fn decimal_text(value: f64) -> String {
    value.to_string()
}

pub(super) async fn safe_cancel_probe(request: &SignedRequest<'_>) -> ExchangeResult<()> {
    let resp = signed_request(request, Method::DELETE).await?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|error| ExchangeError::Network(format!("gate safe cancel probe body: {error}")))?;
    if status == 404 || is_safe_cancel_nonmatch_status(status, &body) {
        return Ok(());
    }
    if (200..300).contains(&status) {
        return Err(ExchangeError::Api {
            exchange: "gate".into(),
            code: "safe_cancel_collision".into(),
            message:
                "gate safe cancel probe unexpectedly matched an order; no-match evidence rejected"
                    .into(),
        });
    }
    Err(ExchangeError::Http {
        status,
        body: body_excerpt(&body),
    })
}

async fn signed_get(request: &SignedRequest<'_>) -> ExchangeResult<reqwest::Response> {
    signed_request(request, Method::GET).await
}

async fn signed_request(
    request: &SignedRequest<'_>,
    method: Method,
) -> ExchangeResult<reqwest::Response> {
    let url = request.url();
    request
        .http
        .execute_with_retry(|| signed_builder(request, method.clone(), &url))
        .await
}

fn signed_builder(
    request: &SignedRequest<'_>,
    method: Method,
    url: &str,
) -> reqwest::RequestBuilder {
    let mut req = request
        .http
        .request(method, url)
        .header("X-Gate-Size-Decimal", "1");
    for (key, value) in request.headers {
        req = req.header(key, value);
    }
    req
}

async fn json_with_context<T>(
    resp: reqwest::Response,
    operation: &str,
    path: &str,
) -> ExchangeResult<T>
where
    T: DeserializeOwned,
{
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
        .to_owned();
    let body = resp.text().await.map_err(|error| {
        ExchangeError::Parse(format!(
            "gate {operation} body: path={path} status={status} error={error}"
        ))
    })?;
    parse_json_with_context(&body, operation, path, status, &content_type)
}

async fn optional_json_with_context<T>(
    resp: reqwest::Response,
    operation: &str,
    path: &str,
) -> ExchangeResult<Option<T>>
where
    T: DeserializeOwned,
{
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
        .to_owned();
    let body = resp.text().await.map_err(|error| {
        ExchangeError::Parse(format!(
            "gate {operation} body: path={path} status={status} error={error}"
        ))
    })?;
    if status == 404 {
        return Ok(None);
    }
    if status >= 400 {
        return Err(ExchangeError::Http {
            status,
            body: body_excerpt(&body),
        });
    }
    parse_json_with_context(&body, operation, path, status, &content_type).map(Some)
}

fn parse_json_with_context<T>(
    body: &str,
    operation: &str,
    path: &str,
    status: u16,
    content_type: &str,
) -> ExchangeResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(body).map_err(|error| {
        ExchangeError::Parse(format!(
            "gate {operation} json: path={path} status={status} content_type={content_type} error={error}; body={}",
            body_excerpt(body)
        ))
    })
}

fn body_excerpt(body: &str) -> String {
    if body.len() <= BODY_EXCERPT_LIMIT {
        body.to_owned()
    } else {
        body.chars().take(BODY_EXCERPT_LIMIT).collect()
    }
}

fn gate_safe_cancel_nonmatch(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("order_not_found")
        || body.contains("order not found")
        || body.contains("not found")
        || body.contains("not exist")
        || body.contains("not found or finished")
}

fn is_safe_cancel_nonmatch_status(status: u16, body: &str) -> bool {
    (400..500).contains(&status)
        && status != 401
        && status != 403
        && gate_safe_cancel_nonmatch(body)
}

fn futures_settle_from_account_path(path: &str) -> ExchangeResult<&str> {
    let mut parts = path.trim_matches('/').split('/');
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some("api"), Some("v4"), Some("futures"), Some(settle), Some("accounts"))
            if !settle.trim().is_empty() =>
        {
            Ok(settle)
        }
        _ => Err(ExchangeError::Parse(format!(
            "gate balance path missing futures settle evidence: {path}"
        ))),
    }
}

fn numeric_order_id(order_id: &str) -> ExchangeResult<u64> {
    let parsed = order_id.parse::<u64>().map_err(|_| {
        ExchangeError::Parse(format!(
            "gate my_trades order must be a numeric futures order id: {order_id}"
        ))
    })?;
    if parsed == 0 {
        return Err(ExchangeError::Parse(
            "gate my_trades order must be greater than zero".into(),
        ));
    }
    Ok(parsed)
}

fn validate_private_read_request(
    request: &SignedRequest<'_>,
    settle: &str,
    endpoint: &str,
    expected_query: &str,
) -> ExchangeResult<()> {
    let expected_path = format!("/api/v4/futures/{}/{endpoint}", settle.to_ascii_lowercase());
    if request.path != expected_path || request.query != expected_query {
        return Err(ExchangeError::Parse(format!(
            "gate signed private read mismatch: expected {expected_path}?{expected_query}, got {}?{}",
            request.path, request.query
        )));
    }
    Ok(())
}

impl SignedRequest<'_> {
    fn url(&self) -> String {
        if self.query.is_empty() {
            format!("{}{}", self.base_url, self.path)
        } else {
            format!("{}{}?{}", self.base_url, self.path, self.query)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize)]
    struct Row {
        value: i64,
    }

    #[test]
    fn parse_json_context_keeps_private_endpoint_evidence() {
        let err = parse_json_with_context::<Vec<Row>>(
            r#"{"label":"INVALID_CREDENTIALS","message":"bad key"}"#,
            "positions",
            "/api/v4/futures/usdt/positions",
            401,
            "application/json",
        )
        .expect_err("object is not a positions array");
        let text = err.to_string();

        assert!(text.contains("gate positions json"));
        assert!(text.contains("path=/api/v4/futures/usdt/positions"));
        assert!(text.contains("status=401"));
        assert!(text.contains("content_type=application/json"));
        assert!(text.contains("INVALID_CREDENTIALS"));
    }

    #[test]
    fn parse_json_context_accepts_valid_json() {
        let parsed = parse_json_with_context::<Row>(
            r#"{"value":7}"#,
            "balance",
            "/x",
            200,
            "application/json",
        )
        .expect("valid row parses");

        assert_eq!(parsed.value, 7);
    }

    // PR-CT: official Gate Futures get-order HTTP-status finality contract.
    // Gate carries no `{code,data}` envelope; order existence/errors ride on the
    // HTTP status. order_row therefore must map 404 -> not found (None), >=400
    // -> typed Http error, and only parse the body on a real success. The
    // struct/parse tests above cover field mapping but never exercise this
    // status-driven fail-closed boundary end-to-end.
    fn test_headers() -> SignedHeaders {
        [
            ("KEY".to_owned(), "k".to_owned()),
            ("Timestamp".to_owned(), "1700000000".to_owned()),
            ("SIGN".to_owned(), "s".to_owned()),
        ]
    }

    fn gate_http() -> HttpClient {
        HttpClient::builder("gate")
            .timeout_secs(5)
            .max_retries(1)
            .build()
            .expect("http client")
    }

    fn gate_rate_limit_http() -> HttpClient {
        HttpClient::builder("gate_pr_fb_rate_limit")
            .timeout_secs(5)
            .max_retries(1)
            .build()
            .expect("rate-limit http client")
    }

    #[tokio::test]
    async fn account_position_mode_reads_futures_account_evidence() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/accounts"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"currency":"USDT","total":"100","available":"80","position_margin":"12","order_margin":"3","unrealised_pnl":"1","position_mode":"dual"}"#,
            ))
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/accounts",
            query: "",
            headers: &headers,
        };

        assert_eq!(
            account_position_mode(&request)
                .await
                .expect("Gate account position_mode parses"),
            "dual"
        );
    }

    #[tokio::test]
    async fn account_position_mode_rejects_unknown_semantics() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/accounts"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"currency":"USDT","total":"100","available":"80","position_margin":"12","order_margin":"3","unrealised_pnl":"1","position_mode":"portfolio"}"#,
            ))
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/accounts",
            query: "",
            headers: &headers,
        };

        assert!(account_position_mode(&request).await.is_err());
    }

    #[tokio::test]
    async fn order_row_returns_none_on_404() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/orders/777"))
            .respond_with(
                wiremock::ResponseTemplate::new(404)
                    .set_body_string(r#"{"label":"ORDER_NOT_FOUND"}"#),
            )
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/orders/777",
            query: "",
            headers: &headers,
        };
        let found = order_row(&request).await.expect("404 is a clean not-found");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn order_row_surfaces_http_error_on_server_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/orders/777"))
            .respond_with(
                wiremock::ResponseTemplate::new(500).set_body_string(r#"{"label":"SERVER_ERROR"}"#),
            )
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/orders/777",
            query: "",
            headers: &headers,
        };
        let err = order_row(&request)
            .await
            .expect_err("5xx must fail-closed, not be treated as not-found");
        assert!(matches!(err, ExchangeError::Http { status: 500, .. }));
    }

    #[tokio::test]
    async fn order_row_parses_official_success_envelope() {
        let server = wiremock::MockServer::start().await;
        let body = r#"{"id":777,"contract":"BTC_USDT","status":"finished","finish_as":"filled","size":1,"left":0,"price":"30000","fill_price":"30000","tif":"gtc","create_time_ms":1700028800123}"#;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/orders/777"))
            .and(wiremock::matchers::header("X-Gate-Size-Decimal", "1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/orders/777",
            query: "",
            headers: &headers,
        };
        let row = order_row(&request)
            .await
            .expect("200 yields a row")
            .expect("order present");
        let parsed =
            crate::adapters::gate_private_data::parse_open_order(&row).expect("order parses");
        assert_eq!(parsed.order_id, "777");
        assert!(matches!(parsed.status, shared_types::OrderStatus::Filled));
        assert!(matches!(parsed.side, shared_types::OrderSide::Buy));
        assert_eq!(parsed.filled_price, 30000.0);
    }

    #[tokio::test]
    async fn order_row_fails_closed_on_invalid_body() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/orders/777"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("not-json"))
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/orders/777",
            query: "",
            headers: &headers,
        };
        assert!(order_row(&request).await.is_err());
    }

    #[tokio::test]
    async fn balance_uses_endpoint_settle_for_blank_currency() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/accounts"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"currency":"","total":"100","available":"80","position_margin":"12","order_margin":"3","unrealised_pnl":"1"}"#,
            ))
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/accounts",
            query: "",
            headers: &headers,
        };

        let rows = balance(&request, Some("USDT"))
            .await
            .expect("endpoint settle backs blank currency");
        assert_eq!(rows.get("USDT").map(|row| row.available), Some(80.0));
    }

    #[tokio::test]
    async fn account_read_reuses_balance_response_for_equity_summary() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/accounts"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"currency":"USDT","total":"100","available":"80","position_margin":"12","order_margin":"3","unrealised_pnl":"2","cross_maintenance_margin":"1","position_mode":"single"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/accounts",
            query: "",
            headers: &headers,
        };

        let read = account_read(&request, None, 1_700_000_000_000)
            .await
            .expect("account read");
        assert_eq!(read.balances.len(), 1);
        assert_eq!(read.summaries[0].total_equity_usd, 102.0);
        assert_eq!(read.summaries[0].total_initial_margin_usd, 15.0);
        assert_eq!(read.summaries[0].total_maintenance_margin_usd, 1.0);
    }

    #[test]
    fn futures_settle_from_account_path_is_fail_closed() {
        assert_eq!(
            futures_settle_from_account_path("/api/v4/futures/usdt/accounts").expect("settle"),
            "usdt"
        );
        assert!(futures_settle_from_account_path("/api/v4/futures/accounts").is_err());
    }

    #[test]
    fn rest_position_prefers_average_maintenance_and_current_isolated_leverage() {
        let position = parsed_position(serde_json::json!({
            "contract": "BTC_USDT",
            "size": "2",
            "entry_price": "30000",
            "mark_price": "30100",
            "unrealised_pnl": "1",
            "margin": "100",
            "liq_price": "25000",
            "mode": "single",
            "maintenance_rate": "0.009",
            "average_maintenance_rate": "0.0042",
            "leverage": "3",
            "cross_leverage_limit": "7",
            "lever": "11",
            "pos_margin_mode": "isolated"
        }));

        assert_eq!(position.maintenance_margin_ratio, 0.0042);
        assert_eq!(position.leverage, 11.0);
        assert_eq!(position.margin_mode.as_deref(), Some("isolated"));
    }

    #[test]
    fn rest_position_missing_effective_maintenance_and_cross_leverage_stay_unknown() {
        let position = parsed_position(serde_json::json!({
            "contract": "BTC_USDT",
            "size": "2",
            "entry_price": "30000",
            "mark_price": "30100",
            "unrealised_pnl": "1",
            "margin": "100",
            "liq_price": "25000",
            "mode": "single",
            "maintenance_rate": "0.009",
            "leverage": "3",
            "cross_leverage_limit": "7",
            "lever": "7",
            "pos_margin_mode": "cross"
        }));

        assert_eq!(position.maintenance_margin_ratio, 0.0);
        assert_eq!(position.leverage, 0.0);
        assert_eq!(position.margin_mode.as_deref(), Some("cross"));
    }

    #[test]
    fn rest_position_rejects_invalid_authoritative_semantics() {
        let invalid_maintenance = normalize_position_row(serde_json::json!({
            "average_maintenance_rate": "bad"
        }));
        let invalid_mode = normalize_position_row(serde_json::json!({
            "average_maintenance_rate": "0.01",
            "lever": "5",
            "pos_margin_mode": "portfolio"
        }));

        assert!(invalid_maintenance.is_err());
        assert!(invalid_mode.is_err());
    }

    #[tokio::test]
    async fn gate_positions_http_fixture_maps_current_risk_semantics() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/positions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(include_str!(
                    "../../fixtures/gate/futures_usdt_positions_v4_106_106.json"
                )),
            )
            .mount(&server)
            .await;
        let http = gate_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/positions",
            query: "",
            headers: &headers,
        };

        let rows = position_rows(&request).await.expect("position rows");
        let position =
            crate::adapters::gate_private_data::parse_positions(&rows, None, |_| Ok(0.0001))
                .expect("positions parse")
                .pop()
                .expect("open position");

        assert_eq!(position.side, "short");
        assert_eq!(position.position_mode.as_deref(), Some("single"));
        assert_eq!(position.margin_mode.as_deref(), Some("isolated"));
        assert_eq!(position.leverage, 30.0);
        assert_eq!(position.maintenance_margin_ratio, 0.005);
        assert_eq!(position.liquidation_price, Some(99_999_999.0));
        assert!(position
            .liquidation_distance_pct
            .is_some_and(f64::is_finite));
    }

    #[tokio::test]
    async fn gate_positions_http_rate_limit_uses_reset_timestamp() {
        let server = wiremock::MockServer::start().await;
        let reset_at_ms = (chrono::Utc::now().timestamp_millis() + 1_000).to_string();
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v4/futures/usdt/positions"))
            .respond_with(
                wiremock::ResponseTemplate::new(429)
                    .insert_header("x-gate-ratelimit-reset-timestamp", reset_at_ms.as_str())
                    .set_body_string(include_str!(
                        "../../fixtures/gate/futures_usdt_positions_rate_limited.json"
                    )),
            )
            .mount(&server)
            .await;
        let http = gate_rate_limit_http();
        let headers = test_headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v4/futures/usdt/positions",
            query: "",
            headers: &headers,
        };

        let error = position_rows(&request)
            .await
            .expect_err("429 must stay typed");

        assert!(matches!(
            error,
            ExchangeError::RateLimited {
                retry_after_secs: 1
            }
        ));
    }

    fn parsed_position(value: Value) -> shared_types::PositionInfo {
        let normalized = normalize_position_row(value).expect("position semantics normalize");
        let row: PositionRow = serde_json::from_value(normalized).expect("position row decodes");
        crate::adapters::gate_private_data::parse_positions(&[row], None, |_| Ok(1.0))
            .expect("position parses")
            .pop()
            .expect("open position")
    }
}

#[cfg(test)]
#[path = "gate_private_rest_fill_fee_tests.rs"]
mod fill_fee_tests;
