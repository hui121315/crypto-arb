use super::*;
use crate::adapter::ExchangeAdapter;
use crate::adapters::{Kraken, KrakenConfig, KrakenCredentials};
use crate::LiveTradingAdapter;
use wiremock::matchers::{method as http_method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const METHOD_ID: &str = "3e7f8072-cc6d-4394-982a-5f4ca6ab27dd";
const ADDRESS_ID: &str = "ABR6SXP-SF6CY-VJMONY";
const WITHDRAWAL_ID: &str = "FTVZiTI-e02T84mm87JmibnObWNdnW";
fn credentials() -> KrakenSpotCredentials {
    KrakenSpotCredentials {
        api_key: "fixture-key".into(),
        api_secret: "c2VjcmV0".into(),
    }
}
fn amount(value: &str) -> Value {
    json!({"asset":{"class":"currency","name":"USDC"},"amount":value})
}
fn method_row() -> Value {
    json!({"asset":{"class":"currency","name":"USDC"},"method_id":METHOD_ID,"minimum_amount":"1","maximum_amount":"100",
    "network":{"network_id":"b336ce74-8d60-42b8-8714-b1095e06b711","network_name":"Solana","contract_address":"CaseSensitiveMint"},
    "fees":{"base":amount("0.1"),"included":false}})
}
fn address_row() -> Value {
    json!({"address_id":ADDRESS_ID,"scope":{"method_id":METHOD_ID},"verified":true,"address_details":{"crypto":{"address":"SolanaWallet"}}})
}
fn limits_row() -> Value {
    json!({"available_balance":amount("50"),"withdrawal_limits":[{"method_id":METHOD_ID,"maximum_amount":amount("40"),
    "limits":[{"time_window":"86400","limit":{"limit_type":"attempt","remaining":"10","maximum":"20"}}]}]})
}
fn quote_row() -> Value {
    json!({"fee":amount("0.1"),"net_amount":amount("12.5"),"gross_amount":amount("12.6"),"withdrawal_fee_token":"fixture-fee-token"})
}
fn ack_row() -> Value {
    json!({"withdrawal_id":WITHDRAWAL_ID,"fee":{"asset_amount":amount("0.1")},"net_amount":{"asset_amount":amount("12.5")},"gross_amount":{"asset_amount":amount("12.6")}})
}
fn history_row(now: i64) -> Value {
    json!({"withdrawal_id":WITHDRAWAL_ID,"fee":amount("0.1"),"amount":amount("12.5"),"method_id":METHOD_ID,"address_id":ADDRESS_ID,
    "status":"success","onchain_transaction":"SolanaTxSignature","create_time":rfc3339(now).unwrap()})
}
fn request() -> WithdrawalSubmitRequest {
    WithdrawalSubmitRequest {
        venue: "kraken".into(),
        currency: "USDC".into(),
        network: METHOD_ID.into(),
        address: "SolanaWallet".into(),
        tag: None,
        amount: Decimal::new(125, 1),
        client_withdrawal_id: "durable-local-id".into(),
        wallet_type: WithdrawalWalletType::Spot,
        max_fee: Some(Decimal::new(1, 1)),
    }
}
fn history_request(now: i64) -> WithdrawalStatusRequest {
    let r = request();
    WithdrawalStatusRequest {
        venue: r.venue,
        currency: r.currency,
        network: r.network,
        address: r.address,
        tag: r.tag,
        client_withdrawal_id: r.client_withdrawal_id,
        provider_withdrawal_id: Some(WITHDRAWAL_ID.into()),
        submitted_at_ms: now - 1000,
    }
}
fn destination_request() -> TransferDestinationRequest {
    let r = request();
    TransferDestinationRequest {
        currency: r.currency,
        network: r.network,
        direction: TransferDirection::WithdrawToChain,
        expected_address: Some(r.address),
        expected_tag: r.tag,
        amount: Some(r.amount),
    }
}
fn client() -> HttpClient {
    HttpClient::builder("kraken-withdrawal-fixture")
        .max_retries(3)
        .build()
        .unwrap()
}
async fn mount_get(server: &MockServer, url: &str, body: Value) {
    Mock::given(http_method("GET"))
        .and(path(url))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}
async fn setup(server: &MockServer, method: Value, address: Value, limits: Value, quote: Value) {
    mount_get(
        server,
        "/funding/v1/methods/withdraw",
        json!({"methods":[method]}),
    )
    .await;
    mount_get(server, "/funding/v1/methods/deposit", json!({"methods":[]})).await;
    mount_get(
        server,
        "/0/public/Assets",
        json!({"error":[],"result":{"USDC":{"aclass":"currency","decimals":6,"status":"enabled"}}}),
    )
    .await;
    Mock::given(http_method("GET"))
        .and(path(ADDRESSES))
        .and(query_param("scope[method_id]", METHOD_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"addresses":[address]})))
        .mount(server)
        .await;
    mount_get(
        server,
        "/funding/v1/limits/withdrawal/currency/USDC",
        limits,
    )
    .await;
    Mock::given(http_method("GET"))
        .and(path(format!("/funding/v1/fees/{METHOD_ID}")))
        .and(query_param("amount", "12.5"))
        .and(query_param("fee_included", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(quote))
        .mount(server)
        .await;
}
fn adapter(server: &MockServer, write: bool) -> Kraken {
    let ws = server.uri().replacen("http:", "ws:", 1);
    Kraken::new(KrakenConfig {
        credentials: Some(KrakenCredentials {
            spot: Some(credentials()),
            futures: None,
        }),
        allow_live_writes: write,
        spot_rest_url_override: Some(server.uri()),
        futures_rest_url_override: Some(server.uri()),
        spot_public_ws_url_override: Some(format!("{ws}/spot")),
        spot_private_ws_url_override: Some(format!("{ws}/private")),
        futures_ws_url_override: Some(format!("{ws}/futures")),
        ..Default::default()
    })
    .unwrap()
}

#[tokio::test]
async fn kraken_withdrawal_adapter_net_amount_fee_token_and_exact_finality_round_trip() {
    let server = MockServer::start().await;
    setup(
        &server,
        method_row(),
        address_row(),
        limits_row(),
        quote_row(),
    )
    .await;
    Mock::given(http_method("POST"))
        .and(path(WITHDRAWALS))
        .respond_with(ResponseTemplate::new(200).set_body_json(ack_row()))
        .expect(1)
        .mount(&server)
        .await;
    let now = common::time::now_ms();
    mount_get(
        &server,
        WITHDRAWALS,
        json!({"withdrawals":[history_row(now)]}),
    )
    .await;
    let adapter = adapter(&server, true);
    assert!(adapter.withdrawal_submission_supported());
    let networks = adapter
        .fetch_transfer_networks_for(&["USDC".into()])
        .await
        .unwrap();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].withdrawal_step, Some(Decimal::new(1, 6)));
    assert_eq!(
        adapter
            .fetch_transfer_destination(&destination_request())
            .await
            .unwrap()
            .status,
        TransferDestinationStatus::Verified
    );
    let balance = adapter
        .withdrawal_source_balance(&WithdrawalSourceBalanceRequest {
            venue: "kraken".into(),
            currency: "USDC".into(),
            wallet_type: WithdrawalWalletType::Spot,
        })
        .await
        .unwrap();
    assert_eq!(balance.available, Decimal::from(50));
    let ack = adapter.submit_withdrawal(&request()).await.unwrap();
    assert!(ack.problem.is_none());
    assert_eq!(ack.provider_withdrawal_id, WITHDRAWAL_ID);
    let evidence = adapter
        .withdrawal_status(&history_request(now))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(evidence.status, WithdrawalStatus::Completed);
    assert_eq!(evidence.amount, request().amount);
    assert_eq!(evidence.transaction_fee, Decimal::new(1, 1));
    assert_eq!(
        evidence.transaction_id.as_deref(),
        Some("SolanaTxSignature")
    );
    assert!(adapter.spot_private_stream.get().is_none());
    let requests = server.received_requests().await.unwrap();
    for r in requests
        .iter()
        .filter(|r| r.url.path().starts_with("/funding/"))
    {
        let signed_path = match r.url.query() {
            Some(q) => format!("{}?{q}", r.url.path()),
            None => r.url.path().into(),
        };
        let nonce = r.headers["API-Nonce"].to_str().unwrap();
        let body = std::str::from_utf8(&r.body).unwrap();
        assert_eq!(
            r.headers["API-Sign"],
            crate::signing::kraken::spot_rest_sign(
                &credentials().api_secret,
                &signed_path,
                nonce,
                body
            )
            .unwrap()
        );
        if r.method == "POST" {
            let body: Value = serde_json::from_str(body).unwrap();
            assert_eq!(body["expected_address"], "SolanaWallet");
            assert_eq!(body["address_id"], ADDRESS_ID);
            assert_eq!(body["amount"]["asset_amount"]["amount"], "12.5");
            assert_eq!(body["fee"]["fee_included"], false);
            assert_eq!(body["fee"]["quoted_fee"]["token"], "fixture-fee-token");
            assert!(!body.to_string().contains("durable-local-id"));
        }
    }
}

#[tokio::test]
async fn kraken_withdrawal_rejects_changed_fees_limits_addresses_and_precision_before_write() {
    let server = MockServer::start().await;
    let http = client();
    for case in [
        "fee",
        "net",
        "asset",
        "token",
        "balance",
        "limit",
        "attempts",
        "unknown_limit",
        "unverified",
        "address_case",
        "tag",
        "precision",
        "no_budget",
        "zero_budget",
        "minimum",
        "maximum",
    ] {
        server.reset().await;
        let mut m = method_row();
        let mut a = address_row();
        let mut l = limits_row();
        let mut q = quote_row();
        let mut r = request();
        match case {
            "fee" => {
                q["fee"] = amount("0.2");
                q["gross_amount"] = amount("12.7");
            }
            "net" => q["net_amount"] = amount("12.4"),
            "asset" => q["fee"]["asset"]["name"] = "USDT".into(),
            "token" => q["withdrawal_fee_token"] = "".into(),
            "balance" => l["available_balance"] = amount("12.5"),
            "limit" => l["withdrawal_limits"][0]["maximum_amount"] = amount("12.5"),
            "attempts" => l["withdrawal_limits"][0]["limits"][0]["limit"]["remaining"] = "0".into(),
            "unknown_limit" => {
                l["withdrawal_limits"][0]["limits"][0]["limit"]["limit_type"] = "unknown".into()
            }
            "unverified" => a["verified"] = false.into(),
            "address_case" => a["address_details"]["crypto"]["address"] = "solanawallet".into(),
            "tag" => a["address_details"]["crypto"]["tag"] = "123".into(),
            "precision" => r.amount = Decimal::new(125000001, 7),
            "no_budget" => r.max_fee = None,
            "zero_budget" => r.max_fee = Some(Decimal::ZERO),
            "minimum" => m["minimum_amount"] = "20".into(),
            "maximum" => m["maximum_amount"] = "12.5".into(),
            _ => (),
        }
        setup(&server, m, a, l, q).await;
        let error = submit(&http, &server.uri(), &credentials(), &r)
            .await
            .unwrap_err();
        assert_eq!(
            error.exchange_code().as_deref(),
            Some("LOCAL_WITHDRAWAL_PREFLIGHT_REJECTED"),
            "{case}"
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET"),
            "{case} must never send a funding write"
        );
    }
}

#[tokio::test]
async fn kraken_withdrawal_one_write_on_timeout_and_known_receipt_survives_bad_ack() {
    let server = MockServer::start().await;
    let http = client();
    for failed in [true, false] {
        server.reset().await;
        setup(
            &server,
            method_row(),
            address_row(),
            limits_row(),
            quote_row(),
        )
        .await;
        let mut bad = ack_row();
        bad["gross_amount"]["asset_amount"] = amount("90");
        Mock::given(http_method("POST"))
            .and(path(WITHDRAWALS))
            .respond_with(if failed {
                ResponseTemplate::new(504).set_body_string("mock uncertain response")
            } else {
                ResponseTemplate::new(200).set_body_json(bad)
            })
            .expect(1)
            .mount(&server)
            .await;
        let result = submit(&http, &server.uri(), &credentials(), &request()).await;
        if failed {
            assert!(result.is_err());
        } else {
            let ack = result.unwrap();
            assert_eq!(ack.provider_withdrawal_id, WITHDRAWAL_ID);
            assert!(ack.problem.unwrap().contains("保留编号"));
        }
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn kraken_withdrawal_read_only_history_requires_original_id_and_exact_chain_receipt() {
    let server = MockServer::start().await;
    let http = client();
    let now = common::time::now_ms();
    let mut missing = history_request(now);
    missing.provider_withdrawal_id = None;
    assert!(status(&http, &server.uri(), &credentials(), &missing)
        .await
        .is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    for case in [
        "pending",
        "failed",
        "id",
        "duplicate",
        "method",
        "asset",
        "fee_asset",
        "address",
        "tag",
        "time",
        "no_hash",
    ] {
        server.reset().await;
        let mut row = history_row(now);
        let mut addr = address_row();
        match case {
            "pending" | "failed" => row["status"] = case.into(),
            "id" => row["withdrawal_id"] = "unrelated".into(),
            "method" => row["method_id"] = "unrelated".into(),
            "asset" => row["amount"]["asset"]["name"] = "USDT".into(),
            "fee_asset" => row["fee"]["asset"]["name"] = "ETH".into(),
            "address" => addr["address_details"]["crypto"]["address"] = "OtherWallet".into(),
            "tag" => addr["address_details"]["crypto"]["tag"] = "wrong".into(),
            "time" => row["create_time"] = "2000-01-01T00:00:00Z".into(),
            "no_hash" => row["onchain_transaction"] = Value::Null,
            _ => (),
        }
        let rows = if case == "duplicate" {
            vec![row.clone(), row]
        } else {
            vec![row]
        };
        mount_get(&server, WITHDRAWALS, json!({"withdrawals":rows})).await;
        mount_get(&server, ADDRESSES, json!({"addresses":[addr]})).await;
        let result = status(&http, &server.uri(), &credentials(), &history_request(now)).await;
        match case {
            "pending" => assert_eq!(result.unwrap().unwrap().status, WithdrawalStatus::Pending),
            "failed" => assert_eq!(result.unwrap().unwrap().status, WithdrawalStatus::Failed),
            "id" => assert!(result.unwrap().is_none()),
            _ => assert!(result.is_err(), "{case}"),
        };
    }
}

#[tokio::test]
async fn kraken_withdrawal_respects_write_switch_and_never_substitutes_another_wallet() {
    let server = MockServer::start().await;
    let adapter = adapter(&server, false);
    assert!(adapter.submit_withdrawal(&request()).await.is_err());
    assert!(adapter
        .withdrawal_source_balance(&WithdrawalSourceBalanceRequest {
            venue: "kraken".into(),
            currency: "USDC".into(),
            wallet_type: WithdrawalWalletType::Funding
        })
        .await
        .is_err());
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| !r.url.path().starts_with("/funding")));
}
