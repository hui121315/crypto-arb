#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use exchange::{ExchangeAdapter, Okx, OkxConfig};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn execution_depth_orderbook_uses_rest_books_depth_not_books5_ws() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v5/public/instruments"))
        .and(query_param("instType", "SWAP"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../fixtures/okx/public_instruments_swap.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v5/market/books"))
        .and(query_param("instId", "BTC-USDT-SWAP"))
        .and(query_param("sz", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "msg": "",
            "data": [{
                "asks": [["41006.8", "0.60038921", "0", "1"]],
                "bids": [["41006.3", "0.30178218", "0", "2"]],
                "ts": "1629966436396",
                "seqId": "3235851742"
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let adapter = okx(server.uri());
    let book = adapter
        .get_orderbook("BTC", 20)
        .await
        .expect("REST orderbook depth response parses");

    assert_eq!(book.symbol, "BTC");
    assert_eq!(book.exchange, "okx");
    assert!((book.bids[0][0] - 41006.3).abs() < 1e-9);
    assert!((book.bids[0][1] - 0.003_017_821_8).abs() < 1e-12);
    assert!((book.asks[0][0] - 41006.8).abs() < 1e-9);
    assert!((book.asks[0][1] - 0.006_003_892_1).abs() < 1e-12);
}

fn okx(server_uri: String) -> Okx {
    Okx::new(OkxConfig {
        credentials: None,
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
    })
    .expect("okx adapter")
}
