//! OKX public REST request helpers.

use crate::adapter::{checked_text_with_evidence, PayloadEvidence};
use crate::adapters::okx_funding::FundingRateItem;
use crate::adapters::okx_mark_index_data::{
    parse_mark_index_rows, OkxIndexTickerItem, OkxMarkPriceItem, OkxOpenInterestItem,
};
use crate::adapters::okx_market_data::{
    swap_inst_ids, IndexComponentsItem, InstrumentItem, OrderBookItem, TickerItem,
};
use crate::adapters::okx_response::{parse_err, OkxObjectResponse, OkxResponse};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use reqwest::Method;
use serde::Deserialize;
use shared_types::MarkIndexInfo;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
struct TimeItem {
    ts: String,
}

pub(super) async fn server_time(http: &HttpClient, base_url: &str) -> ExchangeResult<i64> {
    let url = format!("{base_url}/api/v5/public/time");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url))
        .await?;
    let wrap: OkxResponse<TimeItem> = resp.json().await.map_err(|error| parse_err(&error))?;
    let mut items = wrap.into_data("public time")?;
    let item = items
        .pop()
        .ok_or_else(|| ExchangeError::Parse("okx public time empty".into()))?;
    item.ts
        .parse::<i64>()
        .map_err(|error| ExchangeError::Parse(format!("okx public time ts: {error}")))
}

pub(super) async fn swap_inst_ids_rest(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<String>> {
    let url = format!("{base_url}/api/v5/public/instruments");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instType", "SWAP")])
        })
        .await?;
    let wrap: OkxResponse<InstrumentItem> = resp.json().await.map_err(|error| parse_err(&error))?;
    Ok(swap_inst_ids(wrap.into_data("instruments")?))
}

/// 拉取官方 `instruments`(SWAP) 全量行（含 ctVal/lotSz/minSz/tickSz/state），
/// 供 instrument registry 启动期/周期刷新映射为 `VenueInstrument`。
pub(super) async fn swap_instruments_rest(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<crate::adapters::okx_instruments::OkxInstrumentRow>> {
    let url = format!("{base_url}/api/v5/public/instruments");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instType", "SWAP")])
        })
        .await?;
    let wrap: OkxResponse<crate::adapters::okx_instruments::OkxInstrumentRow> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("instruments")
}

pub(super) async fn funding_rate(
    http: &HttpClient,
    base_url: &str,
    inst_id: &str,
) -> ExchangeResult<FundingRateItem> {
    let url = format!("{base_url}/api/v5/public/funding-rate");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instId", inst_id)])
        })
        .await?;
    funding_rate_from_response(resp, inst_id).await
}

pub(super) async fn funding_rate_fast(
    http: &HttpClient,
    base_url: &str,
    inst_id: &str,
) -> ExchangeResult<FundingRateItem> {
    let url = format!("{base_url}/api/v5/public/funding-rate");
    if let Some(rate_limiter) = http.rate_limiter() {
        rate_limiter.wait().await;
    }
    let resp = http
        .request(Method::GET, &url)
        .query(&[("instId", inst_id)])
        .send()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if !resp.status().is_success() {
        return Err(ExchangeError::Http {
            status: resp.status().as_u16(),
            body: String::new(),
        });
    }
    funding_rate_from_response(resp, inst_id).await
}

pub(super) async fn swap_tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<TickerItem>> {
    tickers(http, base_url, "SWAP", "tickers").await
}

pub(super) async fn spot_tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<TickerItem>> {
    tickers(http, base_url, "SPOT", "spot tickers").await
}

pub(super) async fn mark_index_prices(
    http: &HttpClient,
    base_url: &str,
    inst_ids: Option<&[String]>,
) -> ExchangeResult<Vec<MarkIndexInfo>> {
    let requested = inst_ids.map(|rows| rows.iter().cloned().collect::<HashSet<_>>());
    let (marks, indexes, open_interest) = tokio::try_join!(
        mark_prices(http, base_url),
        index_tickers(http, base_url),
        open_interest(http, base_url),
    )?;
    Ok(parse_mark_index_rows(
        marks,
        indexes,
        open_interest,
        requested.as_ref(),
    ))
}

pub(super) async fn ticker(
    http: &HttpClient,
    base_url: &str,
    inst_id: &str,
) -> ExchangeResult<TickerItem> {
    let url = format!("{base_url}/api/v5/market/ticker");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instId", inst_id)])
        })
        .await?;
    let wrap: OkxResponse<TickerItem> = resp.json().await.map_err(|error| parse_err(&error))?;
    let mut items = wrap.into_data("ticker")?;
    items
        .pop()
        .ok_or_else(|| ExchangeError::Parse("okx ticker empty".into()))
}

pub(super) async fn orderbook(
    http: &HttpClient,
    base_url: &str,
    inst_id: &str,
    size: &str,
    context: &str,
) -> ExchangeResult<OrderBookItem> {
    let url = format!("{base_url}/api/v5/market/books");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instId", inst_id), ("sz", size)])
        })
        .await?;
    let wrap: OkxResponse<OrderBookItem> = resp.json().await.map_err(|error| parse_err(&error))?;
    let mut items = wrap.into_data(context)?;
    items
        .pop()
        .ok_or_else(|| ExchangeError::Parse(format!("okx {context} empty")))
}

pub(super) async fn index_components(
    http: &HttpClient,
    base_url: &str,
    index: &str,
) -> ExchangeResult<(IndexComponentsItem, PayloadEvidence)> {
    let url = format!("{base_url}/api/v5/market/index-components");
    let resp = http
        .execute_with_retry(|| http.request(Method::GET, &url).query(&[("index", index)]))
        .await?;
    let (body, evidence) = checked_text_with_evidence(resp, format!("{url}?index={index}")).await?;
    let wrap: OkxObjectResponse<IndexComponentsItem> = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("okx json: {error}")))?;
    Ok((wrap.into_result("index-components")?, evidence))
}

async fn tickers(
    http: &HttpClient,
    base_url: &str,
    inst_type: &str,
    context: &str,
) -> ExchangeResult<Vec<TickerItem>> {
    let url = format!("{base_url}/api/v5/market/tickers");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instType", inst_type)])
        })
        .await?;
    let wrap: OkxResponse<TickerItem> = resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data(context)
}

async fn mark_prices(http: &HttpClient, base_url: &str) -> ExchangeResult<Vec<OkxMarkPriceItem>> {
    let url = format!("{base_url}/api/v5/public/mark-price");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instType", "SWAP")])
        })
        .await?;
    let wrap: OkxResponse<OkxMarkPriceItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("mark-price")
}

async fn index_tickers(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<OkxIndexTickerItem>> {
    let url = format!("{base_url}/api/v5/market/index-tickers");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("quoteCcy", "USDT")])
        })
        .await?;
    let wrap: OkxResponse<OkxIndexTickerItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("index-tickers")
}

async fn open_interest(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<OkxOpenInterestItem>> {
    let url = format!("{base_url}/api/v5/public/open-interest");
    let resp = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .query(&[("instType", "SWAP")])
        })
        .await?;
    let wrap: OkxResponse<OkxOpenInterestItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    wrap.into_data("open-interest")
}

async fn funding_rate_from_response(
    resp: reqwest::Response,
    inst_id: &str,
) -> ExchangeResult<FundingRateItem> {
    let wrap: OkxResponse<FundingRateItem> =
        resp.json().await.map_err(|error| parse_err(&error))?;
    let mut items = wrap.into_data("funding-rate")?;
    items.pop().ok_or_else(|| {
        ExchangeError::Parse(format!("okx funding-rate empty response for {inst_id}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::okx_market_data::orderbook_info;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    struct NoQueryParams;

    impl Match for NoQueryParams {
        fn matches(&self, request: &Request) -> bool {
            request.url.query().is_none()
        }
    }

    fn public_fixture_branch(branch: &str) -> serde_json::Value {
        let fixture =
            include_str!("../../fixtures/okx/public_funding_mark_index_oi_btc_eth_usdt.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("okx public fixture");
        body.get(branch)
            .unwrap_or_else(|| panic!("missing okx fixture branch {branch}"))
            .clone()
    }

    #[tokio::test]
    async fn public_time_parses_official_fixture_and_uses_no_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/okx/public_time.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("okx time fixture");

        Mock::given(method("GET"))
            .and(path("/api/v5/public/time"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("okx")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let server_ms = server_time(&http, &server.uri())
            .await
            .expect("okx public time");
        assert_eq!(server_ms, 1_597_026_383_085);
    }

    #[tokio::test]
    async fn swap_inst_ids_rest_parses_official_fixture_and_uses_swap_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/okx/public_instruments_swap.json");
        let body: serde_json::Value =
            serde_json::from_str(fixture).expect("okx instruments fixture");

        Mock::given(method("GET"))
            .and(path("/api/v5/public/instruments"))
            .and(query_param("instType", "SWAP"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("okx")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let inst_ids = swap_inst_ids_rest(&http, &server.uri())
            .await
            .expect("okx instruments");
        assert_eq!(inst_ids, vec!["BTC-USDT-SWAP".to_owned()]);
    }

    #[tokio::test]
    async fn okx_orderbook_rest_parses_official_fixture_and_uses_inst_id_sz_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/okx/market_books_btc_usdt_swap.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("okx books fixture");

        Mock::given(method("GET"))
            .and(path("/api/v5/market/books"))
            .and(query_param("instId", "BTC-USDT-SWAP"))
            .and(query_param("sz", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("okx")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let row = orderbook(&http, &server.uri(), "BTC-USDT-SWAP", "5", "books")
            .await
            .expect("okx orderbook");
        let parsed = orderbook_info("BTC".into(), row);
        assert_eq!(parsed.bids.len(), 5);
        assert_eq!(parsed.asks.len(), 5);
        assert!((parsed.bids[0][0] - 67_870.3).abs() < 1e-9);
    }

    #[tokio::test]
    async fn okx_market_tickers_use_inst_type_queries() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/okx/market_tickers_swap_spot_btc_eth_usdt.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("okx tickers fixture");
        let swap_body = body
            .get("swap")
            .expect("swap tickers fixture branch")
            .clone();
        let spot_body = body
            .get("spot")
            .expect("spot tickers fixture branch")
            .clone();

        Mock::given(method("GET"))
            .and(path("/api/v5/market/tickers"))
            .and(query_param("instType", "SWAP"))
            .respond_with(ResponseTemplate::new(200).set_body_json(swap_body))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v5/market/tickers"))
            .and(query_param("instType", "SPOT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(spot_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("okx")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let swap_rows = swap_tickers(&http, &server.uri())
            .await
            .expect("okx swap tickers");
        let spot_rows = spot_tickers(&http, &server.uri())
            .await
            .expect("okx spot tickers");
        assert_eq!(swap_rows.len(), 2);
        assert_eq!(spot_rows.len(), 2);
        assert!(swap_rows.iter().all(|row| row.inst_type() == Some("SWAP")));
        assert!(spot_rows.iter().all(|row| row.inst_type() == Some("SPOT")));
    }

    #[tokio::test]
    async fn okx_funding_rate_uses_official_path_and_inst_id_query() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v5/public/funding-rate"))
            .and(query_param("instId", "BTC-USDT-SWAP"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(public_fixture_branch("funding_rate")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("okx")
            .timeout_secs(5)
            .build()
            .expect("http client");

        let row = funding_rate(&http, &server.uri(), "BTC-USDT-SWAP")
            .await
            .expect("okx funding rate");
        let parsed =
            crate::adapters::okx_funding::parse_funding(&row, 0.0).expect("funding parses");
        assert_eq!(parsed.symbol, "BTC");
        assert_eq!(parsed.funding_interval, 8);
    }

    #[tokio::test]
    async fn okx_mark_index_prices_use_official_public_queries() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v5/public/mark-price"))
            .and(query_param("instType", "SWAP"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(public_fixture_branch("mark_price")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v5/market/index-tickers"))
            .and(query_param("quoteCcy", "USDT"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(public_fixture_branch("index_tickers")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v5/public/open-interest"))
            .and(query_param("instType", "SWAP"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(public_fixture_branch("open_interest")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("okx")
            .timeout_secs(5)
            .build()
            .expect("http client");
        let requested = vec!["BTC-USDT-SWAP".to_owned()];

        let rows = mark_index_prices(&http, &server.uri(), Some(&requested))
            .await
            .expect("okx mark/index rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "BTC");
        assert_eq!(rows[0].mark_price, 66_339.1);
        assert_eq!(rows[0].index_price, Some(66_381.6));
        assert_eq!(rows[0].open_interest_value, Some(2_508_003_326.787_561));
    }
}
