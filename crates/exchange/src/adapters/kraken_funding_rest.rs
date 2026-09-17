//! Funding API GET authentication and bounded pagination.
//! <https://docs.kraken.com/exchange/guides/rest/funding>
use super::kraken_config::KrakenSpotCredentials;
use super::kraken_spot_rest::next_nonce;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::signing::kraken::spot_rest_sign;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashSet;

pub(super) async fn rows<T: DeserializeOwned>(
    http: &HttpClient,
    base: &str,
    path: &str,
    parameters: Vec<(String, String)>,
    key: &str,
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<Vec<T>> {
    let mut parameters = parameters;
    let mut cursors = HashSet::new();
    let mut result = Vec::new();
    for _ in 0..5 {
        let value: Value = get(http, base, path, &parameters, credentials).await?;
        let page = value
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(format!("{key} missing")))?;
        if page.len() > 100 {
            return Err(invalid("Funding page exceeds requested limit"));
        }
        for row in page {
            result.push(
                serde_json::from_value(row.clone())
                    .map_err(|error| invalid(format!("{key}: {error}")))?,
            );
        }
        let Some(cursor) = value.get("next_cursor") else {
            return Ok(result);
        };
        let cursor = cursor
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| invalid("invalid Funding cursor"))?;
        if !cursors.insert(cursor.to_owned()) {
            break;
        }
        // Funding cursors carry the original filter; other parameters are forbidden.
        parameters = vec![("cursor".into(), cursor.into())];
    }
    Err(invalid(
        "Funding pagination incomplete; no partial evidence accepted",
    ))
}

pub(super) async fn get<T: DeserializeOwned>(
    http: &HttpClient,
    base: &str,
    path: &str,
    parameters: &[(String, String)],
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<T> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(parameters.iter().map(|(key, value)| (key, value)))
        .finish();
    let signed_path = if query.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{query}")
    };
    let url = format!("{base}{signed_path}");
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            signed_request(http, &url, &signed_path, Method::GET, "", credentials)
        })
        .await?;
    decode(response).await
}

pub(super) async fn post_once<T: DeserializeOwned>(
    http: &HttpClient,
    base: &str,
    path: &str,
    body: &Value,
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<T> {
    let body = serde_json::to_string(body).map_err(|e| invalid(e.to_string()))?;
    let url = format!("{base}{path}");
    let response = http
        .execute_once_fresh(Method::POST, &url, || {
            signed_request(http, &url, path, Method::POST, &body, credentials)
        })
        .await?;
    decode(response).await
}

fn signed_request(
    http: &HttpClient,
    url: &str,
    path: &str,
    method: Method,
    body: &str,
    credentials: &KrakenSpotCredentials,
) -> ExchangeResult<reqwest::RequestBuilder> {
    let nonce = next_nonce().to_string();
    let signature = spot_rest_sign(&credentials.api_secret, path, &nonce, body)
        .map_err(|error| ExchangeError::Auth(format!("Kraken Funding signing: {error}")))?;
    let request = http
        .request(method, url)
        .header("API-Key", &credentials.api_key)
        .header("API-Sign", signature)
        .header("API-Nonce", nonce);
    Ok(if body.is_empty() {
        request
    } else {
        request
            .header("Content-Type", "application/json")
            .body(body.to_owned())
    })
}

pub(super) async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> ExchangeResult<T> {
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
    serde_json::from_str(&body).map_err(|error| invalid(format!("Funding response: {error}")))
}

pub(super) fn invalid(message: impl Into<String>) -> ExchangeError {
    ExchangeError::Parse(format!("kraken: {}", message.into()))
}
