use super::*;
use crate::adapter::ExchangeAdapter;
use crate::adapters::bybit::{Bybit, BybitConfig, BybitCredentials};
use crate::LiveTradingAdapter;
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn signed(_: &str) -> ExchangeResult<SignedHeaders> {
    Ok([
        ("X-BAPI-API-KEY".into(), "fixture-key".into()),
        ("X-BAPI-TIMESTAMP".into(), "1".into()),
        ("X-BAPI-RECV-WINDOW".into(), "5000".into()),
        ("X-BAPI-SIGN".into(), "fixture-signature".into()),
    ])
}
fn envelope(result: Value) -> Value {
    json!({"retCode":0,"retMsg":"OK","result":result})
}
fn page(rows: Vec<Value>) -> Value {
    envelope(json!({"rows":rows,"nextPageCursor":""}))
}
fn withdrawal() -> WithdrawalSubmitRequest {
    WithdrawalSubmitRequest {
        venue: "bybit".into(),
        currency: "USDC".into(),
        network: "SOL".into(),
        address: "SolanaAddress".into(),
        tag: None,
        amount: Decimal::new(125, 1),
        client_withdrawal_id: "crossline-existing-durable-transfer-id".into(),
        wallet_type: WithdrawalWalletType::Spot,
        max_fee: None,
    }
}
fn history_request(now: i64) -> WithdrawalStatusRequest {
    let r = withdrawal();
    WithdrawalStatusRequest {
        venue: r.venue,
        currency: r.currency,
        network: r.network,
        address: r.address,
        tag: r.tag,
        client_withdrawal_id: r.client_withdrawal_id,
        provider_withdrawal_id: Some("withdrawal-42".into()),
        submitted_at_ms: now,
    }
}
fn address_request() -> TransferDestinationRequest {
    TransferDestinationRequest {
        currency: "USDC".into(),
        network: "SOL".into(),
        direction: TransferDirection::WithdrawToChain,
        expected_address: Some("SolanaAddress".into()),
        expected_tag: None,
        amount: None,
    }
}
fn address_row() -> Value {
    json!({"coin":"baseCoin","chain":"SOL","address":"SolanaAddress","tag":"", "status":0,"addressType":0,"verified":1})
}
fn history_row(now: i64) -> Value {
    json!({"withdrawId":"withdrawal-42","txID":"tx-bybit","coin":"USDC","chain":"SOL", "amount":"12.5",
        "withdrawFee":"0.1","status":"success","toAddress":"SolanaAddress","tag":"", "withdrawType":0,"createTime":now.to_string(),"tax":"0"})
}
fn balance_row(amount: &str, available: &str) -> Value {
    json!({"coin":"USDC","withdrawableAmount":amount,"availableBalance":available})
}
fn client() -> HttpClient {
    HttpClient::builder("bybit").max_retries(3).build().unwrap()
}

#[tokio::test]
async fn bybit_withdrawal_adapter_round_trip_uses_uta_exact_receipt_and_signed_requests() {
    let server = MockServer::start().await;
    let now = common::time::now_ms();
    Mock::given(method("GET"))
        .and(path("/v5/market/time"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"timeSecond":(now/1000).to_string(),"timeNano":(now*1_000_000).to_string()}),
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADDRESS_PATH))
        .and(query_param("chain", "SOL"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![address_row()])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(BALANCE_PATH))
        .and(query_param("coin", "USDC"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"withdrawableAmount":{
                "UTA":balance_row("13","20"), "FUND":balance_row("900","900")
            }}),
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(WITHDRAW_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(envelope(json!({"id":"withdrawal-42"}))),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(HISTORY_PATH))
        .and(query_param("withdrawID", "withdrawal-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![history_row(now)])))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = Bybit::new(BybitConfig {
        credentials: Some(BybitCredentials {
            api_key: "fixture-key".into(),
            api_secret: "fixture-secret".into(),
        }),
        base_url_override: Some(server.uri()),
        allow_live_writes: true,
        ..BybitConfig::default()
    })
    .unwrap();
    assert!(adapter.withdrawal_submission_supported());
    assert_eq!(
        adapter
            .fetch_transfer_destination(&address_request())
            .await
            .unwrap()
            .status,
        TransferDestinationStatus::Verified
    );
    let balance = adapter
        .withdrawal_source_balance(&WithdrawalSourceBalanceRequest {
            venue: "bybit".into(),
            currency: "USDC".into(),
            wallet_type: WithdrawalWalletType::Spot,
        })
        .await
        .unwrap();
    assert_eq!(balance.available, Decimal::from(13));
    let request = withdrawal();
    let ack = adapter.submit_withdrawal(&request).await.unwrap();
    assert_eq!(ack.client_withdrawal_id, request.client_withdrawal_id);
    assert_eq!(ack.provider_withdrawal_id, "withdrawal-42");
    let evidence = adapter
        .withdrawal_status(&history_request(now))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(evidence.status, WithdrawalStatus::Completed);
    assert_eq!(evidence.amount, request.amount);
    assert_eq!(evidence.transaction_fee, Decimal::new(1, 1));
    assert_eq!(evidence.transaction_id.as_deref(), Some("tx-bybit"));
    for sent in server.received_requests().await.unwrap() {
        if sent.url.path() == "/v5/market/time" {
            continue;
        }
        let payload = if sent.method.as_str() == "POST" {
            std::str::from_utf8(&sent.body).unwrap()
        } else {
            sent.url.query().unwrap()
        };
        let timestamp = sent
            .headers
            .get("X-BAPI-TIMESTAMP")
            .unwrap()
            .to_str()
            .unwrap();
        let expected = crate::signing::bybit::sign(
            b"fixture-secret",
            timestamp,
            "fixture-key",
            "5000",
            payload,
        );
        assert_eq!(
            sent.headers.get("X-BAPI-SIGN").unwrap().to_str().unwrap(),
            expected
        );
        if sent.method.as_str() == "POST" {
            let body: Value = serde_json::from_slice(&sent.body).unwrap();
            assert_eq!(body["accountType"], "UTA");
            assert_eq!(body["feeType"], 0);
            assert_eq!(body["forceChain"], 1);
            assert_eq!(body["amount"], "12.5");
            assert_eq!(
                body["requestId"],
                wire_request_id(&request.client_withdrawal_id)
            );
            assert_eq!(body["requestId"].as_str().unwrap().len(), 32);
            assert!(body["requestId"]
                .as_str()
                .unwrap()
                .chars()
                .all(|c| c.is_ascii_alphanumeric()));
            assert!(
                (body["timestamp"].as_i64().unwrap() - timestamp.parse::<i64>().unwrap()).abs()
                    < 1000
            );
        }
    }
}

#[tokio::test]
async fn bybit_withdrawal_does_not_retry_failed_submission_or_guess_lost_ack() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(WITHDRAW_PATH))
        .respond_with(ResponseTemplate::new(503).set_body_string("unknown result"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(submit(
        &client(),
        &server.uri(),
        &withdrawal(),
        common::time::now_ms(),
        signed
    )
    .await
    .is_err());
    let mut request = history_request(common::time::now_ms());
    request.provider_withdrawal_id = None;
    let error = status(&client(), &server.uri(), &request, signed)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("不重发"));
    assert!(error.contains(&wire_request_id(&request.client_withdrawal_id)));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn bybit_withdrawal_address_pagination_requires_exact_verified_unlocked_destination() {
    let server = MockServer::start().await;
    Mock::given(path(ADDRESS_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(json!({"rows":[],"nextPageCursor":"page2"}))),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(path(ADDRESS_PATH))
        .and(query_param("cursor", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![address_row()])))
        .with_priority(1)
        .mount(&server)
        .await;
    assert_eq!(
        destination(&client(), &server.uri(), &address_request(), signed)
            .await
            .unwrap()
            .status,
        TransferDestinationStatus::Verified
    );
    for (field, value) in [
        ("status", json!(1)),
        ("verified", json!(0)),
        ("address", json!("solanaaddress")),
        ("tag", json!("42")),
        ("coin", json!("USDT")),
        ("addressType", json!(1)),
    ] {
        server.reset().await;
        let mut row = address_row();
        row[field] = value;
        Mock::given(path(ADDRESS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![row])))
            .mount(&server)
            .await;
        assert_ne!(
            destination(&client(), &server.uri(), &address_request(), signed)
                .await
                .unwrap()
                .status,
            TransferDestinationStatus::Verified,
            "{field}"
        );
    }
    server.reset().await;
    Mock::given(path(ADDRESS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"rows":[address_row()],"nextPageCursor":"loop"}),
        )))
        .expect(2)
        .mount(&server)
        .await;
    assert!(
        destination(&client(), &server.uri(), &address_request(), signed)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn bybit_withdrawal_balance_does_not_use_other_wallets_or_locked_funds() {
    let server = MockServer::start().await;
    let request = WithdrawalSourceBalanceRequest {
        venue: "bybit".into(),
        currency: "USDC".into(),
        wallet_type: WithdrawalWalletType::Spot,
    };
    for (uta, expected) in [
        (Some(balance_row("0", "800")), Some(Decimal::ZERO)),
        (None, None),
        (Some(balance_row("30", "20")), Some(Decimal::from(20))),
        (Some(balance_row("", "20")), None),
    ] {
        server.reset().await;
        let mut wallets = json!({"FUND":balance_row("900","900")});
        if let Some(uta) = uta {
            wallets["UTA"] = uta;
        }
        Mock::given(path(BALANCE_PATH))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(envelope(json!({"withdrawableAmount":wallets}))),
            )
            .mount(&server)
            .await;
        assert_eq!(
            source_balance(&client(), &server.uri(), &request, signed)
                .await
                .ok()
                .map(|row| row.available),
            expected
        );
    }
}

#[test]
fn bybit_withdrawal_history_does_not_accept_mismatches_unknown_costs_or_premature_success() {
    let now = common::time::now_ms();
    let request = history_request(now);
    for (field, value) in [
        ("coin", json!("USDT")),
        ("chain", json!("ETH")),
        ("toAddress", json!("solanaaddress")),
        ("tag", json!("99")),
        ("withdrawType", json!(1)),
        ("createTime", json!((now - 600_000).to_string())),
        ("withdrawFee", json!("")),
        ("tax", json!("0.01")),
        ("txID", json!("")),
        ("status", json!("MoreInformationRequired")),
        ("status", json!("Unknown")),
    ] {
        let mut row = history_row(now);
        row[field] = value;
        assert!(
            parse_status(
                serde_json::from_value(row).unwrap(),
                &request,
                now - HISTORY_CLOCK_SKEW_MS,
                now,
                "fixture".into()
            )
            .is_err(),
            "{field}"
        );
    }
    for (native, expected) in [
        ("BlockchainConfirmed", WithdrawalStatus::Pending),
        ("Pending", WithdrawalStatus::Pending),
        ("CancelByUser", WithdrawalStatus::Cancelled),
        ("Reject", WithdrawalStatus::Rejected),
        ("Fail", WithdrawalStatus::Failed),
    ] {
        let mut row = history_row(now);
        row["status"] = json!(native);
        row["txID"] = json!("");
        let evidence = parse_status(
            serde_json::from_value(row).unwrap(),
            &request,
            now - HISTORY_CLOCK_SKEW_MS,
            now,
            "fixture".into(),
        )
        .unwrap();
        assert_eq!(evidence.status, expected);
    }
}

#[tokio::test]
async fn bybit_withdrawal_history_requires_unique_id_and_complete_pages() {
    let server = MockServer::start().await;
    let now = common::time::now_ms();
    let request = history_request(now);
    for rows in [vec![history_row(now), history_row(now)], vec![]] {
        server.reset().await;
        let duplicate = !rows.is_empty();
        Mock::given(path(HISTORY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(rows)))
            .mount(&server)
            .await;
        let result = status(&client(), &server.uri(), &request, signed).await;
        if duplicate {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_none());
        }
    }
}
