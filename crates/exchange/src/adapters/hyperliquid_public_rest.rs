//! Hyperliquid `/info` REST request helpers.

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;
use serde_json::Value;

pub(super) const INFO_PATH: &str = "/info";

pub(super) async fn post_info<T: serde::de::DeserializeOwned>(
    http: &HttpClient,
    base_url: &str,
    body: Value,
) -> ExchangeResult<T> {
    let url = format!("{base_url}{INFO_PATH}");
    let weight = info_request_weight(&body);
    let resp = http
        .execute_with_retry_weighted(weight, || {
            http.request(Method::POST, &url)
                .header("Content-Type", "application/json")
                .json(&body)
        })
        .await?;
    resp.json::<T>().await.map_err(|error| parse_err(&error))
}

/// Hyperliquid applies operation-specific weights even though all info calls
/// share one `/info` path. Path-only lookup therefore overcharges light account
/// reads and cannot represent `userRole`.
///
/// <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/rate-limits-and-user-limits>
pub(super) fn info_request_weight(body: &Value) -> u32 {
    match body.get("type").and_then(Value::as_str) {
        Some(
            "l2Book"
            | "allMids"
            | "clearinghouseState"
            | "orderStatus"
            | "spotClearinghouseState"
            | "exchangeStatus",
        ) => 2,
        Some("userRole") => 60,
        _ => 20,
    }
}

fn parse_err(error: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("hyperliquid json: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn info_weight_uses_official_operation_class() {
        assert_eq!(info_request_weight(&json!({"type": "allMids"})), 2);
        assert_eq!(
            info_request_weight(&json!({"type": "spotClearinghouseState"})),
            2
        );
        assert_eq!(
            info_request_weight(&json!({"type": "spotMetaAndAssetCtxs"})),
            20
        );
        assert_eq!(info_request_weight(&json!({"type": "userRole"})), 60);
        assert_eq!(info_request_weight(&json!({})), 20);
    }
}
