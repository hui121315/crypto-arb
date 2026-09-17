use super::*;
use crate::{
    adapters::kraken::{KrakenConfig, KrakenCredentials, KrakenSpotCredentials},
    ExchangeAdapter,
};
use serde_json::{json, Value};
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn native(asset: &str, class: &str) -> Value {
    json!({"asset":{"class":class,"name":asset},"method_id":format!("local-{asset}"),
        "minimum_amount":"0.001","maximum_amount":"1000","fees":{"base":{"asset":{"class":class,"name":asset},"amount":"0.000123456789"},
        "included":true,"percentage":"0.01","min":{"asset":{"class":"currency","name":"USDC"},"amount":"0.01"}},
        "network":{"network_id":"local-solana","network_name":"Solana","contract_address":if asset=="USDC"{comparison::SOLANA_USDC}else{"different-issuer-mint"}}})
}

#[test]
fn stock_funding_preserves_fee_asset_and_rejects_wrong_classes_duplicates_or_bad_limits() {
    let row = native("MUx", "tokenized_asset");
    let methods = project(
        vec![serde_json::from_value(row.clone()).unwrap()],
        "MUx",
        "tokenized_asset",
    )
    .unwrap();
    assert_eq!(methods[0].fees.base.amount, "0.000123456789");
    assert_eq!(methods[0].fees.minimum.as_ref().unwrap().asset, "USDC");
    for case in 0..5 {
        let mut bad = row.clone();
        match case {
            0 => bad["asset"]["class"] = json!("currency"),
            1 => bad["asset"]["name"] = json!("MUX"),
            2 => bad["fees"]["base"]["amount"] = json!("-0.1"),
            3 => bad["maximum_amount"] = json!("0.0001"),
            _ => bad["network"] = Value::Null,
        }
        assert!(
            project(
                vec![serde_json::from_value(bad).unwrap()],
                "MUx",
                "tokenized_asset"
            )
            .is_err(),
            "case {case}"
        );
    }
    assert!(project(
        vec![
            serde_json::from_value(row.clone()).unwrap(),
            serde_json::from_value(row).unwrap()
        ],
        "MUx",
        "tokenized_asset"
    )
    .is_err());
}

#[tokio::test]
async fn stock_funding_signed_gets_use_base_units_isolate_unknown_direction_and_never_create_address(
) {
    let server = MockServer::start().await;
    for (asset, class) in [("MUx", "tokenized_asset"), ("USDC", "currency")] {
        for direction in ["deposit", "withdraw"] {
            let body = if asset == "USDC" && direction == "withdraw" {
                json!({"methods":[{"incomplete":true}]})
            } else {
                json!({"methods":[native(asset,class)]})
            };
            Mock::given(method("GET"))
                .and(path(format!("/funding/v1/methods/{direction}")))
                .and(query_param("asset[class]", class))
                .and(query_param("asset[name]", asset))
                .and(query_param("rebase_multiplier", "base"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(&server)
                .await;
        }
    }
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
    let rows = adapter.stock_funding_methods("MUx/USD").await.unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows[..3]
        .iter()
        .all(|r| r.problem.is_none() && r.methods.len() == 1 && r.amount_unit == "base"));
    assert!(rows[3].problem.is_some() && rows[3].methods.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    for r in requests {
        assert_eq!(r.method, "GET");
        assert!(r.body.is_empty());
        let signed_path = format!("{}?{}", r.url.path(), r.url.query().unwrap());
        let nonce = r.headers.get("API-Nonce").unwrap().to_str().unwrap();
        let expected =
            crate::signing::kraken::spot_rest_sign("c2VjcmV0", &signed_path, nonce, "").unwrap();
        assert_eq!(
            r.headers.get("API-Sign").unwrap().to_str().unwrap(),
            expected
        );
    }
    if let Ok(path) = std::env::var("STOCK_PEER_FUNDING_ADAPTER_CAPTURE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
}
