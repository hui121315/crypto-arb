//! Bitget V3 / UTA private REST request helpers.
//!
//! Signature scheme is identical to V2 (HMAC-SHA256 of
//! `timestamp + METHOD + requestPath?queryString + body`, base64-encoded) so
//! we reuse `crate::signing::bitget::sign` verbatim. Only the routes and
//! envelopes change:
//! - GET `/api/v3/account/assets` returns `data: { assets: [...] }`.
//! - GET `/api/v3/position/current-position` returns `data: {list: [...]}`.
//! - GET `/api/v3/trade/unfilled-orders` returns `data: {list: [...], cursor}`.
//! - POST `/api/v3/trade/place-order|cancel-order` returns
//!   `data: {orderId, clientOid}`.
//!
//! Official docs: <https://www.bitget.com/api-doc/uta/guide>.

use crate::adapters::bitget_response::{api_error_from_body, BitgetObjectResponse};
use crate::adapters::bitget_uta_private_data::{
    parse_account_balances, parse_account_summary, parse_open_order, parse_open_orders,
    parse_positions, UtaAccountAssetsPayload, UtaListPayload, UtaOrderRow, UtaPositionRow,
};
use crate::adapters::bitget_uta_trade_data::{ack_from_row, UtaOrderAckRow};
use crate::adapters::funding_payments::{
    parse_bitget_funding_payments, BitgetUtaFinancialRecordsPage, FundingPaymentPage,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{venue_balance_rows, VenueAccountRead};
use common::time::now_ms;
use reqwest::Method;
use serde::Deserialize;
use shared_types::{
    BalanceInfo, CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, PositionInfo,
    VenueAssetValuation,
};
use std::collections::HashMap;

pub(super) type SignedHeaders = [(String, String); 5];

pub(super) struct SignedRequest<'a> {
    pub(super) http: &'a HttpClient,
    pub(super) base_url: &'a str,
    pub(super) path: &'a str,
    pub(super) headers: &'a SignedHeaders,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountSettings {
    hold_mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountInfo {
    perm_type: String,
    #[serde(default)]
    permissions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AccountAssetValuationPayload {
    assets: Vec<AccountAssetValuationRow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountAssetValuationRow {
    coin: String,
    usd_value: String,
}

pub(super) async fn place_order(
    request: &SignedRequest<'_>,
    body: String,
    internal_order_id: String,
    client_order_id: String,
) -> ExchangeResult<OrderAck> {
    let row = signed_post_object::<UtaOrderAckRow>(request, body, "place order").await?;
    Ok(ack_from_row(
        internal_order_id,
        client_order_id,
        row,
        LiveOrderState::Accepted,
        None,
    ))
}

pub(super) fn is_unknown_place_result(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::Api { exchange, code, .. }
            if exchange == "bitget" && matches!(code.as_str(), "40010" | "40725" | "45001")
    )
}

pub(super) async fn cancel_order(
    signed: &SignedRequest<'_>,
    body: String,
    request: &CancelOrderRequest,
) -> ExchangeResult<OrderAck> {
    let row = signed_post_object::<UtaOrderAckRow>(signed, body, "cancel order").await?;
    Ok(ack_from_row(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        row,
        LiveOrderState::CancelRequested,
        Some("bitget cancel accepted; final state requires order query".to_owned()),
    ))
}

pub(super) async fn safe_cancel_probe(
    signed: &SignedRequest<'_>,
    body: String,
) -> ExchangeResult<()> {
    match signed_post_object::<serde_json::Value>(signed, body, "safe cancel probe").await {
        Ok(_) => Err(ExchangeError::Api {
            exchange: "bitget".into(),
            code: "safe_cancel_collision".into(),
            message:
                "bitget safe cancel probe unexpectedly matched an order; no-match evidence rejected"
                    .into(),
        }),
        Err(error) if bitget_safe_cancel_nonmatch(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) async fn get_order(request: &SignedRequest<'_>) -> ExchangeResult<Option<OrderInfo>> {
    let wrap: BitgetObjectResponse<UtaOrderRow> = signed_get_json(request).await?;
    wrap.into_option("get order")?
        .map(parse_open_order)
        .transpose()
}

pub(super) async fn balances(
    request: &SignedRequest<'_>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let wrap: BitgetObjectResponse<UtaAccountAssetsPayload> = signed_get_json(request).await?;
    parse_account_balances(wrap.into_result("account assets")?, currency)
}

pub(super) async fn account_read(
    request: &SignedRequest<'_>,
    currency: Option<&str>,
) -> ExchangeResult<VenueAccountRead> {
    let wrap: BitgetObjectResponse<serde_json::Value> = signed_get_json(request).await?;
    let raw_payload = wrap.into_result("account assets")?;
    let payload: UtaAccountAssetsPayload = serde_json::from_value(raw_payload.clone())
        .map_err(|error| ExchangeError::Parse(format!("bitget account assets schema: {error}")))?;
    let valuation_payload: AccountAssetValuationPayload = serde_json::from_value(raw_payload)
        .map_err(|error| {
            ExchangeError::Parse(format!("bitget account usdValue schema: {error}"))
        })?;
    let observed_at_ms = now_ms();
    let summary = parse_account_summary(&payload, observed_at_ms)?;
    let asset_valuations = parse_asset_valuations(valuation_payload, currency, observed_at_ms)?;
    let balances = parse_account_balances(payload, currency)?;
    Ok(VenueAccountRead {
        balances: venue_balance_rows("bitget", balances),
        summaries: vec![summary],
        asset_valuations,
        issues: Vec::new(),
    })
}

fn parse_asset_valuations(
    payload: AccountAssetValuationPayload,
    currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<Vec<VenueAssetValuation>> {
    let mut rows = Vec::with_capacity(payload.assets.len());
    for row in payload.assets {
        let coin = row.coin.trim();
        if coin.is_empty() {
            return Err(ExchangeError::Parse(
                "bitget account usdValue coin is empty".to_owned(),
            ));
        }
        if currency.is_some_and(|wanted| !wanted.eq_ignore_ascii_case(coin)) {
            continue;
        }
        let usd_value = row
            .usd_value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| {
                ExchangeError::Parse(format!(
                    "bitget account usdValue invalid for {coin}: {}",
                    row.usd_value
                ))
            })?;
        rows.push(VenueAssetValuation {
            venue: "bitget".to_owned(),
            currency: coin.to_owned(),
            usd_value,
            source: "bitget.GET /api/v3/account/assets.usdValue".to_owned(),
            observed_at_ms,
        });
    }
    rows.sort_by(|a, b| a.currency.cmp(&b.currency));
    Ok(rows)
}

pub(super) async fn account_hold_mode(request: &SignedRequest<'_>) -> ExchangeResult<String> {
    let wrap: BitgetObjectResponse<AccountSettings> = signed_get_json(request).await?;
    let mode = wrap.into_result("account settings")?.hold_mode;
    match mode.as_str() {
        "one_way_mode" | "hedge_mode" => Ok(mode),
        _ => Err(ExchangeError::Parse(format!(
            "bitget account settings has unsupported holdMode {mode:?}"
        ))),
    }
}

pub(super) async fn validate_order_permissions(request: &SignedRequest<'_>) -> ExchangeResult<()> {
    let wrap: BitgetObjectResponse<AccountInfo> = signed_get_json(request).await?;
    let info = wrap.into_result("account info")?;
    let permission_type = info
        .perm_type
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    let has_trade_scope = info
        .permissions
        .iter()
        .any(|permission| permission.trim().eq_ignore_ascii_case("uta_trade"));
    if permission_type == "readandwrite" && has_trade_scope {
        return Ok(());
    }
    Err(ExchangeError::Api {
        exchange: "bitget".into(),
        code: "api_trading_disabled".into(),
        message: format!(
            "account info does not grant UTA trade read/write permission: permType={:?}, uta_trade={has_trade_scope}",
            info.perm_type
        ),
    })
}

pub(super) async fn funding_payments(
    request: &SignedRequest<'_>,
) -> ExchangeResult<FundingPaymentPage> {
    let wrap: BitgetObjectResponse<BitgetUtaFinancialRecordsPage> =
        signed_get_json(request).await?;
    parse_bitget_funding_payments(wrap.into_result("financial records")?)
}

pub(super) async fn positions(
    request: &SignedRequest<'_>,
    target: Option<&str>,
) -> ExchangeResult<Vec<PositionInfo>> {
    let wrap: BitgetObjectResponse<UtaListPayload<UtaPositionRow>> =
        signed_get_json(request).await?;
    let payload = wrap.into_result("current position")?;
    parse_positions(&payload.list, target)
}

pub(super) async fn open_orders(request: &SignedRequest<'_>) -> ExchangeResult<Vec<OrderInfo>> {
    let wrap: BitgetObjectResponse<UtaListPayload<UtaOrderRow>> = signed_get_json(request).await?;
    let payload = wrap.into_result("unfilled orders")?;
    parse_open_orders(payload.list)
}

async fn signed_post_object<T: serde::de::DeserializeOwned>(
    request: &SignedRequest<'_>,
    body: String,
    context: &str,
) -> ExchangeResult<T> {
    let url = request.url();
    // 非幂等写：不重放（http.rs execute_once 约定），结果由读侧按
    // client_order_id 对账；重试还会复用一次性签名时间戳。
    let resp = request
        .http
        .execute_once(|| signed_builder(request, Method::POST, &url, Some(body.clone())))
        .await?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if !status.is_success() {
        if let Some(error) = api_error_from_body(&text, context) {
            return Err(error);
        }
        return Err(ExchangeError::Http {
            status: status.as_u16(),
            body: bounded_body(&text),
        });
    }
    let wrap: BitgetObjectResponse<T> = serde_json::from_str(&text)
        .map_err(|error| ExchangeError::Parse(format!("bitget {context} schema: {error}")))?;
    wrap.into_result(context)
}

async fn signed_get_json<T: serde::de::DeserializeOwned>(
    request: &SignedRequest<'_>,
) -> ExchangeResult<T> {
    let url = request.url();
    let resp = request
        .http
        .execute_with_retry(|| signed_builder(request, Method::GET, &url, None))
        .await?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|error| ExchangeError::Network(format!("bitget signed GET body: {error}")))?;
    if !status.is_success() {
        if let Some(error) = api_error_from_body(&text, request.path) {
            return Err(error);
        }
        return Err(ExchangeError::Http {
            status: status.as_u16(),
            body: bounded_body(&text),
        });
    }
    decode_signed_get_json(&text, request.path)
}

fn bounded_body(body: &str) -> String {
    body.chars().take(512).collect()
}

fn decode_signed_get_json<T: serde::de::DeserializeOwned>(
    body: &str,
    path: &str,
) -> ExchangeResult<T> {
    let mut value: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        ExchangeError::Parse(format!("bitget signed GET json at {path}: {error}"))
    })?;
    if let Some(data) = value
        .get_mut("data")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(list) = data.get_mut("list").filter(|list| list.is_null()) {
            *list = serde_json::Value::Array(Vec::new());
        }
        if let Some(cursor) = data.get_mut("cursor").filter(|cursor| cursor.is_null()) {
            *cursor = serde_json::Value::String(String::new());
        }
    }
    serde_json::from_value(value).map_err(|error| {
        ExchangeError::Parse(format!("bitget signed GET schema at {path}: {error}"))
    })
}

fn signed_builder(
    request: &SignedRequest<'_>,
    method: Method,
    url: &str,
    body: Option<String>,
) -> reqwest::RequestBuilder {
    let mut req = request.http.request(method, url);
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

fn bitget_safe_cancel_nonmatch(error: &ExchangeError) -> bool {
    match error {
        ExchangeError::Api { message, .. } => {
            let message = message.to_ascii_lowercase();
            message.contains("order does not exist")
                || message.contains("order not exist")
                || message.contains("order not found")
                || message.contains("not exist")
                || message.contains("not found")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitget_http() -> HttpClient {
        HttpClient::builder("bitget")
            .timeout_secs(5)
            .build()
            .expect("http client")
    }

    fn headers() -> SignedHeaders {
        [
            ("ACCESS-KEY".into(), "key".into()),
            ("ACCESS-SIGN".into(), "sig".into()),
            ("ACCESS-TIMESTAMP".into(), "1700000000000".into()),
            ("ACCESS-PASSPHRASE".into(), "pass".into()),
            ("Content-Type".into(), "application/json".into()),
        ]
    }

    #[tokio::test]
    async fn account_read_preserves_documented_asset_usd_values() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v3/account/assets"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": "00000",
                    "msg": "success",
                    "data": {
                        "accountEquity": "11.13919278",
                        "effEquity": "6.19299777",
                        "imr": "0",
                        "mmr": "0",
                        "mgnRatio": "0",
                        "positionMgnRatio": "0",
                        "usdtUnrealisedPnl": "0",
                        "assets": [
                            {
                                "coin": "BGB",
                                "equity": "1.15582129",
                                "usdValue": "4.94618029",
                                "available": "1.15582129",
                                "locked": "0"
                            },
                            {
                                "coin": "USDT",
                                "equity": "6.19300826",
                                "usdValue": "6.19299777",
                                "available": "6.19300826",
                                "locked": "0"
                            }
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;
        let http = bitget_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v3/account/assets",
            headers: &headers,
        };

        let read = account_read(&request, None)
            .await
            .expect("documented account assets response");

        assert_eq!(read.balances.len(), 2);
        assert_eq!(read.asset_valuations.len(), 2);
        assert_eq!(read.asset_valuations[0].currency, "BGB");
        assert!((read.asset_valuations[0].usd_value - 4.94618029).abs() < 1e-9);
        assert!(read.asset_valuations[0]
            .source
            .contains("account/assets.usdValue"));
    }

    #[tokio::test]
    async fn account_hold_mode_reads_official_uta_settings_path() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v3/account/settings"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"code":"00000","msg":"success","data":{"holdMode":"hedge_mode"}}"#,
            ))
            .mount(&server)
            .await;
        let http = bitget_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v3/account/settings",
            headers: &headers,
        };

        assert_eq!(
            account_hold_mode(&request)
                .await
                .expect("official UTA settings holdMode parses"),
            "hedge_mode"
        );
    }

    #[tokio::test]
    async fn account_hold_mode_rejects_unknown_semantics() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v3/account/settings"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"code":"00000","msg":"success","data":{"holdMode":"portfolio"}}"#,
            ))
            .mount(&server)
            .await;
        let http = bitget_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v3/account/settings",
            headers: &headers,
        };

        assert!(account_hold_mode(&request).await.is_err());
    }

    #[tokio::test]
    async fn account_info_proves_uta_trade_read_write_for_documented_and_live_spellings() {
        for permission_type in ["read-and-write", "read_and_write"] {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .and(wiremock::matchers::path("/api/v3/account/info"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({
                        "code": "00000",
                        "msg": "success",
                        "data": {
                            "permType": permission_type,
                            "permissions": ["uta_mgt", "uta_trade"]
                        }
                    }),
                ))
                .mount(&server)
                .await;
            let http = bitget_http();
            let headers = headers();
            let request = SignedRequest {
                http: &http,
                base_url: &server.uri(),
                path: "/api/v3/account/info",
                headers: &headers,
            };

            validate_order_permissions(&request)
                .await
                .expect("UTA trade read/write permission is explicit");
        }
    }

    #[tokio::test]
    async fn account_info_rejects_read_only_trade_permission() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v3/account/info"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": "00000",
                    "msg": "success",
                    "data": {"permType": "read-only", "permissions": ["uta_trade"]}
                })),
            )
            .mount(&server)
            .await;
        let http = bitget_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v3/account/info",
            headers: &headers,
        };

        assert!(matches!(
            validate_order_permissions(&request).await,
            Err(ExchangeError::Api { code, .. }) if code == "api_trading_disabled"
        ));
    }

    #[tokio::test]
    async fn signed_get_normalizes_live_null_list_to_empty_collection() {
        #[derive(Debug, Deserialize)]
        struct ListPayload {
            list: Vec<serde_json::Value>,
            cursor: String,
        }

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/api/v3/account/financial-records",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": "00000",
                    "msg": "success",
                    "data": {"list": null, "cursor": null}
                })),
            )
            .mount(&server)
            .await;
        let http = bitget_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v3/account/financial-records",
            headers: &headers,
        };

        let wrap: BitgetObjectResponse<ListPayload> = signed_get_json(&request)
            .await
            .expect("signed GET envelope");
        let payload = wrap
            .into_result("financial records")
            .expect("empty list payload");
        assert!(payload.list.is_empty());
        assert!(payload.cursor.is_empty());
    }

    #[tokio::test]
    async fn safe_cancel_probe_rejects_unexpected_2xx_collision() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/v3/trade/cancel-order"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "symbol": "BTCUSDT",
                "category": "USDT-FUTURES",
                "clientOid": "xlinecancel1"
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"code":"00000","msg":"success","data":{"orderId":"","clientOid":"xlinecancel1"}}"#,
            ))
            .mount(&server)
            .await;
        let http = bitget_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v3/trade/cancel-order",
            headers: &headers,
        };

        let error = safe_cancel_probe(
            &request,
            r#"{"category":"USDT-FUTURES","symbol":"BTCUSDT","clientOid":"xlinecancel1"}"#
                .to_owned(),
        )
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
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/v3/trade/cancel-order"))
            .respond_with(
                wiremock::ResponseTemplate::new(400).set_body_string(
                    r#"{"code":"25204","msg":"Order does not exist","data":null}"#,
                ),
            )
            .mount(&server)
            .await;
        let http = bitget_http();
        let headers = headers();
        let request = SignedRequest {
            http: &http,
            base_url: &server.uri(),
            path: "/api/v3/trade/cancel-order",
            headers: &headers,
        };

        safe_cancel_probe(
            &request,
            r#"{"category":"USDT-FUTURES","symbol":"BTCUSDT","clientOid":"xlinecancel1"}"#
                .to_owned(),
        )
        .await
        .expect("unknown clientOid proves the probe did not cancel a live order");
    }
}
