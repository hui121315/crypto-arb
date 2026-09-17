use super::*;
use crate::adapter::ExchangeAdapter;
use crate::adapters::{Kraken, KrakenConfig, KrakenCredentials};
use crate::LiveTradingAdapter;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const METHOD_ID: &str = "3e7f8072-cc6d-4394-982a-5f4ca6ab27dd";
const NETWORK_ID: &str = "b336ce74-8d60-42b8-8714-b1095e06b711";
const REF_ID: &str = "FTcQ4qW-fWGQbQwUfqdnZo4dsMn1ao";

fn credentials() -> KrakenSpotCredentials {
    KrakenSpotCredentials {
        api_key: "fixture-key".into(),
        api_secret: "c2VjcmV0".into(),
    }
}
fn client() -> HttpClient {
    HttpClient::builder("kraken-fixture")
        .max_retries(1)
        .build()
        .unwrap()
}
fn funding_method() -> Value {
    json!({"asset":{"class":"currency","name":"USDC"},"method_id":METHOD_ID,
        "method_name":"USDC - Solana", "minimum_amount":"1", "maximum_amount":"100",
        "fees":{"base":{"asset":{"class":"currency","name":"USDC"},"amount":"0"},"included":true},
        "network":{"network_id":NETWORK_ID,"network_name":"Solana","contract_address":"CaseSensitiveMint"}})
}
fn address_request() -> TransferDestinationRequest {
    TransferDestinationRequest {
        currency: "USDC".into(),
        network: METHOD_ID.into(),
        direction: TransferDirection::DepositToVenue,
        expected_address: None,
        expected_tag: None,
        amount: Some(Decimal::new(125, 1)),
    }
}
fn address_row() -> Value {
    json!({"method_id":METHOD_ID,"address_details":{"crypto":{"address":"SolanaDepositAddress"}}})
}
fn request(now: i64) -> DepositStatusRequest {
    DepositStatusRequest {
        venue: "kraken".into(),
        currency: "USDC".into(),
        network: METHOD_ID.into(),
        address: "SolanaDepositAddress".into(),
        tag: None,
        transaction_id: "SolanaTxSignature".into(),
        amount: Decimal::new(125, 1),
        submitted_at_ms: now - 1000,
    }
}
fn legacy_row(now: i64) -> Value {
    json!({"method":"USDC - Solana","aclass":"currency","asset":"USDC","refid":REF_ID,
        "txid":"SolanaTxSignature","info":"SolanaDepositAddress","amount":"12.5","fee":"0","time":now/1000,"status":"Success"})
}
fn funding_row(now: i64) -> Value {
    json!({"deposit_id":REF_ID,"method_id":METHOD_ID,"network_id":NETWORK_ID,"status":"success",
        "amount":{"asset":{"class":"currency","name":"USDC"},"amount":"12.5"},
        "fee":{"asset":{"class":"currency","name":"USDC"},"amount":"0"},"create_time":rfc3339(now).unwrap()})
}
async fn mock_methods(server: &MockServer, row: Value) {
    Mock::given(method("GET"))
        .and(path("/funding/v1/methods/deposit"))
        .and(query_param("asset[class]", "currency"))
        .and(query_param("asset[name]", "USDC"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"methods":[row]})))
        .mount(server)
        .await;
}
async fn mock_history(server: &MockServer, legacy: Vec<Value>, funding: Vec<Value>) {
    Mock::given(method("POST"))
        .and(path(LEGACY_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"error":[],"result":{"deposit":legacy,"next_cursor":""}})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(DEPOSITS_PATH))
        .and(query_param("scope[method_id]", METHOD_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deposits":funding})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn kraken_deposit_adapter_metadata_address_and_exact_credit_round_trip() {
    let server = MockServer::start().await;
    mock_methods(&server, funding_method()).await;
    Mock::given(method("GET")).and(path("/0/public/Assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":{"USDC":{"aclass":"currency","decimals":6,"status":"enabled"}}})))
        .mount(&server).await;
    let mut withdrawal = funding_method();
    withdrawal["method_id"] = "withdraw-method".into();
    Mock::given(method("GET"))
        .and(path("/funding/v1/methods/withdraw"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"methods":[withdrawal]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADDRESS_PATH))
        .and(query_param("scope[method_id]", METHOD_ID))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"addresses":[address_row()]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let now = common::time::now_ms();
    mock_history(&server, vec![legacy_row(now)], vec![funding_row(now)]).await;
    let ws = server.uri().replacen("http:", "ws:", 1);
    let adapter = Kraken::new(KrakenConfig {
        credentials: Some(KrakenCredentials {
            spot: Some(credentials()),
            futures: None,
        }),
        spot_rest_url_override: Some(server.uri()),
        futures_rest_url_override: Some(server.uri()),
        spot_public_ws_url_override: Some(format!("{ws}/spot")),
        spot_private_ws_url_override: Some(format!("{ws}/private")),
        futures_ws_url_override: Some(format!("{ws}/futures")),
        ..Default::default()
    })
    .unwrap();
    let networks = adapter
        .fetch_transfer_networks_for(&["usdc".into(), "USDC".into()])
        .await
        .unwrap();
    assert_eq!(networks.len(), 2);
    let deposit = &networks[0];
    assert_eq!(deposit.network, METHOD_ID);
    assert_eq!(deposit.canonical_network, "solana");
    assert_eq!(
        deposit.contract_address.as_deref(),
        Some("CaseSensitiveMint")
    );
    assert!(deposit.deposit_enabled && !deposit.withdraw_enabled);
    assert!(networks[1].withdraw_enabled && !networks[1].deposit_enabled);
    let address = adapter
        .fetch_transfer_destination(&address_request())
        .await
        .unwrap();
    assert_eq!(address.status, TransferDestinationStatus::Verified);
    assert_eq!(address.address.as_deref(), Some("SolanaDepositAddress"));
    assert!(adapter.deposit_status_supported());
    let credit = adapter
        .deposit_status(&request(now))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credit.status, DepositStatus::Completed);
    assert_eq!(credit.amount, Decimal::new(125, 1));
    assert_eq!(credit.deposit_fee, Some(Decimal::ZERO));
    assert_eq!(credit.transaction_id, "SolanaTxSignature");
    assert!(
        adapter.spot_private_stream.get().is_none(),
        "funding reads must not warm trading streams"
    );
    let mut nonces = Vec::new();
    for request in server.received_requests().await.unwrap() {
        let path = request.url.path();
        if !path.starts_with("/funding/") && path != LEGACY_PATH {
            continue;
        }
        let (nonce, signed_path, body) = if request.method == "GET" {
            assert!(request.body.is_empty());
            (
                request.headers["API-Nonce"].to_str().unwrap().to_owned(),
                format!("{}?{}", path, request.url.query().unwrap()),
                String::new(),
            )
        } else {
            assert_eq!(path, LEGACY_PATH, "no funding write endpoint may be called");
            let body = String::from_utf8(request.body.clone()).unwrap();
            let nonce = url::form_urlencoded::parse(body.as_bytes())
                .find(|(key, _)| key == "nonce")
                .unwrap()
                .1
                .into_owned();
            (nonce, path.to_owned(), body)
        };
        let signature = crate::signing::kraken::spot_rest_sign(
            &credentials().api_secret,
            &signed_path,
            &nonce,
            &body,
        )
        .unwrap();
        assert_eq!(request.headers["API-Key"], "fixture-key");
        assert_eq!(request.headers["API-Sign"], signature);
        nonces.push(nonce.parse::<u64>().unwrap());
    }
    assert_eq!(nonces.len(), 7);
    assert!(nonces.windows(2).all(|n| n[1] > n[0]));
}

#[tokio::test]
async fn kraken_deposit_preflight_limits_fees_expiry_and_exact_address() {
    let server = MockServer::start().await;
    let http = client();
    for (amount, min, max, fee) in [
        (None, "1", "100", "0"),
        (Some("0.5"), "1", "100", "0"),
        (Some("101"), "1", "100", "0"),
        (Some("12.5"), "bad", "100", "0"),
        (Some("12.5"), "1", "bad", "0"),
        (Some("12.5"), "1", "100", "0.1"),
    ] {
        server.reset().await;
        let mut row = funding_method();
        row["minimum_amount"] = min.into();
        row["maximum_amount"] = max.into();
        row["fees"]["base"]["amount"] = fee.into();
        mock_methods(&server, row).await;
        let mut request = address_request();
        request.amount = amount.map(|v| v.parse().unwrap());
        let evidence = destination(&http, &server.uri(), &credentials(), &request)
            .await
            .unwrap();
        assert_eq!(evidence.status, TransferDestinationStatus::Unverified);
        assert!(evidence.problem.is_some());
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "invalid plans must stop before address acquisition"
        );
    }
    for (rows, expected) in [
        (vec![], TransferDestinationStatus::Missing),
        (
            vec![
                json!({"method_id":METHOD_ID,"expire_time":"2000-01-01T00:00:00Z","address_details":{"crypto":{"address":"old"}}}),
            ],
            TransferDestinationStatus::Missing,
        ),
        (vec![address_row()], TransferDestinationStatus::Verified),
        (
            vec![
                json!({"method_id":METHOD_ID,"address_details":{"crypto":{"address":"SolanaDepositAddress","memo":"123"}}}),
            ],
            TransferDestinationStatus::Unverified,
        ),
    ] {
        server.reset().await;
        mock_methods(&server, funding_method()).await;
        Mock::given(method("GET"))
            .and(path(ADDRESS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"addresses":rows})))
            .mount(&server)
            .await;
        assert_eq!(
            destination(&http, &server.uri(), &credentials(), &address_request())
                .await
                .unwrap()
                .status,
            expected
        );
        let mut request = address_request();
        request.expected_address = Some("solanadepositaddress".into());
        assert_ne!(
            destination(&http, &server.uri(), &credentials(), &request)
                .await
                .unwrap()
                .status,
            TransferDestinationStatus::Verified
        );
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"));
    }
}

#[tokio::test]
async fn kraken_deposit_does_not_credit_holds_sweeps_unmatched_ids_or_unknown_fees() {
    let server = MockServer::start().await;
    let http = client();
    let now = common::time::now_ms();
    for case in [
        "onhold",
        "return",
        "sweep",
        "no_id",
        "fee_missing",
        "charged",
        "pending",
        "failed",
        "hash_case",
        "amount",
        "method",
        "asset",
        "duplicate",
    ] {
        server.reset().await;
        mock_methods(&server, funding_method()).await;
        let mut legacy = legacy_row(now);
        let mut funding = funding_row(now);
        match case {
            "onhold" | "return" => legacy["status-prop"] = case.into(),
            "sweep" => legacy["originators"] = json!(["SolanaTxSignature", "OtherTransaction"]),
            "no_id" => funding["deposit_id"] = "another-deposit".into(),
            "fee_missing" => {
                funding.as_object_mut().unwrap().remove("fee");
            }
            "charged" => {
                funding["fee"]["amount"] = "0.1".into();
                legacy["fee"] = "0.1".into();
            }
            "pending" => funding["status"] = "settled".into(),
            "failed" => {
                funding["status"] = "failure".into();
                legacy["status"] = "Failure".into();
            }
            "hash_case" => legacy["txid"] = "solanatxsignature".into(),
            "amount" => funding["amount"]["amount"] = "12.4".into(),
            "method" => funding["method_id"] = "other-method".into(),
            "asset" => funding["amount"]["asset"]["name"] = "USDT".into(),
            _ => (),
        }
        let legacy = if case == "duplicate" {
            vec![legacy.clone(), legacy]
        } else {
            vec![legacy]
        };
        mock_history(&server, legacy, vec![funding]).await;
        let result = status(&http, &server.uri(), &credentials(), &request(now)).await;
        match case {
            "amount" | "method" | "asset" | "duplicate" => assert!(result.is_err(), "{case}"),
            "hash_case" => assert!(result.unwrap().is_none()),
            _ => {
                let evidence = result.unwrap().unwrap();
                assert_eq!(
                    evidence.status,
                    match case {
                        "no_id" | "pending" => DepositStatus::Pending,
                        "failed" => DepositStatus::Failed,
                        _ => DepositStatus::Blocked,
                    },
                    "{case}"
                );
            }
        }
    }
}

#[tokio::test]
async fn kraken_deposit_funding_cursor_is_exclusive_and_repeated_pages_fail_closed() {
    let server = MockServer::start().await;
    let http = client();
    Mock::given(method("GET"))
        .and(path(ADDRESS_PATH))
        .and(query_param("scope[method_id]", METHOD_ID))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"addresses":[],"next_cursor":"second"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADDRESS_PATH))
        .and(query_param("cursor", "second"))
        .respond_with(|request: &wiremock::Request| {
            assert_eq!(request.url.query_pairs().count(), 1);
            ResponseTemplate::new(200).set_body_json(json!({"addresses":[address_row()]}))
        })
        .mount(&server)
        .await;
    let found: Vec<ClaimedAddress> = rows(
        &http,
        &server.uri(),
        ADDRESS_PATH,
        vec![("scope[method_id]".into(), METHOD_ID.into())],
        "addresses",
        &credentials(),
    )
    .await
    .unwrap();
    assert_eq!(found.len(), 1);
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(ADDRESS_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"addresses":[address_row()],"next_cursor":"loop"})),
        )
        .mount(&server)
        .await;
    assert!(rows::<ClaimedAddress>(
        &http,
        &server.uri(),
        ADDRESS_PATH,
        vec![("limit".into(), "100".into())],
        "addresses",
        &credentials()
    )
    .await
    .is_err());
    Mock::given(method("POST"))
        .and(path(LEGACY_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"error":[],"result":{"deposit":[],"next_cursor":"loop"}})),
        )
        .mount(&server)
        .await;
    assert!(
        legacy_history(&http, &server.uri(), &credentials(), "USDC", 1, 1000)
            .await
            .is_err()
    );
}
