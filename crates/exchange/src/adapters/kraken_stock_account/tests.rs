use super::*;
use crate::{
    adapters::kraken::{KrakenConfig, KrakenCredentials, KrakenSpotCredentials},
    ExchangeAdapter,
};
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

#[test]
fn stock_cash_decimal_inventory_does_not_count_credit_or_earn_and_fee_is_exact_pair() {
    let b = json!({"MUx":{"balance":"2.123456789012345678","hold_trade":"0.1","credit":"999","credit_used":"0.2"},"MUx.F":{"balance":"100","hold_trade":"0"}});
    assert_eq!(cash(&b, "MUx").as_deref(), Some("1.823456789012345678"));
    assert!(cash(&b, "USDC").is_none());
    let mut bad = b.clone();
    bad["MUx"]["hold_trade"] = json!("-1");
    assert!(cash(&bad, "MUx").is_none());
    let f = json!({"fees":{"MUx/USD":{"fee":"0.1000"}},"fees_maker":{"MUx/USD":{"fee":"-0.02"}}});
    assert_eq!(fee(&f, "MUx/USD").as_deref(), Some("0.1"));
    assert!(fee(&f, "MUx/USDT").is_none());
    assert!(result(r#"{"result":{}}"#).is_err());
}

#[tokio::test]
async fn stock_cash_signed_reads_use_equity_class_rebased_units_and_never_order() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/0/public/AssetPairs")).and(query_param("pair","MUx/USD"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":{"MUx/USD":{"wsname":"MUx/USD","base":"MUx","quote":"ZUSD","aclass_base":"tokenized_asset","status":"online"}}}))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/0/public/AssetPairs")).and(query_param("pair","USDC/USD"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":{"USDCUSD":{"wsname":"USDC/USD","base":"USDC","quote":"ZUSD","status":"online"}}}))).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/0/private/TradeVolume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"error":[],"result":{"fees":{"MUx/USD":{"fee":"0.1"},"USDCUSD":{"fee":"0.2"}}}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/0/private/BalanceEx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":{"MUx":{"balance":"2","hold_trade":"0.1","credit":"999"},"ZUSD":{"balance":"20","hold_trade":"3"},"USDC":{"balance":"10","hold_trade":"1","credit_used":"2"}}}))).expect(1).mount(&server).await;
    let adapter = Kraken::new(KrakenConfig {
        spot_rest_url_override: Some(server.uri()),
        credentials: Some(KrakenCredentials {
            spot: Some(KrakenSpotCredentials {
                api_key: "local-fixture".into(),
                api_secret: "c2VjcmV0".into(),
            }),
            futures: None,
        }),
        allow_live_writes: false,
        ..Default::default()
    })
    .unwrap();
    let a = adapter.stock_cash_account("MUx/USD").await.unwrap();
    assert_eq!(a.stock_available.as_deref(), Some("1.9"));
    assert_eq!(a.usdc_available.as_deref(), Some("7"));
    assert_eq!(a.quote_available.as_deref(), Some("17"));
    assert_eq!(a.stock_taker_pct.as_deref(), Some("0.1"));
    assert_eq!(a.fx_taker_pct.as_deref(), Some("0.2"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    for req in requests.iter().filter(|r| r.method == "POST") {
        let body: Value = req.body_json().unwrap();
        assert_eq!(body["rebase_multiplier"], "rebased");
        let signature = crate::signing::kraken::spot_rest_sign(
            "c2VjcmV0",
            req.url.path(),
            &body["nonce"].to_string(),
            std::str::from_utf8(&req.body).unwrap(),
        )
        .unwrap();
        assert_eq!(
            req.headers.get("API-Sign").unwrap().to_str().unwrap(),
            signature
        );
        if req.url.path().ends_with("TradeVolume") {
            assert_eq!(
                body["pair"],
                json!([{"asset":"MUx/USD","aclass":"equity_pair"},{"asset":"USDC/USD","aclass":"forex"}])
            );
        }
    }
}
