use super::*;
use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

fn owner() -> String {
    bs58::encode([3u8; 32]).into_string()
}
fn encoded() -> String {
    let mut b = vec![1];
    b.extend([0; 64]);
    b.extend([0x80, 1, 0, 3, 7]);
    for n in [3u8, 4, 7, 8, 5, 0] {
        b.extend([n; 32]);
    }
    b.extend(
        bs58::decode("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
            .into_vec()
            .unwrap(),
    );
    b.extend([9u8; 32]);
    b.extend([1, 4, 6, 0, 1, 2, 3, 5, 6, 0, 0]);
    STANDARD.encode(b)
}
fn fixture() -> StockComparison {
    let now = common::time::now_ms();
    let mint = StockMintEvidence {
        address: bs58::encode([6u8; 32]).into_string(),
        decimals: 6,
        ui_multiplier: "1.25".into(),
        slot: 10,
        chain_time_ms: now,
        checked_at_ms: now,
        next_change_at_ms: None,
        extensions: vec![],
    };
    StockComparison {
        asset: "MU.US".into(),
        issuer_docs: "fixture".into(),
        budget_usdc: "10".into(),
        keyed: false,
        buy: StockDexQuote {
            input_mint: comparison::SOLANA_USDC.into(),
            output_mint: mint.address.clone(),
            input_raw: "10000000".into(),
            output_raw: "19000".into(),
            minimum_output_raw: "17000".into(),
            router: "metis".into(),
            fee_bps: Some(10),
            fee_mint: Some(comparison::SOLANA_USDC.into()),
            requested_at_ms: now,
            received_at_ms: now,
            expires_at_ms: None,
        },
        mint,
        sell: None,
        sell_problem: None,
        quantity_limit: None,
    }
}
fn order() -> Value {
    let c = fixture();
    json!({"inputMint":c.buy.input_mint,"outputMint":c.buy.output_mint,"inAmount":"10000000","outAmount":"19000","otherAmountThreshold":"17000","swapMode":"ExactIn","router":"metis","mode":"ultra","feeBps":10,"feeMint":comparison::SOLANA_USDC,"transaction":encoded(),"taker":owner(),"requestId":"fixture-read-only","signatureFeeLamports":5000,"signatureFeePayer":owner(),"prioritizationFeeLamports":2000,"prioritizationFeePayer":owner(),"rentFeeLamports":2039280,"rentFeePayer":owner()})
}
fn token(index: usize, mint: &str, raw: &str) -> Value {
    json!({"accountIndex":index,"owner":owner(),"mint":mint,"programId":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA","uiTokenAmount":{"amount":raw,"decimals":6}})
}
fn simulated() -> Value {
    let c = fixture();
    let temporary = bs58::encode([8; 32]).into_string();
    json!({"context":{"slot":12},"value":{"err":null,"fee":7000,
        "preBalances":[10000000,2039280,2039280,0,1,1,1],"postBalances":[9993000,2039280,2039280,0,1,1,1],
        "accounts":[wallet_account(9993000)],"loadedAddresses":{"writable":[],"readonly":[]},
        "innerInstructions":[{"index":0,"instructions":[
            {"programId":funding::SYSTEM,"parsed":{"type":"createAccount","info":{"source":owner(),"newAccount":temporary,"lamports":2039280}},"stackHeight":2},
            {"programId":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA","parsed":{"type":"closeAccount","info":{"account":temporary,"destination":owner()}},"stackHeight":2}
        ]}],
        "preTokenBalances":[token(1,comparison::SOLANA_USDC,"10000000"),token(2,&c.mint.address,"0")],"postTokenBalances":[token(1,comparison::SOLANA_USDC,"0"),token(2,&c.mint.address,"18000")]}})
}
fn wallet_account(lamports: u64) -> Value {
    json!({"owner":funding::SYSTEM,"executable":false,"data":["","base64"],"lamports":lamports})
}
fn wallet_before() -> Value {
    json!({"context":{"slot":11},"value":wallet_account(10000000)})
}
fn native_encoded(raw: u64) -> String {
    let mut bytes = STANDARD.decode(encoded()).unwrap();
    let blockhash = 65 + 5 + 7 * 32;
    bytes[blockhash..blockhash + 8].copy_from_slice(&raw.to_le_bytes());
    STANDARD.encode(bytes)
}
fn native_simulated(raw: u64, fee: u64) -> Value {
    let mut v = simulated();
    let balance = 10_000_000 + raw * 11 - fee;
    v["value"]["fee"] = fee.into();
    v["value"]["postBalances"][0] = balance.into();
    v["value"]["postBalances"][2] = (2_039_280 - raw * 11).into();
    v["value"]["accounts"][0] = wallet_account(balance);
    v["value"]["innerInstructions"] = json!([]);
    v["value"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] =
        (10_000_000 - raw).to_string().into();
    v["value"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] = "0".into();
    v
}
pub(super) fn cost() -> StockChainCost {
    let c = fixture();
    StockChainCost {
        transaction: None,
        asset: c.asset.clone(),
        direction: StockChainDirection::Buy,
        wallet_address: owner(),
        mint: c.mint.clone(),
        quote: c.buy.clone(),
        transaction_fingerprint: "fixture".into(),
        checked_at_ms: c.mint.checked_at_ms,
        valid_until_ms: c.mint.checked_at_ms + 10000,
        provider_fees: vec![
            StockNativeFee {
                kind: "signature".into(),
                lamports: Some("5000".into()),
                payer: Some(owner()),
            },
            StockNativeFee {
                kind: "priority".into(),
                lamports: Some("2000".into()),
                payer: Some(owner()),
            },
            StockNativeFee {
                kind: "rent".into(),
                lamports: Some("2039280".into()),
                payer: Some(owner()),
            },
        ],
        network_fee_lamports: None,
        wallet_debit_lamports: None,
        wallet_budget_lamports: None,
        wallet_required_lamports: None,
        native_valuation: None,
        simulation_slot: None,
        simulation_passed: false,
        problems: vec![],
    }
}

#[tokio::test]
async fn stock_chain_cost_loopback_build_and_rpc_checks_fees_rent_and_wallet_deltas_without_submission(
) {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route(
            "/order",
            get(
                |State(calls): State<Arc<Mutex<Vec<String>>>>,
                 Query(q): Query<HashMap<String, String>>| async move {
                    assert_eq!(q["inputMint"], comparison::SOLANA_USDC);
                    if q["outputMint"] != STOCK_WRAPPED_SOL {
                        calls.lock().unwrap().push("order".into());
                        assert_eq!(q.len(), 4);
                        assert_eq!(q["taker"], owner());
                        assert_eq!(q["amount"], "10000000");
                        Json(order())
                    } else {
                        assert_eq!(q["outputMint"], STOCK_WRAPPED_SOL);
                        let raw=q["amount"].parse::<u64>().unwrap();
                        let mut value=json!({"inputMint":comparison::SOLANA_USDC,"outputMint":STOCK_WRAPPED_SOL,
                            "inAmount":raw.to_string(),"outAmount":(raw*11).to_string(),"otherAmountThreshold":(raw*10).to_string(),
                            "swapMode":"ExactIn","router":"metis","transaction":null,"taker":null});
                        if let Some(taker)=q.get("taker") {
                            assert_eq!(q.len(),4);
                            assert_eq!(taker,&owner());
                            calls.lock().unwrap().push(format!("topup:{raw}"));
                            value["taker"]=taker.clone().into();
                            value["transaction"]=native_encoded(raw).into();
                            value["signatureFeePayer"]=owner().into();
                            value["requestId"]=format!("native-{raw}").into();
                            value["mode"]="manual".into();
                            value["lastValidBlockHeight"]="100".into();
                        } else {
                            assert_eq!(q.len(),3);
                            calls.lock().unwrap().push(format!("quote:{raw}"));
                        }
                        Json(value)
                    }
                },
            ),
        )
        .route(
            "/rpc",
            post(
                |State(calls): State<Arc<Mutex<Vec<String>>>>, Json(r): Json<Value>| async move {
                    let method = r["method"].as_str().unwrap();
                    calls.lock().unwrap().push(method.into());
                    let result = match method {
                        "getGenesisHash" => json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
                        "getFeeForMessage" => {
                            assert!([encoded(),native_encoded(700),native_encoded(2700)].iter().any(|b|r["params"][0]==transaction::inspect(b,&owner()).unwrap().message));
                            let fee=if r["params"][0]==transaction::inspect(&encoded(),&owner()).unwrap().message {7000} else {20000};
                            json!({"context":{"slot":11},"value":fee})
                        }
                        "getAccountInfo" => {
                            assert_eq!(r["params"][0], owner());
                            assert_eq!(r["params"][1]["encoding"], "base64");
                            assert_eq!(r["params"][1]["minContextSlot"], 11);
                            wallet_before()
                        }
                        "getMinimumBalanceForRentExemption" => {
                            assert_eq!(r["params"], json!([0,{"commitment":"confirmed"}]));
                            json!(890880)
                        }
                        "simulateTransaction" => {
                            assert_eq!(r["params"][1]["sigVerify"], false);
                            assert_eq!(r["params"][1]["replaceRecentBlockhash"], false);
                            assert_eq!(r["params"][1]["minContextSlot"], 11);
                            assert_eq!(r["params"][1]["innerInstructions"], true);
                            assert_eq!(r["params"][1]["accounts"],json!({"encoding":"base64","addresses":[owner()]}));
                            if r["params"][0]==encoded(){simulated()}
                            else if r["params"][0]==native_encoded(700){native_simulated(700,20000)}
                            else {assert_eq!(r["params"][0],native_encoded(2700));native_simulated(2700,20000)}
                        }
                        other => panic!("unexpected fund action or RPC {other}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":r["id"],"result":result}))
                },
            ),
        )
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let result = read_with(
        &http,
        &format!("{root}/order"),
        None,
        &http,
        &format!("{root}/rpc"),
        &StockChainCostRequest {
            asset: "MU.US".into(),
            wallet_address: owner(),
            direction: StockChainDirection::Buy,
        },
        &fixture(),
    )
    .await;
    let result = result.unwrap();
    assert!(result.simulation_passed, "{:?}", result.problems);
    assert_eq!(result.network_fee_lamports.as_deref(), Some("7000"));
    assert_eq!(result.wallet_debit_lamports.as_deref(), Some("7000"));
    // getFeeForMessage already includes the CU priority fee: do not add it twice.
    assert_eq!(result.wallet_budget_lamports.as_deref(), Some("2046280"));
    // Rent is funded then refunded: keep it in required liquidity, not in SOL replacement cost.
    assert_eq!(result.wallet_required_lamports.as_deref(), Some("2937160"));
    execution::recheck_original_with(&http,&format!("{root}/rpc"),&result).await.unwrap();
    assert_eq!(
        result.native_usdc_budget(common::time::now_ms()).as_deref(),
        Some("0.0027")
    );
    assert_eq!(
        result
            .complete_native_usdc_budget(common::time::now_ms())
            .as_deref(),
        Some("0.0027")
    );
    let proof = result
        .native_valuation
        .as_ref()
        .unwrap()
        .replenishment
        .as_ref()
        .unwrap();
    assert_eq!(proof.minimum_credit_lamports, "7000");
    assert_eq!(proof.wallet_outflow_lamports, "20000");
    assert_eq!(proof.wallet_required_lamports, "910880");
    let mut topup=result.clone();
    topup.quote=result.native_valuation.as_ref().unwrap().quote.clone();
    topup.transaction=Some(proof.transaction.clone());
    topup.transaction_fingerprint=proof.transaction_fingerprint.clone();
    topup.network_fee_lamports=Some(proof.network_fee_lamports.clone());
    topup.wallet_debit_lamports=Some(proof.wallet_outflow_lamports.clone());
    topup.wallet_budget_lamports=Some(proof.wallet_outflow_lamports.clone());
    topup.wallet_required_lamports=Some(proof.wallet_required_lamports.clone());
    topup.native_valuation=None;topup.provider_fees.clear();
    topup.checked_at_ms=proof.checked_at_ms;topup.valid_until_ms=proof.valid_until_ms;
    execution::recheck_original_with(&http,&format!("{root}/rpc"),&topup).await.unwrap();
    topup.wallet_required_lamports=Some("10000001".into());
    assert!(execution::recheck_original_with(&http,&format!("{root}/rpc"),&topup).await.is_err());
    assert_eq!(
        proof.transaction_fingerprint,
        transaction::inspect(&native_encoded(2700), &owner())
            .unwrap()
            .fingerprint
    );
    assert_eq!(
        result.total_native_required_lamports(common::time::now_ms()),
        Some(2937160)
    );
    assert!(result.problems.is_empty());
    let before_replacement=calls.lock().unwrap().clone();
    let mut expired=result.clone();expired.valid_until_ms=common::time::now_ms()-10_000;
    let fresh=read_native_replacement_with(&http,&format!("{root}/order"),None,&http,&format!("{root}/rpc"),&expired,7000).await.unwrap();
    assert_eq!(fresh.complete_budget("7000",&owner(),common::time::now_ms()).as_deref(),Some("0.0027"));
    assert!(fresh.replenishment.as_ref().unwrap().valid_until_ms>expired.valid_until_ms);
    server.abort();let _=server.await;
    assert_eq!(
        before_replacement[..19].to_vec(),
        vec![
            "order",
            "getGenesisHash",
            "getFeeForMessage",
            "getAccountInfo",
            "simulateTransaction",
            "getMinimumBalanceForRentExemption",
            "quote:1000000",
            "topup:700",
            "getGenesisHash",
            "getFeeForMessage",
            "getAccountInfo",
            "simulateTransaction",
            "getMinimumBalanceForRentExemption",
            "topup:2700",
            "getGenesisHash",
            "getFeeForMessage",
            "getAccountInfo",
            "simulateTransaction",
            "getMinimumBalanceForRentExemption"
        ]
    );
    assert_eq!(before_replacement.len(),34);
    for recheck in before_replacement[19..].chunks_exact(5) {
        assert_eq!(recheck,["getGenesisHash","getFeeForMessage","getAccountInfo","simulateTransaction","getMinimumBalanceForRentExemption"]);
    }
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["transaction"]["transaction_base64"], encoded());
    assert_eq!(serialized["transaction"]["request_id"], "fixture-read-only");
    execution::validate_artifact(&result).unwrap();
}

#[test]
fn stock_chain_cost_rejects_unproven_simulation_amounts_and_missing_fees() {
    let tx = transaction::inspect(&encoded(), &owner()).unwrap();
    let cost = cost();
    let verify = |v: &Value| simulation::check(v, &tx, &cost, 7000, 11);
    assert_eq!(verify(&simulated()).unwrap(), (7000, 7000, 12));
    for field in ["err", "fee", "preBalances", "postTokenBalances"] {
        let mut v = simulated();
        v["value"].as_object_mut().unwrap().remove(field);
        assert!(verify(&v).is_err(), "{field}");
    }
    let mut v = simulated();
    v["value"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] = "16000".into();
    assert!(verify(&v).is_err());
    v = simulated();
    v["value"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] = "1".into();
    assert!(verify(&v).is_err());
    v = simulated();
    v["value"]["postTokenBalances"][1]["uiTokenAmount"]["decimals"] = 9.into();
    assert!(verify(&v).is_err());
    v = simulated();
    v["value"]["err"] = json!({"InstructionError":[1,"Custom"]});
    assert!(verify(&v).is_err());
    v = simulated();
    v["context"]["slot"] = 9.into();
    assert!(verify(&v).is_err());
    let mut fees = cost.provider_fees;
    fees[2].lamports = None;
    assert!(wallet_budget(&fees, &owner()).is_none());
    fees[2].lamports = Some("0".into());
    fees[2].payer = None;
    assert_eq!(wallet_budget(&fees, &owner()), Some(7000));
    fees[1].payer = Some(bs58::encode([9u8; 32]).into_string());
    assert_eq!(wallet_budget(&fees, &owner()), Some(5000));
    fees[0].payer = None;
    assert!(wallet_budget(&fees, &owner()).is_none());
}

#[tokio::test]
async fn stock_stablecoin_loopback_unsigned_quote_simulation_and_preview_preserve_raw_units() {
    let calls=Arc::new(Mutex::new(Vec::<String>::new()));
    let app=Router::new().route("/order",get(|State(calls):State<Arc<Mutex<Vec<String>>>>,Query(q):Query<HashMap<String,String>>|async move {
        assert_eq!(q.len(),4);assert_eq!(q["inputMint"],STOCK_SOLANA_USDT);
        assert_eq!(q["outputMint"],comparison::SOLANA_USDC);assert_eq!(q["amount"],"10000000");
        assert_eq!(q["taker"],owner());calls.lock().unwrap().push("unsigned_order".into());
        let mut v=order();
        v["inputMint"]=STOCK_SOLANA_USDT.into();v["outputMint"]=comparison::SOLANA_USDC.into();
        v["outAmount"]="9950000".into();v["otherAmountThreshold"]="9900000".into();
        for field in ["signatureFeeLamports","prioritizationFeeLamports","rentFeeLamports"] {v[field]=0.into();}
        Json(v)
    })).route("/rpc",post(|State(calls):State<Arc<Mutex<Vec<String>>>>,Json(r):Json<Value>|async move {
        let method=r["method"].as_str().unwrap();calls.lock().unwrap().push(method.into());
        let result=match method {
            "getGenesisHash"=>json!(super::super::rpc::SOLANA_MAINNET_GENESIS_HASH),
            "getFeeForMessage"=>json!({"context":{"slot":11},"value":0}),
            "getAccountInfo"=>wallet_before(),
            "getMinimumBalanceForRentExemption"=>json!(890880),
            "simulateTransaction"=>{
                assert_eq!(r["params"][0],encoded());assert_eq!(r["params"][1]["sigVerify"],false);
                let mut s=simulated();s["value"]["fee"]=0.into();
                s["value"]["postBalances"][0]=10_000_000.into();s["value"]["accounts"][0]=wallet_account(10_000_000);
                s["value"]["innerInstructions"]=json!([]);
                s["value"]["preTokenBalances"]=json!([token(1,STOCK_SOLANA_USDT,"20000000"),token(2,comparison::SOLANA_USDC,"0")]);
                s["value"]["postTokenBalances"]=json!([token(1,STOCK_SOLANA_USDT,"10000000"),token(2,comparison::SOLANA_USDC,"9940000")]);s
            },
            other=>panic!("unexpected fund action {other}"),
        };Json(json!({"jsonrpc":"2.0","id":r["id"],"result":result}))
    })).with_state(calls.clone());
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
    let http=reqwest::Client::builder().no_proxy().build().unwrap();
    let now=common::time::now_ms();
    let mut mint=fixture().mint;mint.address=STOCK_SOLANA_USDT.into();mint.ui_multiplier="1".into();
    let mut quote=fixture().buy;quote.input_mint=STOCK_SOLANA_USDT.into();quote.output_mint=comparison::SOLANA_USDC.into();
    quote.output_raw="9950000".into();quote.minimum_output_raw="9900000".into();
    let request=StockChainCostRequest{asset:"USDT".into(),wallet_address:owner(),direction:StockChainDirection::Sell};
    let result=read_seed_with(&http,&format!("{root}/order"),None,&http,&format!("{root}/rpc"),&request,&mint,&quote).await;
    let cost=result.unwrap();
    execution::recheck_original_with(&http,&format!("{root}/rpc"),&cost).await.unwrap();
    for field in ["input","required","available","fee","expiry"] {
        let mut invalid=cost.clone();
        match field {
            "input"=>invalid.quote.input_raw="10000001".into(),
            "required"=>invalid.wallet_required_lamports=None,
            "available"=>invalid.wallet_required_lamports=Some("10000001".into()),
            "fee"=>invalid.network_fee_lamports=None,
            _=>invalid.valid_until_ms=common::time::now_ms()-1,
        }
        assert!(execution::recheck_original_with(&http,&format!("{root}/rpc"),&invalid).await.is_err(),"{field}");
    }
    server.abort();let _=server.await;
    assert!(cost.simulation_passed,"{:?}",cost.problems);
    assert_eq!(cost.network_fee_lamports.as_deref(),Some("0"));
    assert_eq!(cost.wallet_debit_lamports.as_deref(),Some("0"));
    let wallet=StockWalletEvidence{owner:owner(),mint:STOCK_SOLANA_USDT.into(),stock_raw:Some("20000000".into()),usdc_raw:Some("0".into()),
        sol_lamports:Some("10000000".into()),checked_at_ms:now,problems:vec![]};
    let request=StockStablecoinRequest{asset:"MU.US".into(),wallet_address:owner(),input_usdt:"10".into(),target_usdc:"10".into(),keyed:false};
    let p=stablecoin_preview(request,wallet,cost.quote.clone(),Some(cost),vec![],common::time::now_ms()).unwrap();
    assert_eq!(p.minimum_usdc,"9.9");assert_eq!(p.shortfall_usdc,"0.1");
    assert_eq!(p.after_native_cost_usdc.as_deref(),Some("9.9"));
    assert!(!p.can_reserve(p.checked_at_ms), "the target shortfall must prevent reservation");
    assert_eq!(calls.lock().unwrap().iter().filter(|s|*s=="unsigned_order").count(),1);
    if let Ok(path)=std::env::var("STOCK_STABLECOIN_CAPTURE_PATH") {
        std::fs::write(path,serde_json::to_vec_pretty(&p).unwrap()).unwrap();
    }
}

#[test]
fn stock_native_topup_requires_native_sol_not_wsol_and_exact_usdc_debit() {
    let tx = transaction::inspect(&native_encoded(1400), &owner()).unwrap();
    let mut cost = cost();
    cost.quote.input_mint = comparison::SOLANA_USDC.into();
    cost.quote.output_mint = STOCK_WRAPPED_SOL.into();
    cost.quote.input_raw = "1400".into();
    let verify = |v: &Value| simulation::native_change(v, &tx, &cost, 7000, 11);
    assert_eq!(verify(&native_simulated(1400, 7000)).unwrap(), 8400);
    for invalid in ["fee", "error", "slot", "usdc", "wsol", "stock", "missing"] {
        let mut v = native_simulated(1400, 7000);
        match invalid {
            "fee" => v["value"]["fee"] = 7001.into(),
            "error" => v["value"]["err"] = json!({"InstructionError":[0,"failure"]}),
            "slot" => v["context"]["slot"] = 10.into(),
            "usdc" => {
                v["value"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] = "9998599".into()
            }
            "wsol" => {
                let mut wsol = token(3, STOCK_WRAPPED_SOL, "15400");
                wsol["uiTokenAmount"]["decimals"] = 9.into();
                v["value"]["postTokenBalances"]
                    .as_array_mut()
                    .unwrap()
                    .push(wsol);
            }
            "stock" => v["value"]["preTokenBalances"][1]["uiTokenAmount"]["amount"] = "1".into(),
            _ => v["value"]["preTokenBalances"] = Value::Null,
        }
        assert!(verify(&v).is_err(), "{invalid}");
    }
}

#[test]
fn stock_chain_cost_requires_unsigned_wallet_slot_and_complete_message_header() {
    assert!(transaction::inspect(&encoded(), &owner()).is_ok());
    assert!(transaction::inspect(&encoded(), &bs58::encode([4u8; 32]).into_string()).is_err());
    let mut bytes = STANDARD.decode(encoded()).unwrap();
    bytes[1] = 1;
    assert!(transaction::inspect(&STANDARD.encode(&bytes), &owner()).is_err());
    bytes[1] = 0;
    bytes[0] = 2;
    assert!(transaction::inspect(&STANDARD.encode(&bytes), &owner()).is_err());
    assert!(transaction::inspect(&STANDARD.encode([0u8; 1233]), &owner()).is_err());
    for length in [0, 1, 64, 100, 140] {
        assert!(transaction::inspect(&STANDARD.encode(&bytes[..length]), &owner()).is_err());
    }
    let original = STANDARD.decode(encoded()).unwrap();
    assert!(
        transaction::inspect(&STANDARD.encode(&original[..original.len() - 1]), &owner()).is_err()
    );
    let mut invalid = original.clone();
    invalid.push(0);
    assert!(transaction::inspect(&STANDARD.encode(&invalid), &owner()).is_err());
    let mut invalid = original.clone();
    let count_offset = 65 + 5 + 7 * 32 + 32;
    invalid[count_offset + 1] = 255;
    assert!(transaction::inspect(&STANDARD.encode(&invalid), &owner()).is_err());
    // v0 lookup descriptors must be fully consumed before accepting their account indices.
    let mut lookup = original;
    lookup.pop();
    lookup.push(1);
    lookup.extend([11; 32]);
    lookup.extend([1, 0, 1, 1]);
    let tx = transaction::inspect(&STANDARD.encode(&lookup), &owner()).unwrap();
    assert_eq!(tx.loaded_counts, [1, 1]);
    lookup.pop();
    assert!(transaction::inspect(&STANDARD.encode(&lookup), &owner()).is_err());
}

#[test]
fn stock_funding_counts_root_and_inner_outflows_and_keeps_sponsored_fees_separate() {
    let mut tx = transaction::inspect(&encoded(), &owner()).unwrap();
    assert_eq!(
        funding::required(&simulated(), &tx, &wallet_before(), 890880, 7000, 7000).unwrap(),
        2937160
    );
    let mut data = 2_u32.to_le_bytes().to_vec();
    data.extend(50_u64.to_le_bytes());
    tx.instructions.push(transaction::Instruction {
        program: 5,
        accounts: vec![0, 3],
        data,
    });
    assert_eq!(
        funding::required(&simulated(), &tx, &wallet_before(), 890880, 7000, 7000).unwrap(),
        2937210
    );

    let mut tx = transaction::inspect(&encoded(), &owner()).unwrap();
    tx.keys.swap(0, 1);
    tx.wallet_index = 1;
    tx.fee_payer = tx.keys[0].clone();
    let mut v = simulated();
    v["value"]["preBalances"].as_array_mut().unwrap().swap(0, 1);
    v["value"]["postBalances"] = json!([2032280, 10000000, 2039280, 0, 1, 1, 1]);
    v["value"]["accounts"][0] = wallet_account(10000000);
    assert_eq!(
        funding::required(&v, &tx, &wallet_before(), 890880, 7000, 0).unwrap(),
        2930160
    );
    v["value"]["innerInstructions"] = json!([]);
    assert_eq!(
        funding::required(&v, &tx, &wallet_before(), 890880, 7000, 0).unwrap(),
        0
    );
}

#[test]
fn stock_funding_keeps_missing_trace_or_changed_wallet_unknown() {
    let tx = transaction::inspect(&encoded(), &owner()).unwrap();
    let verify = |v: &Value, before: &Value| funding::required(v, &tx, before, 890880, 7000, 7000);
    for field in ["innerInstructions", "loadedAddresses", "accounts"] {
        let mut v = simulated();
        v["value"].as_object_mut().unwrap().remove(field);
        assert!(verify(&v, &wallet_before()).is_err(), "{field}");
    }
    for case in [
        "duplicate",
        "unknown_program",
        "missing_amount",
        "ownership",
        "missing_group",
        "wrong_key",
        "post_owner",
        "initial_owner",
        "race",
        "unexplained",
    ] {
        let mut v = simulated();
        let mut before = wallet_before();
        match case {
            "duplicate" => {
                let group = v["value"]["innerInstructions"][0].clone();
                v["value"]["innerInstructions"]
                    .as_array_mut()
                    .unwrap()
                    .push(group);
            }
            "unknown_program" => {
                v["value"]["innerInstructions"][0]["instructions"][0]["parsed"]["type"] =
                    "unknown".into()
            }
            "missing_amount" => {
                v["value"]["innerInstructions"][0]["instructions"][0]["parsed"]["info"]
                    ["lamports"] = Value::Null
            }
            "ownership" => {
                v["value"]["innerInstructions"][0]["instructions"][0]["parsed"] =
                    json!({"type":"assign","info":{"account":owner()}})
            }
            "missing_group" => v["value"]["innerInstructions"][0]["instructions"] = Value::Null,
            "wrong_key" => {
                v["value"]["innerInstructions"][0]["instructions"][0]["programId"] = "fake".into()
            }
            "post_owner" => v["value"]["accounts"][0]["owner"] = "program".into(),
            "initial_owner" => before["value"]["owner"] = "program".into(),
            "race" => before["value"]["lamports"] = 10000001_u64.into(),
            _ => {
                v["value"]["innerInstructions"] = json!([]);
                assert!(funding::required(&v, &tx, &before, 890880, 7000, 8000).is_err());
                continue;
            }
        }
        assert!(verify(&v, &before).is_err(), "{case}");
    }
}

#[test]
fn stock_funding_resolves_lookup_accounts_and_raw_system_cpi() {
    let mut tx = transaction::inspect(&encoded(), &owner()).unwrap();
    tx.loaded_counts = [1, 1];
    let mut v = simulated();
    let destination = bs58::encode([12; 32]).into_string();
    v["value"]["loadedAddresses"] =
        json!({"writable":[destination],"readonly":[bs58::encode([13;32]).into_string()]});
    for field in ["preBalances", "postBalances"] {
        v["value"][field]
            .as_array_mut()
            .unwrap()
            .extend([json!(0), json!(1)]);
    }
    let mut data = 2_u32.to_le_bytes().to_vec();
    data.extend(2039280_u64.to_le_bytes());
    v["value"]["innerInstructions"][0]["instructions"] = json!([{"programId":funding::SYSTEM,"accounts":[owner(),destination],"data":bs58::encode(&data).into_string(),"stackHeight":2}]);
    assert_eq!(
        funding::required(&v, &tx, &wallet_before(), 890880, 7000, 7000).unwrap(),
        2937160
    );
    v["value"]["loadedAddresses"]["readonly"] = json!([owner()]);
    assert!(funding::required(&v, &tx, &wallet_before(), 890880, 7000, 7000).is_err());
}
