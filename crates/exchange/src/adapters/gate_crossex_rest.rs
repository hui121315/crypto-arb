//! Bounded Gate `CrossEx` REST metadata reads and `APIv4` signing.

use super::gate_crossex_config::GateCrossExCredentials;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use common::signing::hmac_sha512_hex;
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use sha2::{Digest, Sha512};
use std::collections::HashMap;

const FUNDING_PATH: &str = "/api/v4/crossex/market/funding_info";
const ACCOUNT_PATH: &str = "/api/v4/crossex/accounts";
const OPEN_ORDERS_PATH: &str = "/api/v4/crossex/open_orders";
const POSITIONS_PATH: &str = "/api/v4/crossex/positions";

pub(super) async fn fetch_funding_intervals(
    http: &HttpClient,
    rest_base_url: &str,
    credentials: &GateCrossExCredentials,
) -> ExchangeResult<HashMap<String, u32>> {
    let url = format!("{rest_base_url}/crossex/market/funding_info");
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            Ok(signed_get_request(
                http,
                &url,
                FUNDING_PATH,
                "",
                credentials,
            ))
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
    parse_funding_intervals(&body)
}

pub(super) async fn fetch_account(
    http: &HttpClient,
    rest_base_url: &str,
    credentials: &GateCrossExCredentials,
) -> ExchangeResult<String> {
    signed_get(http, rest_base_url, ACCOUNT_PATH, "", credentials).await
}

pub(super) async fn fetch_open_orders(
    http: &HttpClient,
    rest_base_url: &str,
    credentials: &GateCrossExCredentials,
    symbol: Option<&str>,
) -> ExchangeResult<String> {
    let query = optional_query("symbol", symbol);
    signed_get(http, rest_base_url, OPEN_ORDERS_PATH, &query, credentials).await
}

pub(super) async fn fetch_positions(
    http: &HttpClient,
    rest_base_url: &str,
    credentials: &GateCrossExCredentials,
    symbol: Option<&str>,
) -> ExchangeResult<String> {
    let query = optional_query("symbol", symbol);
    signed_get(http, rest_base_url, POSITIONS_PATH, &query, credentials).await
}

pub(super) async fn fetch_order(
    http: &HttpClient,
    rest_base_url: &str,
    credentials: &GateCrossExCredentials,
    order_or_text: &str,
) -> ExchangeResult<Option<String>> {
    let path = format!("/api/v4/crossex/orders/{order_or_text}");
    match signed_get(http, rest_base_url, &path, "", credentials).await {
        Ok(body) => Ok(Some(body)),
        Err(ExchangeError::Http { status: 404, .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn signed_get(
    http: &HttpClient,
    rest_base_url: &str,
    signed_path: &str,
    query: &str,
    credentials: &GateCrossExCredentials,
) -> ExchangeResult<String> {
    let endpoint = signed_path.strip_prefix("/api/v4").ok_or_else(|| {
        ExchangeError::Parse(format!("invalid CrossEx signed path {signed_path}"))
    })?;
    let mut url = format!("{rest_base_url}{endpoint}");
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            Ok(signed_get_request(
                http,
                &url,
                signed_path,
                query,
                credentials,
            ))
        })
        .await?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(ExchangeError::Http {
            status: status.as_u16(),
            body,
        })
    }
}

fn optional_query(name: &str, value: Option<&str>) -> String {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return String::new();
    };
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair(name, value);
    serializer.finish()
}

fn signed_get_request(
    http: &HttpClient,
    url: &str,
    signed_path: &str,
    query: &str,
    credentials: &GateCrossExCredentials,
) -> RequestBuilder {
    let timestamp = common::time::now_ms() / 1_000;
    let signature = rest_signature(
        Method::GET.as_str(),
        signed_path,
        query,
        "",
        timestamp,
        &credentials.api_secret,
    );
    http.request(Method::GET, url)
        .header("KEY", &credentials.api_key)
        .header("Timestamp", timestamp.to_string())
        .header("SIGN", signature)
}

fn rest_signature(
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    timestamp: i64,
    secret: &str,
) -> String {
    let body_hash = hex::encode(Sha512::digest(body.as_bytes()));
    let message = format!("{method}\n{path}\n{query}\n{body_hash}\n{timestamp}");
    hmac_sha512_hex(secret.as_bytes(), message.as_bytes())
}

fn parse_funding_intervals(text: &str) -> ExchangeResult<HashMap<String, u32>> {
    let rows: Vec<FundingInfo> = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("gate crossex funding info: {error}")))?;
    let mut intervals = HashMap::new();
    for row in rows {
        let seconds = row.funding_interval.parse::<u32>().map_err(|error| {
            ExchangeError::Parse(format!(
                "gate crossex funding interval {:?}: {error}",
                row.funding_interval
            ))
        })?;
        if seconds == 0 || seconds % 3_600 != 0 {
            return Err(ExchangeError::Parse(format!(
                "gate crossex funding interval is not whole hours: {seconds}"
            )));
        }
        intervals.insert(row.symbol, seconds / 3_600);
    }
    Ok(intervals)
}

#[derive(Debug, Deserialize)]
struct FundingInfo {
    symbol: String,
    funding_interval: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn funding_metadata_keeps_native_event_intervals() {
        let rows = parse_funding_intervals(include_str!(
            "../../fixtures/gate_crossex/funding_info_intervals.json"
        ))
        .unwrap();
        assert_eq!(rows["KRAKEN_FUTURE_BTC_USD"], 1);
        assert_eq!(rows["GATE_FUTURE_BTC_USDT"], 8);
    }

    #[test]
    fn rest_signature_is_stable_and_binds_full_api_path() {
        let signature = rest_signature("GET", FUNDING_PATH, "", "", 1_700_000_000, "secret");
        assert_eq!(signature.len(), 128);
        assert_eq!(
            signature,
            rest_signature("GET", FUNDING_PATH, "", "", 1_700_000_000, "secret")
        );
        assert_ne!(
            signature,
            rest_signature("GET", "/wrong", "", "", 1_700_000_000, "secret")
        );
    }

    #[test]
    fn signed_private_reads_bind_canonical_crossex_paths_and_queries() {
        let http = HttpClient::builder("gate-crossex-rest-test")
            .build()
            .unwrap();
        let credentials = GateCrossExCredentials {
            api_key: "key".to_owned(),
            api_secret: "secret".to_owned(),
        };
        let query = optional_query("symbol", Some("GATE_FUTURE_BTC_USDT"));
        let request = signed_get_request(
            &http,
            &format!("https://api.gateio.ws/api/v4/crossex/open_orders?{query}"),
            OPEN_ORDERS_PATH,
            &query,
            &credentials,
        )
        .build()
        .unwrap();
        assert_eq!(request.method(), Method::GET);
        assert_eq!(request.url().path(), OPEN_ORDERS_PATH);
        assert_eq!(request.url().query(), Some(query.as_str()));
        assert_eq!(request.headers()["KEY"], "key");
        assert_eq!(request.headers()["SIGN"].to_str().unwrap().len(), 128);

        for path in [
            FUNDING_PATH,
            ACCOUNT_PATH,
            POSITIONS_PATH,
            "/api/v4/crossex/orders/order-1",
        ] {
            let request = signed_get_request(
                &http,
                &format!("https://api.gateio.ws{path}"),
                path,
                "",
                &credentials,
            )
            .build()
            .unwrap();
            assert_eq!(request.method(), Method::GET);
            assert_eq!(request.url().path(), path);
            assert_eq!(request.headers()["KEY"], "key");
        }
    }
}
