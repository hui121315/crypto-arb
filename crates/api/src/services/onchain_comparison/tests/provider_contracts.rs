use super::*;

#[test]
fn official_order_quote_without_taker_decodes_as_read_only_evidence() -> anyhow::Result<()> {
    let quote: JupiterOrderQuote = serde_json::from_value(serde_json::json!({
        "inputMint": "base",
        "outputMint": "quote",
        "inAmount": "1000000000",
        "outAmount": "150000000",
        "router": "iris",
        "transaction": null
    }))?;
    let quote = quote::jupiter_provider_quote(quote);

    assert_eq!(quote_price(&quote, 9, 6), Some(150.0));
    let pair = quote_pair(
        quote,
        provider_quote("quote", "base", "100000000", "1000000000"),
    );
    let evidence = pair.evidence();
    assert!(evidence.iter().all(|row| !row.transaction_requested));
    assert_eq!(evidence[0].official_docs_url, JUPITER_ORDER_DOCS);
    Ok(())
}

#[test]
fn official_zeroex_price_decodes_indicative_read_only_amounts() -> anyhow::Result<()> {
    let quote: ZeroExPrice = serde_json::from_value(serde_json::json!({
        "buyAmount": "150000000",
        "buyToken": "0xquote",
        "sellAmount": "1000000000000000000",
        "sellToken": "0xbase",
        "liquidityAvailable": true,
        "route": { "fills": [{ "source": "Uniswap_V3" }] }
    }))?;
    let quote = quote::zeroex_provider_quote(quote).map_err(anyhow::Error::msg)?;

    assert_eq!(quote.input_address, "0xbase");
    assert_eq!(quote.output_address, "0xquote");
    assert_eq!(quote.router.as_deref(), Some("Uniswap_V3"));
    assert_eq!(
        ZEROEX_PRICE_DOCS,
        "https://docs.0x.org/api-reference/evm-ap-is/swap/allowanceholder-getprice"
    );
    Ok(())
}

#[test]
fn official_okx_v6_quote_decodes_read_only_amounts() -> anyhow::Result<()> {
    let quote = okx::decode_envelope(
        r#"{
            "code":"0",
            "msg":"",
            "data":[{
                "chainIndex":"8453",
                "fromTokenAmount":"1000000000000000000",
                "toTokenAmount":"3000000000",
                "router":"native--usdc",
                "fromToken":{"tokenContractAddress":"0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},
                "toToken":{"tokenContractAddress":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"}
            }]
        }"#,
    )
    .map_err(anyhow::Error::msg)?;
    let quote = okx::provider_quote(quote, 8_453).map_err(anyhow::Error::msg)?;

    assert_eq!(quote.input_amount_raw, "1000000000000000000");
    assert_eq!(quote.output_amount_raw, "3000000000");
    assert_eq!(quote.router.as_deref(), Some("native--usdc"));
    assert_eq!(
        okx::OKX_QUOTE_DOCS,
        "https://web3.okx.com/zh-hans/onchainos/dev-docs/trade/dex-get-quote"
    );
    Ok(())
}

#[test]
fn firm_provider_responses_require_buildable_transactions() -> anyhow::Result<()> {
    let jupiter: JupiterOrderBuild = serde_json::from_value(serde_json::json!({
        "inputMint": "quote",
        "outputMint": "base",
        "inAmount": "100000000",
        "outAmount": "1000000000",
        "transaction": "AQID",
        "requestId": "jup-request-1",
        "router": "metis",
        "mode": "ultra",
        "lastValidBlockHeight": 300000001
    }))?;
    assert_eq!(jupiter.transaction.as_deref(), Some("AQID"));
    assert_eq!(jupiter.request_id, "jup-request-1");

    let zeroex: ZeroExFirmQuote = serde_json::from_value(serde_json::json!({
        "buyAmount": "100000000",
        "minBuyAmount": "99000000",
        "buyToken": "0xquote",
        "sellAmount": "1000000000000000000",
        "sellToken": "0xbase",
        "liquidityAvailable": true,
        "issues": {
            "allowance": null,
            "balance": null,
            "simulationIncomplete": false
        },
        "transaction": {
            "to": "0x1111111111111111111111111111111111111111",
            "data": "0x1234",
            "value": "0",
            "gas": "150000",
            "gasPrice": "1000000"
        }
    }))?;
    assert_eq!(
        zeroex
            .transaction
            .as_ref()
            .map(|transaction| transaction.to.as_str()),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert!(zeroex.issues.is_some_and(|issues| {
        issues.allowance.is_none()
            && issues.balance.is_none()
            && issues.simulation_incomplete == Some(false)
    }));
    assert_eq!(zeroex.min_buy_amount.as_deref(), Some("99000000"));

    let okx = okx::decode_swap_envelope(
        r#"{
            "code":"0",
            "msg":"",
            "data":[{
                "routerResult":{
                    "chainIndex":"8453",
                    "fromTokenAmount":"1000000000000000000",
                    "toTokenAmount":"3000000000",
                    "router":"native--usdc",
                    "fromToken":{"tokenContractAddress":"0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},
                    "toToken":{"tokenContractAddress":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"}
                },
                "tx":{
                    "from":"0x2222222222222222222222222222222222222222",
                    "to":"0x3333333333333333333333333333333333333333",
                    "data":"0x1234",
                    "value":"0",
                    "gas":"150000",
                    "gasPrice":"1000000"
                }
            }]
        }"#,
    )
    .map_err(anyhow::Error::msg)?;
    assert_eq!(okx.tx.data, "0x1234");
    assert_eq!(okx.router_result.to_token_amount, "3000000000");
    Ok(())
}

#[test]
fn official_cow_fast_quote_keeps_fee_inside_observed_input() -> anyhow::Result<()> {
    let quote = cow::decode_quote(
        r#"{
            "quote": {
                "sellToken": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                "buyToken": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "sellAmount": "999624142083565405",
                "buyAmount": "1886195487",
                "feeAmount": "375857916434595",
                "kind": "sell"
            },
            "from": "0x0000000000000000000000000000000000000001",
            "expiration": "1970-01-01T00:00:00Z",
            "id": null,
            "verified": false
        }"#,
        "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
        "0xA0b86991c6218b36c1d19d4a2e9Eb0cE3606eB48",
        "1000000000000000000",
    )
    .map_err(anyhow::Error::msg)?;

    assert_eq!(quote.input_amount_raw, "1000000000000000000");
    assert_eq!(quote.output_amount_raw, "1886195487");
    assert_eq!(quote.router.as_deref(), Some("CoW fast · unverified"));
    assert_eq!(
        cow::COW_QUOTE_DOCS,
        "https://api.cow.fi/docs/#/default/post_api_v1_quote"
    );
    Ok(())
}

#[test]
fn cow_request_matches_official_fast_quote_contract() -> anyhow::Result<()> {
    let request = cow::build_quote_request(
        quote::quote_client(),
        "https://api.cow.fi/mainnet/api/v1/quote",
        "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
        "0xA0b86991c6218b36c1d19d4a2e9Eb0cE3606eB48",
        "1000000000000000000",
    )
    .build()?;

    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().path(), "/mainnet/api/v1/quote");
    let body = request
        .body()
        .and_then(reqwest::Body::as_bytes)
        .ok_or_else(|| anyhow::anyhow!("CoW request body is unavailable"))?;
    let body: serde_json::Value = serde_json::from_slice(body)?;
    assert_eq!(body["kind"], "sell");
    assert_eq!(body["priceQuality"], "fast");
    assert_eq!(body["sellAmountBeforeFee"], "1000000000000000000");
    assert_eq!(body["timeout"], 1_000);
    assert_eq!(body["onchainOrder"], false);
    Ok(())
}

#[test]
fn cow_uses_official_wrapped_native_identity_and_rejects_optimism() {
    assert_eq!(
        cow::quote_token_address("base", shared_types::EVM_NATIVE_TOKEN_ADDRESS),
        Some("0x4200000000000000000000000000000000000006")
    );
    assert_eq!(
        cow::quote_token_address("optimism", shared_types::EVM_NATIVE_TOKEN_ADDRESS),
        None
    );
}

#[test]
fn okx_v6_request_matches_official_method_query_and_auth_contract() -> anyhow::Result<()> {
    let credentials = OkxCredentials {
        api_key: "api-key".to_owned(),
        secret_key: "secret-key".to_owned(),
        passphrase: "passphrase".to_owned(),
    };
    let timestamp = "2026-07-28T12:00:00.000Z";
    let request = okx::build_quote_request(
        quote::quote_client(),
        &credentials,
        timestamp,
        okx::OkxQuoteRequest {
            chain_id: 8_453,
            from_token: "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            to_token: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            amount: "1000000000000000000",
        },
    )
    .map_err(anyhow::Error::msg)?;

    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(request.url().path(), "/api/v6/dex/aggregator/quote");
    let query = request.url().query().unwrap_or_default();
    assert!(query.contains("chainIndex=8453"));
    assert!(query.contains("swapMode=exactIn"));
    assert!(query.contains("fromTokenAddress=0xeeee"));
    assert_eq!(request.headers()["OK-ACCESS-KEY"], "api-key");
    assert_eq!(request.headers()["OK-ACCESS-PASSPHRASE"], "passphrase");
    assert_eq!(request.headers()["OK-ACCESS-TIMESTAMP"], timestamp);
    let prehash = format!("{timestamp}GET{}?{query}", request.url().path());
    let expected =
        common::signing::hmac_sha256_base64(credentials.secret_key.as_bytes(), prehash.as_bytes());
    assert_eq!(request.headers()["OK-ACCESS-SIGN"], expected);
    assert!(okx::OKX_AUTH_DOCS.contains("api-access-and-usage"));
    Ok(())
}

#[test]
fn okx_swap_request_binds_wallet_slippage_and_firm_route() -> anyhow::Result<()> {
    let credentials = OkxCredentials {
        api_key: "api-key".to_owned(),
        secret_key: "secret-key".to_owned(),
        passphrase: "passphrase".to_owned(),
    };
    let request = okx::build_swap_request(
        quote::quote_client(),
        &credentials,
        "2026-08-11T00:00:00.000Z",
        okx::OkxSwapRequest {
            chain_id: 8_453,
            from_token: "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            to_token: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            amount: "1000000000000000000",
            slippage_percent: "0.100000",
            wallet_address: "0x2222222222222222222222222222222222222222",
        },
    )
    .map_err(anyhow::Error::msg)?;

    assert_eq!(request.url().path(), "/api/v6/dex/aggregator/swap");
    let query = request.url().query().unwrap_or_default();
    assert!(query.contains("userWalletAddress=0x2222"));
    assert!(query.contains("slippagePercent=0.100000"));
    assert!(query.contains("swapMode=exactIn"));
    Ok(())
}

#[test]
fn okx_approval_request_binds_exact_token_amount_and_auth() -> anyhow::Result<()> {
    let credentials = OkxCredentials {
        api_key: "api-key".to_owned(),
        secret_key: "secret-key".to_owned(),
        passphrase: "passphrase".to_owned(),
    };
    let timestamp = "2026-08-12T00:00:00.000Z";
    let request = okx::build_approval_request(
        quote::quote_client(),
        &credentials,
        timestamp,
        okx::OkxApprovalRequest {
            chain_id: 8_453,
            token_contract_address: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            approve_amount: "1000000",
        },
    )
    .map_err(anyhow::Error::msg)?;

    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(
        request.url().path(),
        "/api/v6/dex/aggregator/approve-transaction"
    );
    let query = request.url().query().unwrap_or_default();
    assert!(query.contains("chainIndex=8453"));
    assert!(query.contains("tokenContractAddress=0x833589"));
    assert!(query.contains("approveAmount=1000000"));
    assert_eq!(request.headers()["OK-ACCESS-KEY"], "api-key");
    assert_eq!(request.headers()["OK-ACCESS-TIMESTAMP"], timestamp);
    let prehash = format!("{timestamp}GET{}?{query}", request.url().path());
    let expected =
        common::signing::hmac_sha256_base64(credentials.secret_key.as_bytes(), prehash.as_bytes());
    assert_eq!(request.headers()["OK-ACCESS-SIGN"], expected);
    assert!(okx::OKX_APPROVAL_DOCS.contains("approve-transaction"));
    Ok(())
}

#[test]
fn okx_approval_response_keeps_spender_calldata_and_gas() -> anyhow::Result<()> {
    let approval = okx::decode_approval_envelope(
        r#"{
            "code":"0",
            "msg":"",
            "data":[{
                "data":"0x095ea7b3000000000000000000000000111111111111111111111111111111111111111100000000000000000000000000000000000000000000000000000000000f4240",
                "dexContractAddress":"0x1111111111111111111111111111111111111111",
                "gasLimit":"65000",
                "gasPrice":"1000000"
            }]
        }"#,
    )
    .map_err(anyhow::Error::msg)?;

    assert_eq!(
        approval.dex_contract_address,
        "0x1111111111111111111111111111111111111111"
    );
    assert!(approval.data.starts_with("0x095ea7b3"));
    assert_eq!(approval.gas_limit, "65000");
    assert_eq!(approval.gas_price, "1000000");
    Ok(())
}

#[test]
fn exact_raw_amounts_reject_zero_and_invalid_values() {
    assert_eq!(raw_units("1000000", 6), Some(1.0));
    assert_eq!(raw_units("0", 6), None);
    assert_eq!(raw_units("1.5", 6), None);
}

#[test]
fn reverse_quote_output_is_the_only_base_amount_anchor() {
    let reverse = provider_quote("quote", "base", "100000000", "975000000");
    assert_eq!(
        quote::anchored_base_amount(&reverse),
        Ok("975000000".to_owned())
    );

    let invalid = provider_quote("quote", "base", "100000000", "0");
    assert!(quote::anchored_base_amount(&invalid).is_err());
}
