use super::*;
use axum::{extract::State, routing::post, Json, Router};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn owner() -> String {
    bs58::encode([3u8; 32]).into_string()
}
fn mint() -> StockMintEvidence {
    StockMintEvidence {
        address: bs58::encode([5u8; 32]).into_string(),
        decimals: 6,
        ui_multiplier: "1.25".into(),
        slot: 10,
        chain_time_ms: 1000,
        checked_at_ms: 1000,
        next_change_at_ms: None,
        extensions: vec![],
    }
}
fn row(owner: &str, mint: &str, seed: u8, state: &str, raw: &str) -> Value {
    json!({
        "pubkey":bs58::encode([seed;32]).into_string(),
        "account":{"owner":TOKEN,"executable":false,
            "data":{"parsed":{"type":"account","info":{
                "owner":owner,"mint":mint,"state":state,
                "tokenAmount":{"amount":raw,"decimals":6}
            }}}
        }
    })
}
#[test]
fn stock_wallet_inventory_rejects_wrong_identity_duplicate_accounts_and_frozen_spend() {
    let owner = owner();
    let mint = mint();
    let a = row(&owner, &mint.address, 8, "initialized", "1234");
    let b = row(&owner, &mint.address, 9, "frozen", "999999");
    let valid = json!({"context":{"slot":10},"value":[a.clone(),b]});
    assert_eq!(
        token_balance(&valid, &owner, &mint.address, 6, 10).unwrap(),
        "1234"
    );
    let duplicate = json!({"context":{"slot":10},"value":[a.clone(),a.clone()]});
    assert!(token_balance(&duplicate, &owner, &mint.address, 6, 10).is_err());
    for path in ["owner", "mint"] {
        let mut wrong = a.clone();
        wrong["account"]["data"]["parsed"]["info"][path] = "wrong".into();
        assert!(token_balance(
            &json!({"context":{"slot":10},"value":[wrong]}),
            &owner,
            &mint.address,
            6,
            10
        )
        .is_err());
    }
    assert!(token_balance(&valid, &owner, &mint.address, 9, 10).is_err());
    assert!(token_balance(&valid, &owner, &mint.address, 6, 11).is_err());
    assert_eq!(
        token_balance(
            &json!({"context":{"slot":10},"value":[]}),
            &owner,
            &mint.address,
            6,
            10
        )
        .unwrap(),
        "0"
    );
}

#[test]
fn stock_stablecoin_wallet_rejects_wrong_usdt_token_program() {
    let address=shared_types::stocks::STOCK_SOLANA_USDT;
    let mut token=row(&owner(),address,8,"initialized","10123456");
    assert_eq!(token_balance(&json!({"context":{"slot":10},"value":[token.clone()]}),&owner(),address,6,10).unwrap(),"10123456");
    token["account"]["owner"]=TOKEN_2022.into();
    assert!(token_balance(&json!({"context":{"slot":10},"value":[token]}),&owner(),address,6,10).is_err());
}
#[tokio::test]
async fn stock_wallet_inventory_loopback_rpc_reads_mainnet_identity_raw_balances_and_gas_without_signing(
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app=Router::new().route("/",post(|State(calls):State<Arc<AtomicUsize>>,Json(request):Json<Value>|async move{
        calls.fetch_add(1,Ordering::SeqCst);
        let result=match request["method"].as_str().unwrap(){
            "getGenesisHash"=>json!(MAINNET),
            "getTokenAccountsByOwner"=>{assert_eq!(request["params"][2]["minContextSlot"],10);let mint=request["params"][1]["mint"].as_str().unwrap();json!({"context":{"slot":10},"value":[row(&owner(),mint,8,"initialized",if mint==SOLANA_USDC{"10000000"}else{"16000"})]})},
            "getBalance"=>json!({"context":{"slot":10},"value":1000000}),
            other=>panic!("unexpected RPC {other}"),
        };Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
    })).with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let result = read_with(
        &reqwest::Client::builder().no_proxy().build().unwrap(),
        &url,
        &owner(),
        &mint(),
    )
    .await;
    server.abort();
    let _ = server.await;
    let result = result.unwrap();
    assert_eq!(result.stock_raw.as_deref(), Some("16000"));
    assert_eq!(result.usdc_raw.as_deref(), Some("10000000"));
    assert_eq!(result.sol_lamports.as_deref(), Some("1000000"));
    assert!(result.problems.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}
