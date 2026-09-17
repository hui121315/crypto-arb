//! Freshly signed Binance USD-M position-mode read.

use crate::adapters::binance_private_data::{
    parse_position_mode, BinancePositionMode, PositionSideDualResponse,
};
use crate::adapters::binance_response::checked_json;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;

const POSITION_MODE_PATH: &str = "/fapi/v1/positionSide/dual";

pub(super) async fn position_mode<F>(
    http: &HttpClient,
    base_url: &str,
    mut sign: F,
) -> ExchangeResult<BinancePositionMode>
where
    F: FnMut() -> ExchangeResult<(String, String)>,
{
    let endpoint = format!("{base_url}{POSITION_MODE_PATH}");
    let response = http
        .execute_with_retry_fresh(Method::GET, &endpoint, || {
            let (signed_query, api_key) = sign()?;
            if api_key.trim().is_empty() {
                return Err(ExchangeError::Auth(
                    "binance private request missing API key".into(),
                ));
            }
            Ok(http
                .request(Method::GET, format!("{endpoint}?{signed_query}"))
                .header("X-MBX-APIKEY", api_key))
        })
        .await?;
    let row = checked_json::<PositionSideDualResponse>(response, "binance position mode").await?;
    Ok(parse_position_mode(&row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn position_mode_signs_at_transport_attempt_and_parses_official_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(POSITION_MODE_PATH))
            .and(query_param("timestamp", "1700000000000"))
            .and(query_param("signature", "signed"))
            .and(header("X-MBX-APIKEY", "key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "dualSidePosition": true
            })))
            .expect(1)
            .mount(&server)
            .await;
        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");
        let calls = AtomicUsize::new(0);

        let mode = position_mode(&http, &server.uri(), || {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok((
                "recvWindow=5000&timestamp=1700000000000&signature=signed".to_owned(),
                "key".to_owned(),
            ))
        })
        .await
        .expect("position mode");

        assert_eq!(mode, BinancePositionMode::Hedge);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
