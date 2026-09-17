use super::*;
use crate::adapters::okx_market_data::TickerItem;
use crate::adapters::okx_response::OkxResponse;
use pretty_assertions::assert_eq;

fn okx() -> Okx {
    Okx::new(OkxConfig::default()).unwrap()
}

#[test]
fn name() {
    assert_eq!(okx().name(), "okx");
}

#[test]
fn symbol_round_trip() {
    let o = okx();
    assert_eq!(o.to_exchange_symbol("BTC"), "BTC-USDT-SWAP");
    assert_eq!(o.to_exchange_symbol("ETH"), "ETH-USDT-SWAP");
    assert_eq!(o.normalize_symbol("BTC-USDT-SWAP"), "BTC");
    assert_eq!(o.normalize_symbol("ETH-USDT"), "ETH");
}

#[test]
fn okx_response_propagates_api_error() {
    let body = r#"{"code":"50001","msg":"system error","data":[]}"#;
    let wrap: OkxResponse<TickerItem> = serde_json::from_str(body).unwrap();
    let err = wrap.into_data("tickers").unwrap_err();
    assert!(matches!(err, ExchangeError::Api { .. }));
}

#[test]
fn require_credentials_errors_when_missing() {
    let o = okx();
    assert!(o.require_credentials().is_err());
}

#[tokio::test]
async fn orderbook_normalizes_contract_counts_to_base_quantity() {
    use wiremock::matchers::{method, path, query_param};

    let server = wiremock::MockServer::start().await;
    let instruments = include_str!("../../fixtures/okx/public_instruments_swap.json");
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v5/public/instruments"))
        .and(query_param("instType", "SWAP"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(instruments))
        .expect(1)
        .mount(&server)
        .await;
    let depth = include_str!("../../fixtures/okx/market_books_btc_usdt_swap.json");
    wiremock::Mock::given(method("GET"))
        .and(path("/api/v5/market/books"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .and(query_param("sz", "5"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(depth))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Okx::new(OkxConfig {
        base_url_override: Some(server.uri()),
        ..OkxConfig::default()
    })
    .expect("OKX adapter");

    let book = ExchangeAdapter::get_orderbook(&adapter, "BTC", 5)
        .await
        .expect("normalized OKX orderbook");

    assert!((book.bids[0][1] - 4.2854).abs() < 1e-12);
    assert!((book.asks[0][1] - 0.4066).abs() < 1e-12);
}
