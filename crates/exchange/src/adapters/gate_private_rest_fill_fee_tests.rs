use super::*;

fn test_headers() -> SignedHeaders {
    [
        ("KEY".to_owned(), "k".to_owned()),
        ("Timestamp".to_owned(), "1700000000".to_owned()),
        ("SIGN".to_owned(), "s".to_owned()),
    ]
}

fn gate_http() -> HttpClient {
    HttpClient::builder("gate")
        .timeout_secs(5)
        .build()
        .expect("http client")
}

#[test]
fn my_trades_query_accepts_only_positive_numeric_order_id() {
    assert_eq!(
        my_trades_query("21893289839").expect("numeric order id"),
        "order=21893289839"
    );
    assert!(my_trades_query("t-client-id").is_err());
    assert!(my_trades_query("0").is_err());
}

#[tokio::test]
async fn my_trades_uses_signed_order_query_and_official_fixture() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/v4/futures/usdt/my_trades"))
        .and(wiremock::matchers::query_param("order", "21893289839"))
        .and(wiremock::matchers::header("KEY", "k"))
        .and(wiremock::matchers::header("Timestamp", "1700000000"))
        .and(wiremock::matchers::header("SIGN", "s"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_string(include_str!(
                "../../fixtures/gate/futures_usdt_my_trades_order.json"
            )),
        )
        .expect(1)
        .mount(&server)
        .await;
    let http = gate_http();
    let headers = test_headers();
    let query = my_trades_query("21893289839").expect("query");
    let request = SignedRequest {
        http: &http,
        base_url: &server.uri(),
        path: "/api/v4/futures/usdt/my_trades",
        query: &query,
        headers: &headers,
    };

    let rows = my_trades(&request, "usdt", "21893289839")
        .await
        .expect("signed fills");

    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn fee_read_uses_signed_endpoint_and_preserves_rebate() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/v4/futures/usdt/fee"))
        .and(wiremock::matchers::header("KEY", "k"))
        .and(wiremock::matchers::header("Timestamp", "1700000000"))
        .and(wiremock::matchers::header("SIGN", "s"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_string(include_str!("../../fixtures/gate/futures_usdt_fee.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let http = gate_http();
    let headers = test_headers();
    let request = SignedRequest {
        http: &http,
        base_url: &server.uri(),
        path: "/api/v4/futures/usdt/fee",
        query: "",
        headers: &headers,
    };

    let rows = futures_fee_evidence(&request, "usdt", 1_700_000_000_000)
        .await
        .expect("signed fee evidence");

    assert!(rows.iter().any(|row| row.maker_fee_rate < 0.0));
}

#[tokio::test]
async fn private_reads_reject_unsigned_query_or_path_drift_before_io() {
    let http = gate_http();
    let headers = test_headers();
    let request = SignedRequest {
        http: &http,
        base_url: "http://127.0.0.1:1",
        path: "/api/v4/futures/usdt/my_trades",
        query: "order=t-client-id",
        headers: &headers,
    };

    let error = my_trades(&request, "usdt", "21893289839")
        .await
        .expect_err("query mismatch must fail before request");

    assert!(error.to_string().contains("signed private read mismatch"));
}
