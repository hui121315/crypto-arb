//! Binance public REST request helpers.

use super::binance_exchange_info::ExchangeInfoResponse;
use super::binance_funding_info::FundingInfoItem;
use super::binance_market_data::{
    BookTickerItem, IndexConstituentsResponse, OpenInterestItem, PremiumIndexItem, Ticker24hItem,
};
use super::binance_response::{checked_json, parse_err, DepthResponse, ServerTimeResponse};
use crate::adapter::{checked_text_with_evidence, PayloadEvidence};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;

const SPOT_TICKER_BODY_ATTEMPTS: usize = 2;

pub(super) async fn server_time(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<ServerTimeResponse> {
    let url = format!("{base_url}/fapi/v1/time");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    checked_json(resp, "binance server time").await
}

pub(super) async fn funding_info(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<FundingInfoItem>> {
    let url = format!("{base_url}/fapi/v1/fundingInfo");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    checked_json(resp, "binance funding info").await
}

pub(super) async fn exchange_info(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<ExchangeInfoResponse> {
    let url = format!("{base_url}/fapi/v1/exchangeInfo");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    checked_json(resp, "binance exchange info").await
}

pub(super) async fn funding_rate(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<PremiumIndexItem> {
    let url = format!("{base_url}/fapi/v1/premiumIndex");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    checked_json(resp, "binance premium index").await
}

pub(super) async fn premium_indexes(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<PremiumIndexItem>> {
    let url = format!("{base_url}/fapi/v1/premiumIndex");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    checked_json(resp, "binance premium indexes").await
}

pub(super) async fn open_interest(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<OpenInterestItem> {
    let url = format!("{base_url}/fapi/v1/openInterest");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    checked_json(resp, "binance open interest").await
}

pub(super) async fn index_constituents(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<(IndexConstituentsResponse, PayloadEvidence)> {
    let url = format!("{base_url}/fapi/v1/constituents");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    let (body, evidence) =
        checked_text_with_evidence(resp, format!("{url}?symbol={symbol}")).await?;
    let parsed = serde_json::from_str(&body).map_err(|error| {
        crate::error::ExchangeError::Parse(format!("binance index constituents: {error}"))
    })?;
    Ok((parsed, evidence))
}

pub(super) async fn funding_rates_and_tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<(Vec<PremiumIndexItem>, Vec<Ticker24hItem>)> {
    let premium_url = format!("{base_url}/fapi/v1/premiumIndex");
    let ticker_url = format!("{base_url}/fapi/v1/ticker/24hr");
    let (premium_resp, ticker_resp) = tokio::try_join!(
        http.execute_with_retry(|| http.request(Method::GET, &premium_url)),
        http.execute_with_retry(|| http.request(Method::GET, &ticker_url)),
    )?;
    Ok((
        checked_json(premium_resp, "binance premium indexes").await?,
        checked_json(ticker_resp, "binance ticker 24hr").await?,
    ))
}

pub(super) async fn ticker_and_book(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<(Ticker24hItem, BookTickerItem)> {
    let ticker_url = format!("{base_url}/fapi/v1/ticker/24hr");
    let book_url = format!("{base_url}/fapi/v1/ticker/bookTicker");
    let (ticker_resp, book_resp) = tokio::try_join!(
        http.execute_with_retry(|| http
            .request(Method::GET, &ticker_url)
            .query(&[("symbol", symbol)])),
        http.execute_with_retry(|| http
            .request(Method::GET, &book_url)
            .query(&[("symbol", symbol)])),
    )?;
    Ok((
        checked_json(ticker_resp, "binance ticker 24hr").await?,
        checked_json(book_resp, "binance book ticker").await?,
    ))
}

pub(super) async fn tickers_and_books(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<(Vec<Ticker24hItem>, Vec<BookTickerItem>)> {
    let ticker_url = format!("{base_url}/fapi/v1/ticker/24hr");
    let book_url = format!("{base_url}/fapi/v1/ticker/bookTicker");
    let (ticker_resp, book_resp) = tokio::try_join!(
        http.execute_with_retry(|| http.request(Method::GET, &ticker_url)),
        http.execute_with_retry(|| http.request(Method::GET, &book_url)),
    )?;
    Ok((
        checked_json(ticker_resp, "binance ticker 24hr").await?,
        checked_json(book_resp, "binance book ticker").await?,
    ))
}

pub(super) async fn spot_tickers(
    http: &HttpClient,
    spot_base_url: &str,
    symbols_param: Option<&str>,
) -> ExchangeResult<Vec<Ticker24hItem>> {
    match request_spot_tickers(http, spot_base_url, symbols_param).await {
        Err(error) if symbols_param.is_some() && is_invalid_symbol_error(&error) => {
            tracing::warn!(
                error = %error,
                "binance spot ticker symbols batch contained an unavailable pair; retrying all-market snapshot"
            );
            request_spot_tickers(http, spot_base_url, None).await
        }
        result => result,
    }
}

async fn request_spot_tickers(
    http: &HttpClient,
    spot_base_url: &str,
    symbols_param: Option<&str>,
) -> ExchangeResult<Vec<Ticker24hItem>> {
    let url = format!("{spot_base_url}/api/v3/ticker/24hr");
    let weight = spot_ticker_request_weight(symbols_param);
    let mut last_retryable_error = None;

    for attempt in 1..=SPOT_TICKER_BODY_ATTEMPTS {
        let resp = http
            .execute_with_retry_weighted(weight, || {
                let mut req = http.request(Method::GET, &url);
                if let Some(value) = symbols_param {
                    req = req.query(&[("symbols", value)]);
                }
                req
            })
            .await?;
        match decode_spot_ticker_response(resp).await {
            Ok(rows) => return Ok(rows),
            Err((error, true)) if attempt < SPOT_TICKER_BODY_ATTEMPTS => {
                tracing::warn!(
                    attempt,
                    max_attempts = SPOT_TICKER_BODY_ATTEMPTS,
                    error = %error,
                    "retrying binance spot ticker after incomplete response body"
                );
                last_retryable_error = Some(error);
            }
            Err((error, _)) => return Err(error),
        }
    }

    Err(last_retryable_error.unwrap_or_else(|| {
        ExchangeError::Network("binance spot ticker body attempts exhausted".into())
    }))
}

fn is_invalid_symbol_error(error: &ExchangeError) -> bool {
    let ExchangeError::Http { status: 400, body } = error else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("code")?.as_i64())
        == Some(-1121)
}

async fn decode_spot_ticker_response(
    resp: reqwest::Response,
) -> Result<Vec<Ticker24hItem>, (ExchangeError, bool)> {
    let status = resp.status();
    let body = resp.bytes().await.map_err(|error| {
        (
            ExchangeError::Network(format!("binance spot ticker body: {error}")),
            true,
        )
    })?;
    if !status.is_success() {
        return Err((
            ExchangeError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            },
            false,
        ));
    }
    serde_json::from_slice(&body).map_err(|error| {
        let retryable = error.is_eof();
        (
            ExchangeError::Parse(format!("binance spot ticker 24hr: {error}")),
            retryable,
        )
    })
}

/// Official query-dependent request weights:
/// <https://developers.binance.com/docs/binance-spot-api-docs/rest-api/market-data-endpoints#24hr-ticker-price-change-statistics>
fn spot_ticker_request_weight(symbols_param: Option<&str>) -> u32 {
    match symbols_param
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .map(|symbols| symbols.len())
    {
        Some(1..=20) => 2,
        Some(21..=100) => 40,
        _ => 80,
    }
}

pub(super) async fn orderbook(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
    limit: &str,
) -> ExchangeResult<DepthResponse> {
    let url = format!("{base_url}/fapi/v1/depth");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("symbol", symbol), ("limit", limit)])
        })
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

pub(super) async fn spot_orderbook(
    http: &HttpClient,
    spot_base_url: &str,
    symbol: &str,
    limit: &str,
) -> ExchangeResult<DepthResponse> {
    let url = format!("{spot_base_url}/api/v3/depth");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("symbol", symbol), ("limit", limit)])
        })
        .await?;
    resp.json().await.map_err(|error| parse_err(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    struct NoQueryParams;

    impl Match for NoQueryParams {
        fn matches(&self, request: &Request) -> bool {
            request.url.query().is_none()
        }
    }

    #[tokio::test]
    async fn exchange_info_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/usdm_exchange_info_btcusdt.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("exchangeInfo fixture");

        Mock::given(method("GET"))
            .and(path("/fapi/v1/exchangeInfo"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        exchange_info(&http, &server.uri())
            .await
            .expect("exchangeInfo request");
    }

    #[tokio::test]
    async fn binance_spot_ticker_24hr_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/spot_ticker_24hr_full.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("spot ticker fixture");

        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/24hr"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = spot_tickers(&http, &server.uri(), None)
            .await
            .expect("spot tickers request");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BNBBTC");
    }

    #[tokio::test]
    async fn binance_spot_ticker_24hr_uses_exact_symbols_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/spot_ticker_24hr_full.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("spot ticker fixture");

        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/24hr"))
            .and(wiremock::matchers::query_param("symbols", r#"["BNBBTC"]"#))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = spot_tickers(&http, &server.uri(), Some(r#"["BNBBTC"]"#))
            .await
            .expect("targeted spot ticker request");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BNBBTC");
    }

    #[tokio::test]
    async fn binance_spot_ticker_retries_one_truncated_response_body() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/spot_ticker_24hr_full.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("spot ticker fixture");

        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/24hr"))
            .and(NoQueryParams)
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"[{"symbol":"BNBBTC""#, "application/json"),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/24hr"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = spot_tickers(&http, &server.uri(), None)
            .await
            .expect("spot ticker retry");
        let requests = server.received_requests().await.expect("request history");
        assert_eq!(requests.len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BNBBTC");
    }

    #[tokio::test]
    async fn binance_spot_ticker_invalid_pair_falls_back_to_all_market_snapshot() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/spot_ticker_24hr_full.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("spot ticker fixture");
        let symbols = r#"["BNBBTC","NOTLISTEDUSDT"]"#;

        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/24hr"))
            .and(wiremock::matchers::query_param("symbols", symbols))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "code": -1121,
                "msg": "Invalid symbol."
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/24hr"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = spot_tickers(&http, &server.uri(), Some(symbols))
            .await
            .expect("all-market fallback");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BNBBTC");
    }

    #[tokio::test]
    async fn binance_spot_ticker_does_not_retry_schema_mismatch() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/24hr"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let error = spot_tickers(&http, &server.uri(), None)
            .await
            .expect_err("schema mismatch must fail closed");
        assert!(matches!(error, ExchangeError::Parse(_)));
        let requests = server.received_requests().await.expect("request history");
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn binance_spot_ticker_weight_matches_official_symbol_tiers() {
        let symbols = |count: usize| {
            serde_json::to_string(
                &(0..count)
                    .map(|index| format!("ASSET{index}USDT"))
                    .collect::<Vec<_>>(),
            )
            .expect("symbols json")
        };

        assert_eq!(spot_ticker_request_weight(Some(&symbols(1))), 2);
        assert_eq!(spot_ticker_request_weight(Some(&symbols(20))), 2);
        assert_eq!(spot_ticker_request_weight(Some(&symbols(21))), 40);
        assert_eq!(spot_ticker_request_weight(Some(&symbols(100))), 40);
        assert_eq!(spot_ticker_request_weight(Some(&symbols(101))), 80);
        assert_eq!(spot_ticker_request_weight(None), 80);
    }

    #[tokio::test]
    async fn premium_indexes_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/usdm_premium_index_btcusdt.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("premium index fixture");

        Mock::given(method("GET"))
            .and(path("/fapi/v1/premiumIndex"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = premium_indexes(&http, &server.uri())
            .await
            .expect("premium indexes request");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BTCUSDT");
    }

    #[tokio::test]
    async fn open_interest_uses_official_path_and_symbol_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/usdm_open_interest_btcusdt.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("open interest fixture");

        Mock::given(method("GET"))
            .and(path("/fapi/v1/openInterest"))
            .and(wiremock::matchers::query_param("symbol", "BTCUSDT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let row = open_interest(&http, &server.uri(), "BTCUSDT")
            .await
            .expect("open interest request");
        assert_eq!(row.symbol, "BTCUSDT");
        assert_eq!(row.open_interest, "10659.509");
    }

    #[tokio::test]
    async fn usdm_ticker_24hr_uses_official_path_without_query() {
        let server = MockServer::start().await;
        let ticker_fixture = include_str!("../../fixtures/binance/usdm_ticker_24hr_btcusdt.json");
        let ticker_body: serde_json::Value =
            serde_json::from_str(ticker_fixture).expect("usdm ticker fixture");

        Mock::given(method("GET"))
            .and(path("/fapi/v1/ticker/24hr"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(ticker_body))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fapi/v1/ticker/bookTicker"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let (tickers, books) = tickers_and_books(&http, &server.uri())
            .await
            .expect("tickers and books request");
        assert_eq!(tickers.len(), 1);
        assert_eq!(tickers[0].symbol, "BTCUSDT");
        assert!(books.is_empty());
    }
}
