#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use exchange::{Okx, OkxConfig, OkxCredentials};
use serde_json::json;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn okx_account_mode_reads_signed_account_config_without_demo_header() {
    let server = MockServer::start().await;
    mock_public_time(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v5/account/config"))
        .and(header("OK-ACCESS-KEY", "k"))
        .and(header_exists("OK-ACCESS-SIGN"))
        .and(header_exists("OK-ACCESS-TIMESTAMP"))
        .and(header("OK-ACCESS-PASSPHRASE", "p"))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
            "../fixtures/okx/account_config_long_short.json"
        )))
        .expect(1)
        .mount(&server)
        .await;

    let adapter = okx(server.uri());
    let info = adapter
        .get_exchange_account_mode("okx")
        .await
        .expect("account config request succeeds")
        .expect("account mode evidence exists");

    assert_eq!(info.venue, "okx");
    assert_eq!(info.mode, "long_short_mode");
    assert_eq!(info.source, "okx.GET /api/v5/account/config");
    assert_eq!(info.account_scope, None);

    let requests = server.received_requests().await.expect("recorded requests");
    let account_config = requests
        .iter()
        .find(|request| request.url.path() == "/api/v5/account/config")
        .expect("account config request recorded");
    assert!(
        account_config.headers.get("x-simulated-trading").is_none(),
        "credential validation must use the production account-config header shape"
    );
}

fn okx(server_uri: String) -> Okx {
    Okx::new(OkxConfig {
        credentials: Some(OkxCredentials {
            api_key: "k".into(),
            api_secret: "s".into(),
            passphrase: "p".into(),
        }),
        timeout_secs: 5,
        qps: 100,
        base_url_override: Some(server_uri),
    })
    .expect("okx adapter")
}

async fn mock_public_time(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v5/public/time"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "0",
            "msg": "",
            "data": [{"ts": "1700000000000"}]
        })))
        .mount(server)
        .await;
}
