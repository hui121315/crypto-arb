//! Gate public REST request helpers.

use crate::adapter::{checked_text_with_evidence, PayloadEvidence};
use crate::adapters::gate_market_data::{
    ContractItem, IndexConstituentsResp, OrderBookResp, SpotTickerItem, TickerItem,
};
use crate::adapters::gate_response::parse_err;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;
use serde::Deserialize;

pub(super) const SIZE_DECIMAL_HEADER: &str = "x-gate-size-decimal";
pub(super) const SIZE_DECIMAL_HEADER_VALUE: &str = "1";

pub(super) fn futures_get(http: &HttpClient, url: &str) -> reqwest::RequestBuilder {
    http.request(Method::GET, url)
        .header(SIZE_DECIMAL_HEADER, SIZE_DECIMAL_HEADER_VALUE)
}

#[derive(Debug, Deserialize)]
struct ServerTime {
    server_time: i64,
}

pub(super) async fn server_time(http: &HttpClient, base_url: &str) -> ExchangeResult<i64> {
    let url = format!("{base_url}/api/v4/spot/time");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let server: ServerTime = resp.json().await.map_err(|error| parse_err(&error))?;
    Ok(server.server_time)
}

pub(super) async fn contract(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<ContractItem> {
    let url = format!("{base_url}/api/v4/futures/usdt/contracts/{symbol}");
    let resp = http.execute_with_retry(|| futures_get(http, &url)).await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

pub(super) async fn ticker(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<Vec<TickerItem>> {
    let url = format!("{base_url}/api/v4/futures/usdt/tickers");
    let resp = http
        .execute_with_retry(|| futures_get(http, &url).query(&[("contract", symbol)]))
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

pub(super) async fn tickers(http: &HttpClient, base_url: &str) -> ExchangeResult<Vec<TickerItem>> {
    let url = format!("{base_url}/api/v4/futures/usdt/tickers");
    let resp = http.execute_with_retry(|| futures_get(http, &url)).await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

pub(super) async fn spot_tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<SpotTickerItem>> {
    let url = format!("{base_url}/api/v4/spot/tickers");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

pub(super) async fn orderbook(
    http: &HttpClient,
    base_url: &str,
    contract: &str,
    limit: &str,
) -> ExchangeResult<OrderBookResp> {
    let url = format!("{base_url}/api/v4/futures/usdt/order_book");
    let resp = http
        .execute_with_retry(|| {
            futures_get(http, &url).query(&[("contract", contract), ("limit", limit)])
        })
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

pub(super) async fn index_constituents(
    http: &HttpClient,
    base_url: &str,
    index: &str,
) -> ExchangeResult<(IndexConstituentsResp, PayloadEvidence)> {
    let url = format!("{base_url}/api/v4/futures/usdt/index_constituents/{index}");
    let resp = http.execute_with_retry(|| futures_get(http, &url)).await?;
    let (body, evidence) = checked_text_with_evidence(resp, url.clone()).await?;
    let parsed = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("gate json: {error}")))?;
    Ok((parsed, evidence))
}

pub(super) async fn spot_orderbook(
    http: &HttpClient,
    base_url: &str,
    currency_pair: &str,
    limit: &str,
) -> ExchangeResult<OrderBookResp> {
    let url = format!("{base_url}/api/v4/spot/order_book");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("currency_pair", currency_pair), ("limit", limit)])
        })
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    struct NoQueryParams;

    impl Match for NoQueryParams {
        fn matches(&self, request: &Request) -> bool {
            request.url.query().is_none()
        }
    }

    #[tokio::test]
    async fn gate_server_time_parses_official_fixture_and_uses_no_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/gate/server_time.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("gate time fixture");

        Mock::given(method("GET"))
            .and(path("/api/v4/spot/time"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("gate")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let server_ms = server_time(&http, &server.uri())
            .await
            .expect("gate server time");
        assert_eq!(server_ms, 1_597_026_383_085);
    }

    #[tokio::test]
    async fn gate_futures_tickers_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/gate/futures_usdt_tickers_btc_eth_usdt.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("gate tickers fixture");

        Mock::given(method("GET"))
            .and(path("/api/v4/futures/usdt/tickers"))
            .and(header(SIZE_DECIMAL_HEADER, SIZE_DECIMAL_HEADER_VALUE))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("gate")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = tickers(&http, &server.uri())
            .await
            .expect("gate futures tickers");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.contract == "BTC_USDT"));
        assert!(rows.iter().any(|row| row.contract == "ETH_USDT"));
    }

    #[tokio::test]
    async fn gate_spot_tickers_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/gate/spot_tickers_btc_eth_usdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("gate spot tickers fixture");

        Mock::given(method("GET"))
            .and(path("/api/v4/spot/tickers"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("gate")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = spot_tickers(&http, &server.uri())
            .await
            .expect("gate spot tickers");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.currency_pair == "BTC_USDT"));
        assert!(rows.iter().any(|row| row.currency_pair == "ETH_USDT"));
    }

    #[tokio::test]
    async fn gate_futures_orderbook_uses_official_path_contract_and_limit_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/gate/futures_usdt_order_book_btc_usdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("gate orderbook fixture");

        Mock::given(method("GET"))
            .and(path("/api/v4/futures/usdt/order_book"))
            .and(header(SIZE_DECIMAL_HEADER, SIZE_DECIMAL_HEADER_VALUE))
            .and(query_param("contract", "BTC_USDT"))
            .and(query_param("limit", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("gate")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let book = orderbook(&http, &server.uri(), "BTC_USDT", "5")
            .await
            .expect("gate futures orderbook");
        assert_eq!(book.bids.len(), 5);
        assert_eq!(book.asks.len(), 5);
    }
}
