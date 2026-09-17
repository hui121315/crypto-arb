use super::*;
use axum::{extract::State, routing::post, Json, Router};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

type Replies = Arc<Mutex<VecDeque<(String, Value)>>>;
pub(in crate::services::onchain_comparison) struct RpcFixture {
    pub(in crate::services::onchain_comparison) url: String,
    task: tokio::task::JoinHandle<()>,
    pub(in crate::services::onchain_comparison) replies: Replies,
}
impl Drop for RpcFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(in crate::services::onchain_comparison) async fn fixture(
    replies: Vec<(&str, Value)>,
) -> RpcFixture {
    let replies: Replies = Arc::new(Mutex::new(
        replies
            .into_iter()
            .map(|(method, value)| (method.to_owned(), value))
            .collect(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/",
            post(
                |State(replies): State<Replies>, Json(request): Json<Value>| async move {
                    let (expected, result) = replies
                        .lock()
                        .unwrap()
                        .pop_front()
                        .expect("unexpected RPC request");
                    assert_eq!(request["method"], expected);
                    if expected == "eth_getBlockByNumber" {
                        assert_eq!(request["params"], json!(["0x64", false]));
                    }
                    if expected == "debug_traceTransaction" {
                        assert_eq!(request["params"][1]["tracer"], "callTracer");
                        assert_eq!(request["params"][1]["timeout"], "5s");
                    }
                    if expected == "getTransaction" {
                        assert_eq!(request["params"][1]["commitment"], "finalized");
                        assert_eq!(request["params"][1]["encoding"], "jsonParsed");
                        assert_eq!(request["params"][1]["maxSupportedTransactionVersion"], 0);
                    }
                    if expected == "eth_call" {
                        assert_eq!(
                            request["params"][0]["to"],
                            "0x420000000000000000000000000000000000000F"
                        );
                        let selector = common::signing::keccak256(b"getOperatorFee(uint256)");
                        assert_eq!(
                            request["params"][0]["data"],
                            format!("0x{}{:064x}", hex::encode(&selector[..4]), 21_000)
                        );
                        assert_eq!(
                            request["params"][1],
                            json!({"blockHash":"0xcanonical","requireCanonical":true})
                        );
                    }
                    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
                },
            ),
        )
        .with_state(replies.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    RpcFixture { url, task, replies }
}

fn scope() -> CreditScope {
    CreditScope {
        destination: format!("0x{:040x}", 0xab),
        asset_address: format!("0x{:040x}", 0xcd),
        expected_raw: 100,
        required_confirmations: 2,
    }
}
fn transfer(from: u64, to: u64, amount: u128, index: u64) -> Value {
    json!({"address":scope().asset_address, "topics":[ERC20_TRANSFER_TOPIC,format!("0x{from:064x}"),format!("0x{to:064x}")],
        "data":format!("0x{amount:064x}"),"logIndex":format!("0x{index:x}"),"removed":false})
}
fn receipt() -> Value {
    json!({"transactionHash":"0xtransaction", "blockHash":"0xcanonical", "blockNumber":"0x64", "status":"0x1",
        "logs":[transfer(1,0xab,150,0),transfer(0xab,2,30,1)]})
}
pub(in crate::services::onchain_comparison) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap()
}

#[path = "network_cost_tests.rs"]
mod network_cost_tests;

#[tokio::test]
async fn evm_credit_uses_canonical_receipt_and_wallet_net_amount() {
    let fixture = fixture(vec![
        ("eth_chainId", json!("0x2105")),
        ("eth_getTransactionReceipt", receipt()),
        (
            "eth_getBlockByNumber",
            json!({"hash":"0xcanonical", "transactions":vec![format!("0x{:064x}", 1); 2048]}),
        ),
        ("eth_blockNumber", json!("0x65")),
    ])
    .await;
    let result = check_on_rpc(&client(), &fixture.url, "base", &scope(), "0xtransaction").await;
    assert!(matches!(
        result,
        DestinationCreditCheck::Credited {
            credited_amount_raw: 120,
            confirmations: Some(2),
            ..
        }
    ));
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn different_rpc_chain_or_reorg_cannot_prove_credit() {
    let wrong = fixture(vec![("eth_chainId", json!("0x1"))]).await;
    assert!(matches!(
        check_on_rpc(&client(), &wrong.url, "base", &scope(), "0xtransaction").await,
        DestinationCreditCheck::Rejected { .. }
    ));
    let reorg = fixture(vec![
        ("eth_chainId", json!("0x2105")),
        ("eth_getTransactionReceipt", receipt()),
        ("eth_getBlockByNumber", json!({"hash":"0xother-fork"})),
    ])
    .await;
    assert!(matches!(
        check_on_rpc(&client(), &reorg.url, "base", &scope(), "0xtransaction").await,
        DestinationCreditCheck::Pending { .. }
    ));
}

#[test]
fn transfer_receipts_reject_duplicates_removed_logs_and_self_transfer_credit() {
    let mut value = receipt();
    value["logs"] = json!([transfer(1, 0xab, 150, 0), transfer(1, 0xab, 150, 0)]);
    assert!(evm_token_credit(&value, &scope()).is_err());
    value["logs"] = json!([transfer(0xab, 0xab, 200, 0)]);
    assert_eq!(evm_token_credit(&value, &scope()), Ok(0));
    value["logs"][0]["removed"] = json!(true);
    assert!(evm_token_credit(&value, &scope()).is_err());
}

#[tokio::test]
async fn native_bridge_credit_reads_internal_calls_instead_of_transaction_value() {
    let mut native_scope = scope();
    native_scope.asset_address = EVM_NATIVE_TOKEN_ADDRESS.into();
    let tx = json!({"hash":"0xtransaction","blockHash":"0xcanonical","from":format!("0x{:040x}",1),"to":format!("0x{:040x}",2),"value":"0x0","input":"0x1234"});
    let trace = json!({"type":"CALL","from":tx["from"],"to":tx["to"],"value":"0x0","calls":[
        {"type":"CALL","from":tx["to"],"to":native_scope.destination,"value":"0x78"}]});
    let fixture = fixture(vec![
        ("eth_chainId", json!("0x2105")),
        ("eth_getTransactionReceipt", receipt()),
        ("eth_getBlockByNumber", json!({"hash":"0xcanonical"})),
        ("eth_blockNumber", json!("0x65")),
        ("eth_getTransactionByHash", tx),
        ("debug_traceTransaction", trace),
    ])
    .await;
    assert!(matches!(
        check_on_rpc(
            &client(),
            &fixture.url,
            "base",
            &native_scope,
            "0xtransaction"
        )
        .await,
        DestinationCreditCheck::Credited {
            credited_amount_raw: 120,
            ..
        }
    ));
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn solana_credit_requires_the_signature_slot_owner_mint_and_balance_delta() {
    let scope = CreditScope {
        destination: "wallet".into(),
        asset_address: "mint".into(),
        expected_raw: 100,
        required_confirmations: 2,
    };
    let transaction = json!({"slot":120,"transaction":{"signatures":["signature"],
        "message":{"accountKeys":["wallet","token-account","other-token-account"]}},"meta":{"err":null,
        "preTokenBalances":[{"accountIndex":1,"owner":"wallet","mint":"mint","uiTokenAmount":{"amount":"30"}}],
        "postTokenBalances":[{"accountIndex":1,"owner":"wallet","mint":"mint","uiTokenAmount":{"amount":"150"}},
            {"accountIndex":2,"owner":"someone-else","mint":"mint","uiTokenAmount":{"amount":"999999"}}]}});
    for case in [
        "valid",
        "missing_owner",
        "duplicate_account",
        "missing_result",
    ] {
        let mut response = transaction.clone();
        match case {
            "missing_owner" => response["meta"]["preTokenBalances"][0]["owner"] = Value::Null,
            "duplicate_account" => {
                response["meta"]["postTokenBalances"][1] =
                    response["meta"]["postTokenBalances"][0].clone();
            }
            "missing_result" => {
                response["meta"].as_object_mut().unwrap().remove("err");
            }
            _ => {}
        }
        let fixture = fixture(vec![("getGenesisHash",json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            ("getSignatureStatuses",json!({"value":[{"slot":120,"err":null,"confirmations":null,"confirmationStatus":"finalized"}]})),
            ("getTransaction",response)]).await;
        let result = check_on_rpc(&client(), &fixture.url, "solana", &scope, "signature").await;
        match case {
            "valid" => assert!(matches!(
                result,
                DestinationCreditCheck::Credited {
                    credited_amount_raw: 120,
                    ..
                }
            )),
            "missing_result" => assert!(matches!(result, DestinationCreditCheck::Pending { .. })),
            _ => assert!(
                matches!(result, DestinationCreditCheck::Rejected { .. }),
                "{case}"
            ),
        }
        assert!(fixture.replies.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn solana_rpc_credit_separates_native_output_network_fee_and_wrapped_rent_refund() {
    let scope = CreditScope {
        destination: "wallet".into(),
        asset_address: SOLANA_WRAPPED_SOL_MINT.into(),
        expected_raw: 1_000_000,
        required_confirmations: 1,
    };
    let initial = 1_000_000_000_u64;
    let fee = 5_000_u64;
    let rent = 2_039_280_u64;
    let mut transaction = json!({"slot":120,"transaction":{"signatures":["signature"],"message":{
        "accountKeys":["wallet","old-account","wsol-account"],"instructions":[]}},"meta":{"err":null,"fee":fee,
        "innerInstructions":[],"preBalances":[initial,0,0],"postBalances":[initial+1_000_000-fee,0,0],
        "preTokenBalances":[],"postTokenBalances":[]}});
    for wrapped_only in [false, true] {
        if wrapped_only {
            transaction["meta"]["preBalances"] = json!([initial, rent, rent]);
            transaction["meta"]["postBalances"] =
                json!([initial + rent - fee, 0, rent + 1_000_000]);
            transaction["meta"]["preTokenBalances"] = json!([
                {"accountIndex":1,"mint":"USDC","owner":"wallet","uiTokenAmount":{"amount":"0"}},
                {"accountIndex":2,"mint":SOLANA_WRAPPED_SOL_MINT,"owner":"wallet","uiTokenAmount":{"amount":"0"}}]);
            transaction["meta"]["postTokenBalances"] = json!([
                {"accountIndex":2,"mint":SOLANA_WRAPPED_SOL_MINT,"owner":"wallet","uiTokenAmount":{"amount":"1000000"}}]);
            transaction["transaction"]["message"]["instructions"] = json!([{
                "programId":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                "parsed":{"type":"closeAccount","info":{"account":"old-account","destination":"wallet"}}}]);
        }
        let fixture = fixture(vec![
            ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            ("getSignatureStatuses", json!({"value":[{"slot":120,"err":null,"confirmations":null,"confirmationStatus":"finalized"}]})),
            ("getTransaction", transaction.clone()),
        ]).await;
        let result = check_on_rpc(&client(), &fixture.url, "solana", &scope, "signature").await;
        if wrapped_only {
            assert!(
                matches!(result, DestinationCreditCheck::Rejected { problem, .. } if problem.contains("WSOL") && problem.contains("1000000"))
            );
        } else {
            assert!(matches!(
                result,
                DestinationCreditCheck::Credited {
                    credited_amount_raw: 1_000_000,
                    ..
                }
            ));
        }
        assert!(fixture.replies.lock().unwrap().is_empty());
    }
}
