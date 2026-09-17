//! Binance REST response parse adapters.

use crate::error::{ExchangeError, ExchangeResult};
use serde::de::DeserializeOwned;
use serde::Deserialize;

pub(super) async fn checked_json<T: DeserializeOwned>(
    resp: reqwest::Response,
    context: &str,
) -> ExchangeResult<T> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if !status.is_success() {
        return Err(ExchangeError::Http {
            status: status.as_u16(),
            body,
        });
    }
    serde_json::from_str(&body).map_err(|error| ExchangeError::Parse(format!("{context}: {error}")))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ServerTimeResponse {
    pub(super) server_time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListenKeyResponse {
    pub(super) listen_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DepthResponse {
    #[serde(default)]
    pub(super) bids: Vec<[String; 2]>,
    #[serde(default)]
    pub(super) asks: Vec<[String; 2]>,
}

pub(super) fn parse_err(error: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("binance json: {error}"))
}
