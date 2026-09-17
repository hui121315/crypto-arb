use super::super::rpc_tests::{client, fixture};
use super::*;
use serde_json::json;

fn asset(symbol: &str, address: &str, decimals: u8) -> shared_types::OnchainExecutionToken {
    shared_types::OnchainExecutionToken {
        symbol: symbol.into(),
        address: address.into(),
        decimals,
    }
}

pub(in crate::services::onchain_comparison) fn solana_basis() -> OnchainChainSettlementBasis {
    OnchainChainSettlementBasis {
        chain: "solana".into(),
        wallet: "wallet".into(),
        transaction_id: "signature".into(),
        assets: shared_types::OnchainSwapAssets {
            input: asset("USDT", "mint-a", 6),
            output: asset("USDC", "mint-b", 6),
        },
        maximum_input_raw: "100000000".into(),
        minimum_output_raw: Some("104000000".into()),
    }
}

pub(in crate::services::onchain_comparison) fn solana_tx() -> Value {
    let balance = |index, mint, amount: u64| {
        json!({"accountIndex":index,"owner":"wallet","mint":mint,
        "uiTokenAmount":{"amount":amount.to_string(),"decimals":6}})
    };
    json!({"slot":120,"transaction":{"signatures":["signature"],"message":{
        "accountKeys":["wallet","account-a","account-b"],"instructions":[]}},
        "meta":{"err":null,"fee":5000,"innerInstructions":[],"preBalances":[1000000000,2039280,2039280],
        "postBalances":[999986000,2039280,2039280],
        "preTokenBalances":[balance(1,"mint-a",100000000),balance(2,"mint-b",0)],
        "postTokenBalances":[balance(1,"mint-a",1000000),balance(2,"mint-b",105000000)]}})
}

#[tokio::test]
async fn chain_settlement_solana_uses_net_token_change_and_separate_actual_fee_and_tip() {
    let fixture = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", solana_tx()),
    ])
    .await;
    let row = read_on_rpc(&client(), &fixture.url, &solana_basis())
        .await
        .unwrap();
    assert_eq!(row.status, Status::Complete);
    assert_eq!(row.input_amount_raw.as_deref(), Some("99000000"));
    assert_eq!(row.output_amount_raw.as_deref(), Some("105000000"));
    assert_eq!(row.additional_native_change_raw.as_deref(), Some("-9000"));
    assert_eq!(
        row.network_cost
            .as_ref()
            .unwrap()
            .total_fee_exact
            .as_deref(),
        Some("0.000005")
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
    if let Ok(path) = std::env::var("CROSSLINE_CHAIN_RECEIPT_FIXTURE") {
        std::fs::write(path, serde_json::to_vec(&row).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn chain_settlement_missing_fee_keeps_proven_token_amounts_without_claiming_zero_cost() {
    let mut tx = solana_tx();
    tx["meta"].as_object_mut().unwrap().remove("fee");
    let fixture = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", tx),
    ])
    .await;
    let row = read_on_rpc(&client(), &fixture.url, &solana_basis())
        .await
        .unwrap();
    assert_eq!(row.status, Status::Pending);
    assert_eq!(row.input_amount_raw.as_deref(), Some("99000000"));
    assert!(row.network_cost.unwrap().total_fee_exact.is_none());
}

#[tokio::test]
async fn chain_settlement_rejects_wrong_signature_decimals_or_missing_owner() {
    for case in ["signature", "decimals", "owner"] {
        let mut tx = solana_tx();
        match case {
            "signature" => tx["transaction"]["signatures"][0] = json!("other"),
            "decimals" => {
                tx["meta"]["postTokenBalances"][0]["uiTokenAmount"]["decimals"] = json!(9)
            }
            _ => {
                tx["meta"]["preTokenBalances"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("owner");
            }
        }
        let fixture = fixture(vec![
            ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            ("getTransaction", tx),
        ])
        .await;
        assert!(
            read_on_rpc(&client(), &fixture.url, &solana_basis())
                .await
                .is_err(),
            "{case}"
        );
    }
}

#[tokio::test]
async fn chain_settlement_failed_swap_retains_gas_instead_of_filling_from_the_quote() {
    let mut tx = solana_tx();
    tx["meta"]["err"] = json!({"InstructionError":[0,"Custom"]});
    let fixture = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", tx),
    ])
    .await;
    let row = read_on_rpc(&client(), &fixture.url, &solana_basis())
        .await
        .unwrap();
    assert_eq!(row.status, Status::ReviewRequired);
    assert_eq!(row.input_amount_raw.as_deref(), Some("0"));
    assert_eq!(row.output_amount_raw.as_deref(), Some("0"));
    assert_eq!(
        row.network_cost.unwrap().total_fee_exact.as_deref(),
        Some("0.000005")
    );
}

#[test]
fn chain_settlement_solana_counts_existing_wsol_spend_but_not_unwrap_as_profit() {
    let mut tx = solana_tx();
    tx["meta"]["postBalances"][0] = json!(999995000);
    for phase in ["preTokenBalances", "postTokenBalances"] {
        tx["meta"][phase][0]["mint"] = json!(SOLANA_WRAPPED_SOL_MINT);
    }
    assert_eq!(solana::native_change(&tx, "wallet"), Ok(-99000000));
    tx["meta"]["postBalances"][0] = json!(1098995000);
    assert_eq!(solana::native_change(&tx, "wallet"), Ok(0));
}

#[tokio::test]
async fn chain_settlement_evm_counts_router_refunds_and_transfer_fees_from_canonical_receipt() {
    let wallet = format!("0x{:040x}", 1);
    let router = format!("0x{:040x}", 2);
    let token = format!("0x{:040x}", 3);
    let transfer = |from: u64, to: u64, value: u128, index| {
        json!({"address":token,"logIndex":format!("0x{index:x}"),
        "topics":[ERC20_TRANSFER_TOPIC,format!("0x{from:064x}"),format!("0x{to:064x}")],"data":format!("0x{value:064x}"),"removed":false})
    };
    let tx = json!({"hash":"0xtransaction","blockHash":"0xcanonical","from":wallet,"to":router,"value":"0x64","input":"0x1234"});
    let receipt = json!({"transactionHash":"0xtransaction","blockHash":"0xcanonical","blockNumber":"0x64","from":wallet,
        "status":"0x1","gasUsed":"0x5208","effectiveGasPrice":"0x3b9aca00","logs":[transfer(2,1,110,0),transfer(1,4,5,1)]});
    let trace = json!({"type":"CALL","from":wallet,"to":router,"value":"0x64","calls":[
        {"type":"CALL","from":router,"to":wallet,"value":"0xa"},
        {"type":"DELEGATECALL","from":router,"to":wallet,"value":"0xffff"}]});
    let fixture = fixture(vec![
        ("eth_chainId", json!("0x1")),
        ("eth_getTransactionReceipt", receipt),
        ("eth_getBlockByNumber", json!({"hash":"0xcanonical"})),
        ("eth_blockNumber", json!("0x65")),
        ("eth_getTransactionByHash", tx),
        ("debug_traceTransaction", trace),
    ])
    .await;
    let basis = OnchainChainSettlementBasis {
        chain: "ethereum".into(),
        wallet,
        transaction_id: "0xtransaction".into(),
        assets: shared_types::OnchainSwapAssets {
            input: asset("ETH", EVM_NATIVE_TOKEN_ADDRESS, 18),
            output: asset("USDC", &token, 6),
        },
        maximum_input_raw: "100".into(),
        minimum_output_raw: Some("100".into()),
    };
    let row = read_on_rpc(&client(), &fixture.url, &basis).await.unwrap();
    assert_eq!(row.status, Status::Complete);
    assert_eq!(row.input_amount_raw.as_deref(), Some("90"));
    assert_eq!(row.output_amount_raw.as_deref(), Some("105"));
    assert!(row.additional_native_change_raw.is_none());
    assert_eq!(
        row.network_cost.unwrap().total_fee_exact.as_deref(),
        Some("0.000021")
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
}
