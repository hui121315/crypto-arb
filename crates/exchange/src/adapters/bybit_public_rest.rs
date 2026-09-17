//! Bybit public REST request helpers.

use super::bybit_market_data::{IndexComponentsItem, InstrumentInfoItem, MarketTickerItem};
use super::bybit_response::{
    parse_err, BybitObjectResponse, BybitOptionalResultResponse, BybitResponse,
};
use crate::adapter::{checked_text_with_evidence, PayloadEvidence};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct OrderBookResponse {
    #[serde(default)]
    pub(super) s: String,
    #[serde(default)]
    pub(super) b: Vec<[String; 2]>,
    #[serde(default)]
    pub(super) a: Vec<[String; 2]>,
    #[serde(default)]
    pub(super) ts: i64,
}

pub(super) async fn server_time(http: &HttpClient, base_url: &str) -> ExchangeResult<i64> {
    let url = format!("{base_url}/v5/market/time");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: BybitResponse<serde_json::Value> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_server_time("server time")
}

/// 串联 `nextPageCursor` 拉全量 linear 合约元数据（Bybit 单页上限 1000）。
pub(super) async fn instruments_rest(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<crate::adapters::bybit_instruments::BybitInstrumentRow>> {
    use crate::adapters::bybit_instruments::BybitInstrumentsPage;
    let url = format!("{base_url}/v5/market/instruments-info");
    let mut rows = Vec::new();
    let mut cursor = String::new();
    loop {
        let cursor_for_query = cursor.clone();
        let resp = http
            .execute_with_retry(|| {
                let mut req = http
                    .request(Method::GET, &url)
                    .query(&[("category", "linear"), ("limit", "1000")]);
                if !cursor_for_query.is_empty() {
                    req = req.query(&[("cursor", cursor_for_query.as_str())]);
                }
                req
            })
            .await?;
        let wrap: BybitObjectResponse<BybitInstrumentsPage> =
            resp.json().await.map_err(|error| parse_err(&error))?;
        let page = wrap.into_result("instruments-info")?;
        rows.extend(page.list);
        if page.next_page_cursor.is_empty() {
            break;
        }
        cursor = page.next_page_cursor;
    }
    Ok(rows)
}

pub(super) async fn funding_rate(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<(MarketTickerItem, i64)> {
    let mut items = market_tickers(http, base_url, Some(symbol)).await?;
    let item = items
        .rows
        .pop()
        .ok_or_else(|| ExchangeError::Parse("bybit ticker empty".into()))?;
    Ok((item, items.server_time_ms))
}

pub(super) async fn funding_rates_and_instruments(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<(Vec<MarketTickerItem>, i64, Vec<InstrumentInfoItem>)> {
    let tickers_url = format!("{base_url}/v5/market/tickers");
    let info_url = format!("{base_url}/v5/market/instruments-info");
    let (tickers_resp, info_resp) = tokio::try_join!(
        http.execute_with_retry(|| http
            .request(Method::GET, &tickers_url)
            .query(&[("category", "linear")])),
        http.execute_with_retry(|| http
            .request(Method::GET, &info_url)
            .query(&[("category", "linear")])),
    )?;
    let tickers_wrap: BybitResponse<MarketTickerItem> = tickers_resp
        .json()
        .await
        .map_err(|error| parse_err(&error))?;
    let (tickers, server_time_ms) = tickers_wrap.into_list_with_time("tickers")?;
    let info_wrap: BybitResponse<InstrumentInfoItem> =
        info_resp.json().await.map_err(|error| parse_err(&error))?;
    Ok((
        tickers,
        server_time_ms,
        info_wrap.into_list("instruments-info")?,
    ))
}

pub(super) async fn ticker(
    http: &HttpClient,
    base_url: &str,
    symbol: &str,
) -> ExchangeResult<MarketTickerItem> {
    let mut items = market_tickers(http, base_url, Some(symbol)).await?;
    items
        .rows
        .pop()
        .ok_or_else(|| ExchangeError::Parse("bybit ticker empty".into()))
}

pub(super) async fn tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<MarketTickerItem>> {
    Ok(market_tickers(http, base_url, None).await?.rows)
}

pub(super) async fn spot_tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<(Vec<MarketTickerItem>, i64)> {
    let url = format!("{base_url}/v5/market/tickers");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("category", "spot")])
        })
        .await?;
    let wrap: BybitResponse<MarketTickerItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_list_with_time("spot tickers")
}

pub(super) async fn index_components(
    http: &HttpClient,
    base_url: &str,
    index_name: &str,
) -> ExchangeResult<(IndexComponentsItem, PayloadEvidence)> {
    let url = format!("{base_url}/v5/market/index-price-components");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("indexName", index_name)])
        })
        .await?;
    let (body, evidence) =
        checked_text_with_evidence(resp, format!("{url}?indexName={index_name}")).await?;
    let wrap: BybitObjectResponse<IndexComponentsItem> = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("bybit json: {error}")))?;
    Ok((wrap.into_result("index price components")?, evidence))
}

pub(super) async fn orderbook(
    http: &HttpClient,
    base_url: &str,
    category: &str,
    symbol: &str,
    limit: &str,
    context: &str,
) -> ExchangeResult<OrderBookResponse> {
    let url = format!("{base_url}/v5/market/orderbook");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url).query(&[
                ("category", category),
                ("symbol", symbol),
                ("limit", limit),
            ])
        })
        .await?;
    let wrap: BybitOptionalResultResponse<OrderBookResponse> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_result(context)
}

struct MarketTickers {
    rows: Vec<MarketTickerItem>,
    server_time_ms: i64,
}

async fn market_tickers(
    http: &HttpClient,
    base_url: &str,
    symbol: Option<&str>,
) -> ExchangeResult<MarketTickers> {
    let url = format!("{base_url}/v5/market/tickers");
    let resp = http
        .execute_with_retry(|| {
            let mut req = http
                .request(Method::GET, &url)
                .query(&[("category", "linear")]);
            if let Some(symbol) = symbol {
                req = req.query(&[("symbol", symbol)]);
            }
            req
        })
        .await?;
    let wrap: BybitResponse<MarketTickerItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    let (rows, server_time_ms) = wrap.into_list_with_time("tickers")?;
    Ok(MarketTickers {
        rows,
        server_time_ms,
    })
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

    struct MissingQueryParam(&'static str);

    impl Match for MissingQueryParam {
        fn matches(&self, request: &Request) -> bool {
            !request.url.query_pairs().any(|(key, _)| key == self.0)
        }
    }

    #[tokio::test]
    async fn bybit_server_time_parses_official_fixture_and_uses_no_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/bybit/server_time.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("bybit time fixture");

        Mock::given(method("GET"))
            .and(path("/v5/market/time"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bybit")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let server_ms = server_time(&http, &server.uri())
            .await
            .expect("bybit server time");
        assert_eq!(server_ms, 1_672_364_174_910);
    }

    #[tokio::test]
    async fn funding_rates_and_instruments_uses_linear_instruments_query() {
        let server = MockServer::start().await;
        let instruments_fixture =
            include_str!("../../fixtures/bybit/instruments_info_linear_btcusdt.json");
        let instruments_body: serde_json::Value =
            serde_json::from_str(instruments_fixture).expect("bybit instruments fixture");
        let tickers_body = serde_json::json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "category": "linear",
                "list": []
            },
            "retExtInfo": {},
            "time": 1_780_435_735_479_i64
        });

        Mock::given(method("GET"))
            .and(path("/v5/market/tickers"))
            .and(query_param("category", "linear"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tickers_body))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v5/market/instruments-info"))
            .and(query_param("category", "linear"))
            .respond_with(ResponseTemplate::new(200).set_body_json(instruments_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bybit")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let (tickers, server_time_ms, instruments) =
            funding_rates_and_instruments(&http, &server.uri())
                .await
                .expect("bybit funding and instruments");
        assert!(tickers.is_empty());
        assert_eq!(server_time_ms, 1_780_435_735_479);
        assert_eq!(instruments.len(), 1);
        assert_eq!(instruments[0].symbol, "BTCUSDT");
        assert_eq!(instruments[0].funding_interval, 480);
    }

    #[tokio::test]
    async fn bybit_market_tickers_uses_linear_and_spot_queries() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/bybit/market_tickers_linear_spot_btcusdt.json");
        let body: MarketTickersFixtureBodies =
            serde_json::from_str(fixture).expect("bybit market tickers fixture");

        Mock::given(method("GET"))
            .and(path("/v5/market/tickers"))
            .and(query_param("category", "linear"))
            .and(query_param("symbol", "BTCUSDT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.linear.clone()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v5/market/tickers"))
            .and(query_param("category", "linear"))
            .and(MissingQueryParam("symbol"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.linear))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v5/market/tickers"))
            .and(query_param("category", "spot"))
            .and(MissingQueryParam("symbol"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.spot))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bybit")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let single = ticker(&http, &server.uri(), "BTCUSDT")
            .await
            .expect("bybit symbol ticker");
        let linear_rows = tickers(&http, &server.uri())
            .await
            .expect("bybit linear tickers");
        let (spot_rows, spot_server_time_ms) = spot_tickers(&http, &server.uri())
            .await
            .expect("bybit spot tickers");

        assert_eq!(single.symbol, "BTCUSDT");
        assert_eq!(linear_rows.len(), 1);
        assert_eq!(linear_rows[0].funding_rate, "-0.00000183");
        assert_eq!(spot_rows.len(), 1);
        assert_eq!(spot_rows[0].symbol, "BTCUSDT");
        assert_eq!(spot_server_time_ms, 1_780_440_271_831);
    }

    #[test]
    fn bybit_orderbook_parses_official_linear_fixture() {
        let fixture = include_str!("../../fixtures/bybit/orderbook_linear_btcusdt.json");
        let response: BybitOptionalResultResponse<OrderBookResponse> =
            serde_json::from_str(fixture).expect("bybit orderbook fixture");
        let book = response.into_result("orderbook").expect("orderbook");

        assert_eq!(book.s, "BTCUSDT");
        assert_eq!(book.a.len(), 5);
        assert_eq!(book.b.len(), 5);
        assert_eq!(book.a[0], ["67789.00".to_owned(), "0.855".to_owned()]);
        assert_eq!(book.b[0], ["67788.90".to_owned(), "6.776".to_owned()]);
        assert_eq!(book.ts, 1_780_436_417_336);
    }

    #[tokio::test]
    async fn bybit_orderbook_rest_uses_linear_symbol_limit_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/bybit/orderbook_linear_btcusdt.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("bybit book fixture");

        Mock::given(method("GET"))
            .and(path("/v5/market/orderbook"))
            .and(query_param("category", "linear"))
            .and(query_param("symbol", "BTCUSDT"))
            .and(query_param("limit", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("bybit")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let book = orderbook(&http, &server.uri(), "linear", "BTCUSDT", "5", "orderbook")
            .await
            .expect("bybit orderbook");
        assert_eq!(book.s, "BTCUSDT");
        assert_eq!(book.b.len(), 5);
        assert_eq!(book.a.len(), 5);
        assert_eq!(book.ts, 1_780_436_417_336);
    }

    #[derive(Deserialize)]
    struct MarketTickersFixtureBodies {
        linear: serde_json::Value,
        spot: serde_json::Value,
    }
}
