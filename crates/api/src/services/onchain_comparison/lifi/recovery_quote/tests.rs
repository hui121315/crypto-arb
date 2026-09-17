use super::*;
use serde_json::{json, Value};

fn request() -> RecoveryQuoteRequest {
    RecoveryQuoteRequest {
        from_chain: "base".into(),
        to_chain: "base".into(),
        from_token: format!("0x{:040x}", 1),
        to_token: format!("0x{:040x}", 2),
        from_wallet: format!("0x{:040x}", 3),
        to_wallet: format!("0x{:040x}", 3),
        amount_raw: "1000000".into(),
        slippage_bps: 50.0,
    }
}

fn body(request: &RecoveryQuoteRequest) -> Value {
    let from = chain_id(&request.from_chain).unwrap();
    let to = chain_id(&request.to_chain).unwrap();
    json!({"id":"quote-id-not-chain-hash","tool":"fixture",
        "action":{"fromChainId":from,"toChainId":to,
            "fromToken":{"chainId":from,"address":request.from_token},"toToken":{"chainId":to,"address":request.to_token},
            "fromAddress":request.from_wallet,"toAddress":request.to_wallet,"fromAmount":request.amount_raw},
        "estimate":{"fromAmount":request.amount_raw,"toAmount":"999000","toAmountMin":"990000",
            "feeCosts":[],"gasCosts":[{"amountUSD":"0.01"}],"executionDuration":5,"approvalAddress":format!("0x{:040x}", 4)},
        "transactionRequest":{"from":request.from_wallet,"to":format!("0x{:040x}", 4),"chainId":from,
            "data":"0x1234","value":"0x0","gasLimit":"0x5208","gasPrice":"0x1"}})
}

#[tokio::test]
async fn recovery_quote_http_uses_official_same_chain_shape_without_transaction_id() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let request = request();
    let body = body(&request).to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/quote", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![];
        loop {
            let mut chunk = [0; 2048];
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&chunk[..read]);
            if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let wire = String::from_utf8(bytes).unwrap();
        let path = wire
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        assert!(wire.starts_with("GET "));
        assert!(!wire.to_lowercase().contains("api-key"));
        let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["fromChain"], "8453");
        assert_eq!(query["toChain"], "8453");
        assert_eq!(query["fromAmount"], "1000000");
        assert_eq!(query["slippage"], "0.005");
        assert_eq!(query["order"], "CHEAPEST");
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let quote = fetch_at(&client, &endpoint, None, &request).await.unwrap();
    server.await.unwrap();
    assert_eq!(quote.route_id, "quote-id-not-chain-hash");
    assert_eq!(quote.output_raw, "999000");
    assert_eq!(quote.minimum_raw, "990000");
    assert_eq!(quote.fee_usd, Some(0.0));
    assert_eq!(quote.gas_usd, Some(0.01));
}

#[test]
fn recovery_quote_costs_are_unknown_when_absent_partial_or_gas_empty() {
    let request = request();
    for missing in [Value::Null, json!([{"amountUSD":"0.1"},{}])] {
        let mut body = body(&request);
        body["estimate"]["feeCosts"] = missing;
        body["estimate"]["gasCosts"] = json!([]);
        let quote = parse(serde_json::from_value(body).unwrap(), &request, 100).unwrap();
        assert_eq!(quote.fee_usd, None);
        assert_eq!(quote.gas_usd, None);
    }
    let mut body = body(&request);
    body["estimate"]["gasCosts"] = json!([{"amountUSD":"1.7e308"},{"amountUSD":"1.7e308"}]);
    assert!(parse(serde_json::from_value(body).unwrap(), &request, 100).is_err());
}

#[test]
fn recovery_quote_rejects_changed_identity_amount_and_minimum() {
    let request = request();
    for mutate in [
        ("/action/fromAmount", json!("2")),
        ("/action/toChainId", json!(1)),
        ("/action/toToken/address", json!(format!("0x{:040x}", 99))),
        ("/action/toAddress", json!(format!("0x{:040x}", 99))),
        ("/estimate/toAmountMin", json!("1000001")),
        ("/transactionRequest/chainId", json!(1)),
    ] {
        let mut body = body(&request);
        *body.pointer_mut(mutate.0).unwrap() = mutate.1;
        assert!(parse(serde_json::from_value(body).unwrap(), &request, 100).is_err());
    }
    let mut cross = request;
    cross.to_chain = "ethereum".into();
    assert!(parse(serde_json::from_value(body(&cross)).unwrap(), &cross, 100).is_ok());
}
