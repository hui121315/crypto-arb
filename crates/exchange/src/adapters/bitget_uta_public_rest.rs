//! Bitget V3 / UTA public REST request helpers.
//!
//! Every endpoint here is documented under <https://www.bitget.com/api-doc/uta/public/...>.
//! Response envelopes (`{code, msg, requestTime, data}`) are identical between
//! V2 and V3, so we reuse `BitgetResponse` / `BitgetObjectResponse` from the
//! V2 module. Only the path and query shape changes (V3 uses `category` instead
//! of V2 `productType`).

use crate::adapter::{checked_text_with_evidence, PayloadEvidence};
use crate::adapters::bitget_response::{parse_err, BitgetObjectResponse, BitgetResponse};
use crate::adapters::bitget_uta_config::BitgetUtaCategory;
use crate::adapters::bitget_uta_market_data::{
    UtaDepthItem, UtaFundingRateItem, UtaIndexComponentsData, UtaInstrumentItem, UtaTickerItem,
};
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use reqwest::Method;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ServerTimeData {
    #[serde(default, rename = "serverTime")]
    server_time: String,
}

/// `GET /api/v2/public/time` — server epoch (ms) for time-skew calibration.
pub(super) async fn server_time(http: &HttpClient, base_url: &str) -> ExchangeResult<Option<i64>> {
    let url = format!("{base_url}/api/v2/public/time");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: BitgetObjectResponse<ServerTimeData> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    Ok(wrap
        .into_option("server time")?
        .and_then(|item| item.server_time.parse::<i64>().ok()))
}

/// `GET /api/v3/market/current-fund-rate?symbol=...`.
pub(super) async fn current_funding_rate(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<Vec<UtaFundingRateItem>> {
    let url = format!("{base_url}/api/v3/market/current-fund-rate");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    let rate_wrap: BitgetResponse<UtaFundingRateItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    rate_wrap.into_data("current-fund-rate")
}

/// `GET /api/v3/market/instruments?category=...` — online instrument specs.
pub(super) async fn instruments(
    http: &HttpClient,
    base_url: &str,
    category: BitgetUtaCategory,
) -> ExchangeResult<Vec<UtaInstrumentItem>> {
    let url = format!("{base_url}/api/v3/market/instruments");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("category", category.as_query())])
        })
        .await?;
    let wrap: BitgetResponse<UtaInstrumentItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("instruments")
}

/// `GET /api/v3/market/instruments?category=...` — full instrument rows for
/// native identity, registry ingestion and order sizing.
pub(super) async fn instruments_rest(
    http: &HttpClient,
    base_url: &str,
    category: BitgetUtaCategory,
) -> ExchangeResult<Vec<crate::adapters::bitget_instruments::BitgetInstrumentRow>> {
    let url = format!("{base_url}/api/v3/market/instruments");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("category", category.as_query())])
        })
        .await?;
    let wrap: BitgetResponse<crate::adapters::bitget_instruments::BitgetInstrumentRow> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("instruments")
}

/// `GET /api/v3/market/tickers?category=...&symbol=...` — single instrument.
pub(super) async fn ticker(
    http: &HttpClient,
    base_url: &str,
    category: BitgetUtaCategory,
    symbol: &str,
) -> ExchangeResult<Vec<UtaTickerItem>> {
    let url = format!("{base_url}/api/v3/market/tickers");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("category", category.as_query()), ("symbol", symbol)])
        })
        .await?;
    let wrap: BitgetResponse<UtaTickerItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("ticker")
}

/// `GET /api/v3/market/tickers?category=...` — full category snapshot.
pub(super) async fn tickers(
    http: &HttpClient,
    base_url: &str,
    category: BitgetUtaCategory,
) -> ExchangeResult<Vec<UtaTickerItem>> {
    let url = format!("{base_url}/api/v3/market/tickers");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("category", category.as_query())])
        })
        .await?;
    let wrap: BitgetResponse<UtaTickerItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("tickers")
}

/// `GET /api/v3/market/orderbook?category=...&symbol=...&limit=N`.
pub(super) async fn orderbook(
    http: &HttpClient,
    base_url: &str,
    category: BitgetUtaCategory,
    symbol: &str,
    limit: &str,
) -> ExchangeResult<UtaDepthItem> {
    let url = format!("{base_url}/api/v3/market/orderbook");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url).query(&[
                ("category", category.as_query()),
                ("symbol", symbol),
                ("limit", limit),
            ])
        })
        .await?;
    let wrap: BitgetObjectResponse<UtaDepthItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_result("orderbook")
}

/// `GET /api/v3/market/index-components?symbol=...`.
pub(super) async fn index_components(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<(UtaIndexComponentsData, PayloadEvidence)> {
    let url = format!("{base_url}/api/v3/market/index-components");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("symbol", symbol)]))
        .await?;
    let (body, evidence) =
        checked_text_with_evidence(resp, format!("{url}?symbol={symbol}")).await?;
    let wrap: BitgetObjectResponse<UtaIndexComponentsData> = serde_json::from_str(&body)
        .map_err(|error| crate::error::ExchangeError::Parse(format!("bitget json: {error}")))?;
    Ok((wrap.into_result("index-components")?, evidence))
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
    async fn bitget_server_time_parses_official_fixture_and_uses_no_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/bitget/server_time.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("bitget time fixture");

        Mock::given(method("GET"))
            .and(path("/api/v2/public/time"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bitget")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let server_ms = server_time(&http, &server.uri())
            .await
            .expect("bitget server time");
        assert_eq!(server_ms, Some(1_688_008_631_614));
    }

    #[tokio::test]
    async fn bitget_uta_orderbook_uses_official_path_category_symbol_limit_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/bitget/uta_orderbook_usdt_futures_btcusdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("bitget orderbook fixture");

        Mock::given(method("GET"))
            .and(path("/api/v3/market/orderbook"))
            .and(query_param("category", "USDT-FUTURES"))
            .and(query_param("symbol", "BTCUSDT"))
            .and(query_param("limit", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bitget")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let book = orderbook(
            &http,
            &server.uri(),
            BitgetUtaCategory::UsdtFutures,
            "BTCUSDT",
            "5",
        )
        .await
        .expect("bitget orderbook");
        assert_eq!(book.bids.len(), 5);
        assert_eq!(book.asks.len(), 5);
    }

    #[tokio::test]
    async fn bitget_uta_instruments_use_official_path_category_query() {
        let server = MockServer::start().await;
        let fixture =
            include_str!("../../fixtures/bitget/uta_instruments_usdt_futures_btcusdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("bitget instruments fixture");

        Mock::given(method("GET"))
            .and(path("/api/v3/market/instruments"))
            .and(query_param("category", "USDT-FUTURES"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bitget")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = instruments(&http, &server.uri(), BitgetUtaCategory::UsdtFutures)
            .await
            .expect("bitget instruments");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BTCUSDT");
    }

    #[tokio::test]
    async fn bitget_uta_current_funding_uses_official_symbol_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/bitget/uta_current_fund_rate_btcusdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("bitget funding fixture");

        Mock::given(method("GET"))
            .and(path("/api/v3/market/current-fund-rate"))
            .and(query_param("symbol", "BTCUSDT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bitget")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let rows = current_funding_rate(&http, &server.uri(), "BTCUSDT")
            .await
            .expect("bitget current funding");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BTCUSDT");
    }

    #[tokio::test]
    async fn bitget_uta_tickers_use_official_path_category_queries() {
        let server = MockServer::start().await;
        let fixture =
            include_str!("../../fixtures/bitget/uta_tickers_usdt_futures_spot_btcusdt.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("bitget tickers fixture");

        Mock::given(method("GET"))
            .and(path("/api/v3/market/tickers"))
            .and(query_param("category", "USDT-FUTURES"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body["futures"].clone()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/market/tickers"))
            .and(query_param("category", "SPOT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body["spot"].clone()))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bitget")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let futures = tickers(&http, &server.uri(), BitgetUtaCategory::UsdtFutures)
            .await
            .expect("bitget futures tickers");
        let spot = tickers(&http, &server.uri(), BitgetUtaCategory::Spot)
            .await
            .expect("bitget spot tickers");
        assert_eq!(futures[0].symbol, "BTCUSDT");
        assert_eq!(spot[0].symbol, "BTCUSDT");
    }
}
