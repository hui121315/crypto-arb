//! KuCoin public REST request helpers.

use crate::adapters::kucoin_market_data::{
    ContractActive, DepthData, FuturesTickerItem, SpotDepthData, SpotTickerData,
};
use crate::adapters::kucoin_response::{parse_err, KucoinResponse};
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use reqwest::Method;
use std::collections::HashMap;

const SPOT_TICKER_BODY_ATTEMPTS: usize = 2;

pub(super) async fn server_time(http: &HttpClient, base_url: &str) -> ExchangeResult<i64> {
    let url = format!("{base_url}/api/v1/timestamp");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: KucoinResponse<i64> = resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("timestamp")
}

pub(super) async fn contracts(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<ContractActive>> {
    let url = format!("{base_url}/api/v1/contracts/active");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: KucoinResponse<Vec<ContractActive>> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("contracts/active")
}

pub(super) async fn instruments_rest(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<crate::adapters::kucoin_instruments::KucoinContractRow>> {
    let url = format!("{base_url}/api/v1/contracts/active");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: KucoinResponse<Vec<crate::adapters::kucoin_instruments::KucoinContractRow>> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("contracts/active")
}

pub(super) async fn contract(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<ContractActive> {
    let url = format!("{base_url}/api/v1/contracts/{symbol}");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: KucoinResponse<ContractActive> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("contracts/{symbol}")
}

pub(super) async fn futures_tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<HashMap<String, FuturesTickerItem>> {
    let url = format!("{base_url}/api/v1/allTickers");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: KucoinResponse<Vec<FuturesTickerItem>> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    Ok(wrap
        .into_data("allTickers")?
        .into_iter()
        .map(|quote| (quote.symbol.clone(), quote))
        .collect())
}

pub(super) async fn futures_ticker(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<FuturesTickerItem> {
    let url = format!("{base_url}/api/v1/ticker");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    let wrap: KucoinResponse<FuturesTickerItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("ticker")
}

pub(super) async fn spot_tickers(
    http: &HttpClient,
    spot_base_url: &str,
) -> ExchangeResult<SpotTickerData> {
    let url = format!("{spot_base_url}/api/v1/market/allTickers");
    let mut last_retryable_error = None;

    for attempt in 1..=SPOT_TICKER_BODY_ATTEMPTS {
        let resp = http
            .execute_with_retry(|| http.request(Method::GET, &url))
            .await?;
        match decode_spot_ticker_response(resp).await {
            Ok(data) => return Ok(data),
            Err((error, true)) if attempt < SPOT_TICKER_BODY_ATTEMPTS => {
                tracing::warn!(
                    attempt,
                    max_attempts = SPOT_TICKER_BODY_ATTEMPTS,
                    error = %error,
                    "retrying kucoin spot tickers after incomplete response body"
                );
                last_retryable_error = Some(error);
            }
            Err((error, _)) => return Err(error),
        }
    }

    Err(last_retryable_error.unwrap_or_else(|| {
        crate::error::ExchangeError::Network("kucoin spot ticker body attempts exhausted".into())
    }))
}

async fn decode_spot_ticker_response(
    resp: reqwest::Response,
) -> Result<SpotTickerData, (crate::error::ExchangeError, bool)> {
    let status = resp.status();
    let body = resp.bytes().await.map_err(|error| {
        (
            crate::error::ExchangeError::Network(format!("kucoin market/allTickers body: {error}")),
            true,
        )
    })?;
    if !status.is_success() {
        return Err((
            crate::error::ExchangeError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            },
            false,
        ));
    }
    let wrap: KucoinResponse<SpotTickerData> = serde_json::from_slice(&body).map_err(|error| {
        let retryable = error.is_eof();
        (
            crate::error::ExchangeError::Parse(format!("kucoin market/allTickers json: {error}")),
            retryable,
        )
    })?;
    wrap.into_data("market/allTickers")
        .map_err(|error| (error, false))
}

pub(super) async fn orderbook(
    http: &HttpClient,
    base_url: &str,
    endpoint: &str,
    symbol: &str,
) -> ExchangeResult<DepthData> {
    let url = format!("{base_url}/api/v1/level2/{endpoint}");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    let wrap: KucoinResponse<DepthData> = resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("level2/depth")
}

pub(super) async fn spot_orderbook(
    http: &HttpClient,
    spot_base_url: &str,
    endpoint: &str,
    symbol: &str,
) -> ExchangeResult<SpotDepthData> {
    let url = format!("{spot_base_url}/api/v1/market/orderbook/{endpoint}");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    let wrap: KucoinResponse<SpotDepthData> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("spot orderbook")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    struct NoQueryParams;

    impl Match for NoQueryParams {
        fn matches(&self, request: &Request) -> bool {
            request.url.query().is_none()
        }
    }

    #[tokio::test]
    async fn kucoin_server_time_parses_official_fixture_and_uses_no_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/kucoin/server_time.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("kucoin time fixture");

        Mock::given(method("GET"))
            .and(path("/api/v1/timestamp"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let server_ms = server_time(&http, &server.uri())
            .await
            .expect("kucoin server time");
        assert_eq!(server_ms, 1_729_260_030_774);
    }

    #[tokio::test]
    async fn kucoin_contracts_active_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/kucoin/contracts_active_xbt_eth_usdtm.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("kucoin contracts fixture");

        Mock::given(method("GET"))
            .and(path("/api/v1/contracts/active"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = contracts(&http, &server.uri())
            .await
            .expect("kucoin contracts");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.symbol == "XBTUSDTM"));
        assert!(rows.iter().any(|row| row.symbol == "ETHUSDTM"));
    }

    #[tokio::test]
    async fn kucoin_futures_all_tickers_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/kucoin/futures_all_tickers_xbt_eth_usdtm.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("kucoin futures allTickers fixture");

        Mock::given(method("GET"))
            .and(path("/api/v1/allTickers"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let quotes = futures_tickers(&http, &server.uri())
            .await
            .expect("kucoin futures allTickers");
        assert_eq!(quotes.len(), 2);
        assert!(quotes.contains_key("XBTUSDTM"));
        assert!(quotes.contains_key("ETHUSDTM"));
    }

    #[tokio::test]
    async fn kucoin_depth20_uses_official_path_and_symbol_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/kucoin/futures_depth20_xbtusdtm.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("kucoin futures depth20 fixture");

        Mock::given(method("GET"))
            .and(path("/api/v1/level2/depth20"))
            .and(query_param("symbol", "XBTUSDTM"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let book = orderbook(&http, &server.uri(), "depth20", "XBTUSDTM")
            .await
            .expect("kucoin depth20");
        assert_eq!(book.bids.len(), 20);
        assert_eq!(book.asks.len(), 20);
    }

    #[tokio::test]
    async fn kucoin_spot_market_all_tickers_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture =
            include_str!("../../fixtures/kucoin/spot_market_all_tickers_btc_eth_usdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("kucoin spot allTickers fixture");

        Mock::given(method("GET"))
            .and(path("/api/v1/market/allTickers"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let data = spot_tickers(&http, &server.uri())
            .await
            .expect("kucoin spot allTickers");
        assert_eq!(data.ticker.len(), 2);
        assert!(data.ticker.iter().any(|row| row.symbol == "BTC-USDT"));
        assert!(data.ticker.iter().any(|row| row.symbol == "ETH-USDT"));
    }

    #[tokio::test]
    async fn kucoin_spot_tickers_retry_one_truncated_response_body() {
        let server = MockServer::start().await;
        let fixture =
            include_str!("../../fixtures/kucoin/spot_market_all_tickers_btc_eth_usdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("kucoin spot allTickers fixture");

        Mock::given(method("GET"))
            .and(path("/api/v1/market/allTickers"))
            .and(NoQueryParams)
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"{"code":"200000","data":{"ticker":["#, "application/json"),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/market/allTickers"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let http = HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let data = spot_tickers(&http, &server.uri())
            .await
            .expect("kucoin spot allTickers retry");
        let requests = server.received_requests().await.expect("request history");
        assert_eq!(requests.len(), 2);
        assert_eq!(data.ticker.len(), 2);
    }

    #[tokio::test]
    async fn kucoin_spot_tickers_do_not_retry_schema_mismatch() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/market/allTickers"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("kucoin")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let error = spot_tickers(&http, &server.uri())
            .await
            .expect_err("schema mismatch must fail without replay");
        assert!(error.to_string().contains("missing field"));
    }
}
