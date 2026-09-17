//! Kraken Spot signed REST token and reconciliation reads.

use super::kraken_config::KrakenSpotCredentials;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::signing::kraken::spot_rest_sign;
use reqwest::{header::CONTENT_TYPE, Method, RequestBuilder};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

const TOKEN_PATH: &str = "/0/private/GetWebSocketsToken";
const BALANCE_PATH: &str = "/0/private/BalanceEx";
const OPEN_ORDERS_PATH: &str = "/0/private/OpenOrders";
const QUERY_ORDERS_PATH: &str = "/0/private/QueryOrders";

pub(super) async fn fetch_ws_token(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<String> {
    let body = signed_post(http, base_url, TOKEN_PATH, Vec::new(), credentials).await?;
    let value = parse_result(&body, "GetWebSocketsToken")?;
    value
        .get("token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ExchangeError::Parse("kraken websocket token missing".to_owned()))
}

pub(super) async fn fetch_balance_ex(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<String> {
    signed_post(http, base_url, BALANCE_PATH, Vec::new(), credentials).await
}

pub(super) async fn fetch_open_orders(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<String> {
    signed_post(
        http,
        base_url,
        OPEN_ORDERS_PATH,
        vec![("trades".to_owned(), "true".to_owned())],
        credentials,
    )
    .await
}

pub(super) async fn fetch_orders(
    http: &HttpClient,
    base_url: &str,
    credentials: &KrakenSpotCredentials,
    order_ids: &[String],
) -> ExchangeResult<String> {
    signed_post(
        http,
        base_url,
        QUERY_ORDERS_PATH,
        vec![("txid".to_owned(), order_ids.join(","))],
        credentials,
    )
    .await
}

pub(super) async fn signed_post(
    http: &HttpClient,
    base_url: &str,
    path: &str,
    parameters: Vec<(String, String)>,
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<String> {
    let url = format!("{base_url}{path}");
    let credentials = credentials.clone();
    let response = http
        .execute_with_retry_fresh(Method::POST, &url, || {
            signed_request(http, &url, path, &parameters, &credentials)
        })
        .await?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if !status.is_success() {
        return Err(ExchangeError::Http {
            status: status.as_u16(),
            body,
        });
    }
    parse_result(&body, path)?;
    Ok(body)
}

fn signed_request(
    http: &HttpClient,
    url: &str,
    path: &str,
    parameters: &[(String, String)],
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<RequestBuilder> {
    let nonce = next_nonce().to_string();
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("nonce", &nonce);
    for (name, value) in parameters {
        serializer.append_pair(name, value);
    }
    let body = serializer.finish();
    let signature = spot_rest_sign(&credentials.api_secret, path, &nonce, &body)
        .map_err(|error| ExchangeError::Auth(format!("kraken spot signing: {error}")))?;
    Ok(http
        .request(Method::POST, url)
        .header("API-Key", &credentials.api_key)
        .header("API-Sign", signature)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body))
}

fn parse_result(text: &str, operation: &str) -> ExchangeResult<Value> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken {operation} json: {error}")))?;
    if let Some(error) = value
        .get("error")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(Value::as_str)
    {
        return Err(ExchangeError::Api {
            exchange: "kraken".to_owned(),
            code: error.split(':').next().unwrap_or("unknown").to_owned(),
            message: error.to_owned(),
        });
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| ExchangeError::Parse(format!("kraken {operation} result missing")))
}

pub(super) fn next_nonce() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = u64::try_from(common::time::now_ms()).unwrap_or_default() * 1_000;
    let mut previous = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(previous.saturating_add(1));
        match LAST.compare_exchange_weak(previous, next, Ordering::SeqCst, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(actual) => previous = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_request_uses_official_headers_and_body_contract() {
        let http = HttpClient::builder("kraken-test").build().unwrap();
        let request = signed_request(
            &http,
            "https://api.kraken.com/0/private/BalanceEx",
            BALANCE_PATH,
            &[],
            &KrakenSpotCredentials {
                api_key: "key".to_owned(),
                api_secret: "c2VjcmV0".to_owned(),
            },
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(request.method(), Method::POST);
        assert_eq!(request.headers()["API-Key"], "key");
        assert_eq!(request.headers()["API-Sign"].to_str().unwrap().len(), 88);
        assert!(request
            .body()
            .and_then(|body| body.as_bytes())
            .unwrap()
            .starts_with(b"nonce="));
    }

    #[test]
    fn official_token_fixture_and_private_read_requests_match_spot_contracts() {
        let token = parse_result(
            include_str!("../../fixtures/kraken/spot_ws_token.json"),
            "GetWebSocketsToken",
        )
        .unwrap();
        assert_eq!(
            token.get("token").and_then(Value::as_str),
            Some("15-digit-websocket-token")
        );

        let http = HttpClient::builder("kraken-spot-private-test")
            .build()
            .unwrap();
        let credentials = KrakenSpotCredentials {
            api_key: "key".to_owned(),
            api_secret: "c2VjcmV0".to_owned(),
        };
        let open = signed_request(
            &http,
            "https://api.kraken.com/0/private/OpenOrders",
            OPEN_ORDERS_PATH,
            &[("trades".to_owned(), "true".to_owned())],
            &credentials,
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(open.url().path(), OPEN_ORDERS_PATH);
        let open_body = open.body().and_then(|body| body.as_bytes()).unwrap();
        assert!(open_body.windows(11).any(|part| part == b"trades=true"));

        let query = signed_request(
            &http,
            "https://api.kraken.com/0/private/QueryOrders",
            QUERY_ORDERS_PATH,
            &[("txid".to_owned(), "ORDER-1,ORDER-2".to_owned())],
            &credentials,
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(query.url().path(), QUERY_ORDERS_PATH);
        let query_body = query.body().and_then(|body| body.as_bytes()).unwrap();
        assert!(query_body.windows(5).any(|part| part == b"txid="));
    }

    #[test]
    fn nonce_is_strictly_monotonic() {
        let first = next_nonce();
        assert!(next_nonce() > first);
    }
}
