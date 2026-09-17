use super::*;

pub(crate) fn mint_response(address: &str) -> Value {
    serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"context":{"slot":200},"value":[
        {"owner":TOKEN_2022,"executable":false,"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":6,"extensions":[
            {"extension":"scaledUiAmountConfig","state":{"multiplier":"1.25","newMultiplier":"2","newMultiplierEffectiveTimestamp":1010}},
            {"extension":"tokenMetadata","state":{"mint":address}},
            {"extension":"pausableConfig","state":{"paused":false}}
        ]}}}},
        {"owner":TOKEN,"executable":false,"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":6}}}},
        {"owner":"Sysvar1111111111111111111111111111111111111","executable":false,"data":{"parsed":{"type":"clock","info":{"unixTimestamp":1000}}}}
    ]}})
}

#[test]
fn stock_mint_checks_precision_extensions_and_chain_clock_for_rebasing() {
    let address = "MUxEsUKSMACyw5fZf68wxf5FLnZVhtU9CwH8uNNGay1";
    let mut body = mint_response(address);
    let decode = |v: &Value, now| parse_mint(&serde_json::to_vec(v).unwrap(), address, 6, now);
    let before = decode(&body, 1_000_000).unwrap();
    assert_eq!(before.ui_multiplier, "1.25");
    assert_eq!(before.next_change_at_ms, Some(1_010_000));
    body["result"]["value"][2]["data"]["parsed"]["info"]["unixTimestamp"] = 1010.into();
    assert_eq!(decode(&body, 1_010_000).unwrap().ui_multiplier, "2");
    body["result"]["value"][0]["data"]["parsed"]["info"]["extensions"][2]["state"]["paused"] =
        true.into();
    assert!(decode(&body, 1_010_000)
        .unwrap_err()
        .contains("pausableConfig"));
    body = mint_response(address);
    body["result"]["value"][0]["data"]["parsed"]["info"]["decimals"] = 9.into();
    assert!(decode(&body, 1_000_000).is_err());
    assert!(decode(&mint_response(address), 1_100_000).is_err());
    body = mint_response(address);
    body["result"]["value"][0]["data"]["parsed"]["info"]["extensions"][2]["extension"] =
        "transferFeeConfig".into();
    assert!(decode(&body, 1_000_000)
        .unwrap_err()
        .contains("transferFeeConfig"));
}

#[test]
fn stock_jupiter_quotes_require_exact_mints_amount_minimum_expiry_and_no_transaction() {
    let mut body = serde_json::json!({"inputMint":"USDC","outputMint":"MU","inAmount":"10000000","outAmount":"11000",
        "otherAmountThreshold":"10500","swapMode":"ExactIn","router":"metis","feeBps":10,"feeMint":"USDC",
        "transaction":null,"taker":null,"expireAt":"1970-01-01T00:00:10Z"});
    let decode = |v: &Value| parse_quote(&v.to_string(), "USDC", "MU", "10000000", 1000, 1100);
    let q = decode(&body).unwrap();
    assert_eq!(q.minimum_output_raw, "10500");
    assert_eq!(q.fee_bps, Some(10));
    body["outputMint"] = "mu".into();
    assert!(decode(&body).is_err());
    body["outputMint"] = "MU".into();
    body["inAmount"] = "10000001".into();
    assert!(decode(&body).is_err());
    body["inAmount"] = "10000000".into();
    body["otherAmountThreshold"] = "11001".into();
    assert!(decode(&body).is_err());
    body["otherAmountThreshold"] = "10500".into();
    body["transaction"] = "unsigned-but-unrequested".into();
    assert!(decode(&body).is_err());
    body["transaction"] = Value::Null;
    body["expireAt"] = "1970-01-01T00:00:01Z".into();
    assert!(decode(&body).is_err());
    body["expireAt"] = Value::Null;
    body["feeBps"] = Value::Null;
    body["feeMint"] = Value::Null;
    assert_eq!(decode(&body).unwrap().fee_bps, None);
}

#[test]
fn stock_stablecoin_mint_requires_official_usdt_program_without_rebase() {
    let address=shared_types::stocks::STOCK_SOLANA_USDT;
    let mut body=mint_response(address);
    assert!(parse_mint(&serde_json::to_vec(&body).unwrap(),address,6,1_000_000).is_err());
    body["result"]["value"][0]["owner"]=TOKEN.into();
    body["result"]["value"][0]["data"]["parsed"]["info"] = serde_json::json!({"isInitialized":true,"decimals":6});
    let mint=parse_mint(&serde_json::to_vec(&body).unwrap(),address,6,1_000_000).unwrap();
    assert_eq!(mint.ui_multiplier,"1");assert!(mint.extensions.is_empty());
}
