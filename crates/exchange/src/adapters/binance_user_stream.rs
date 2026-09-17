//! Binance USD-M Futures user data stream lifecycle helpers.
//!
//! Official docs checked before moving these calls:
//! - Start User Data Stream
//! - Keepalive User Data Stream
//! - Close User Data Stream

use super::binance_private_rest as private_rest;
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use reqwest::Method;

pub(super) async fn start(
    http: &HttpClient,
    base_url: &str,
    api_key: &str,
) -> ExchangeResult<String> {
    private_rest::user_stream_listen_key(
        http,
        base_url,
        Method::POST,
        api_key,
        "start user data stream",
    )
    .await
}

pub(super) async fn keepalive(
    http: &HttpClient,
    base_url: &str,
    api_key: &str,
) -> ExchangeResult<String> {
    private_rest::user_stream_listen_key(
        http,
        base_url,
        Method::PUT,
        api_key,
        "keepalive user data stream",
    )
    .await
}

pub(super) async fn close(http: &HttpClient, base_url: &str, api_key: &str) -> ExchangeResult<()> {
    private_rest::close_user_data_stream(http, base_url, api_key).await
}
