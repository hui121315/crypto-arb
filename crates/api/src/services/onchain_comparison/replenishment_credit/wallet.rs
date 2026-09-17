use super::*;
use shared_types::{
    OnchainChainSettlementStatus as Status, OnchainWalletReceipt, OnchainWalletReceiptBasis,
};

pub(in crate::services::onchain_comparison) async fn check(
    state: &AppState,
    basis: &OnchainWalletReceiptBasis,
) -> OnchainWalletReceipt {
    let result = tokio::time::timeout(std::time::Duration::from_secs(12), async {
        let (url, _) = rpc_endpoint(state, &basis.chain).ok_or("缺少已验证的交易核算 RPC")?;
        let (url, client) = rpc_target::rpc_target(&url).await?;
        read_on_rpc(&client, url.as_str(), basis).await
    })
    .await
    .unwrap_or_else(|_| Err("链上收支核验超时，保留记录后重查，不重复提交".into()));
    result.unwrap_or_else(|problem| pending_receipt(basis, problem))
}

pub(in crate::services::onchain_comparison) fn pending_receipt(
    basis: &OnchainWalletReceiptBasis,
    problem: String,
) -> OnchainWalletReceipt {
    OnchainWalletReceipt {
        basis: basis.clone(),
        status: Status::Pending,
        asset_changes_raw: vec![None; basis.assets.len()],
        additional_native_change_raw: None,
        network_cost: None,
        block_ref: None,
        observed_at_ms: None,
        problem: Some(problem),
    }
}

pub(in crate::services::onchain_comparison) async fn read_on_rpc(
    client: &reqwest::Client,
    url: &str,
    basis: &OnchainWalletReceiptBasis,
) -> Result<OnchainWalletReceipt, String> {
    let is_solana = basis.chain.eq_ignore_ascii_case("solana");
    let same = |a: &str, b: &str| {
        if is_solana {
            a == b
        } else {
            a.eq_ignore_ascii_case(b)
        }
    };
    if basis.wallet.trim().is_empty()
        || basis.transaction_id.trim().is_empty()
        || basis.assets.is_empty()
        || basis.assets.len() > 2
        || basis
            .assets
            .iter()
            .any(|asset| asset.address.trim().is_empty() || asset.decimals > 38)
        || basis.assets.iter().enumerate().any(|(index, asset)| {
            basis.assets[..index]
                .iter()
                .any(|previous| same(&asset.address, &previous.address))
        })
    {
        return Err("链上核算的地址、资产或精度无效".into());
    }
    let (method, expected) = if is_solana {
        (
            "getGenesisHash",
            crate::services::onchain_comparison::rpc::SOLANA_MAINNET_GENESIS_HASH.to_owned(),
        )
    } else {
        let id = shared_types::onchain_chain_preset(&basis.chain)
            .and_then(|c| c.chain_id)
            .ok_or("链身份未登记")?;
        ("eth_chainId", format!("0x{id:x}"))
    };
    let chain_id = rpc::rpc_result(client, url, method, serde_json::json!([]), 760).await?;
    if chain_id.as_str() != Some(&expected) {
        return Err("交易核算 RPC 链身份不一致".into());
    }
    let mut row = pending_receipt(basis, "链上实际收支待核算".into());
    let mut native_problem = None;
    let (changes, native_change, native_in_pair, failed) = if is_solana {
        let tx = rpc::rpc_result_with_limit(client, url, "getTransaction", serde_json::json!([
            basis.transaction_id, {"commitment":"finalized", "encoding":"jsonParsed", "maxSupportedTransactionVersion":0}
        ]), 761, SOLANA_TRANSACTION_MAX_BYTES).await?;
        if tx
            .pointer("/transaction/signatures/0")
            .and_then(Value::as_str)
            != Some(&basis.transaction_id)
            || tx["slot"].as_u64().is_none()
            || tx.pointer("/meta/err").is_none()
        {
            return Err("Solana 最终交易明细尚未取得，或签名、slot、执行结果不匹配".into());
        }
        row.block_ref = Some(tx["slot"].to_string());
        if basis.require_sender {
            let keys = tx
                .pointer("/transaction/message/accountKeys")
                .and_then(Value::as_array)
                .ok_or("Solana 账户身份缺失")?;
            let signer = keys.iter().enumerate().any(|(index, key)| {
                let address = key.as_str().or_else(|| key["pubkey"].as_str());
                address == Some(basis.wallet.as_str())
                    && (key["signer"] == true
                        || key.is_string()
                            && (index == 0
                                || tx
                                    .pointer("/transaction/message/header/numRequiredSignatures")
                                    .and_then(Value::as_u64)
                                    .is_some_and(|count| (index as u64) < count)))
            });
            if !signer {
                return Err("Solana 源钱包不是交易签名方".into());
            }
        }
        row.network_cost = network_cost::solana(&tx, &basis.transaction_id);
        let failed = !tx["meta"]["err"].is_null();
        if failed {
            (vec![Some(0); basis.assets.len()], Some(0), false, true)
        } else {
            let native = if basis.require_sender {
                solana::native_change(&tx, &basis.wallet)
            } else {
                solana::recipient_native_change(&tx, &basis.wallet)
            }
            .map_err(|e| native_problem = Some(e))
            .ok();
            let mut native_in_pair = false;
            let change = |asset: &shared_types::OnchainExecutionToken| {
                if asset.address == SOLANA_WRAPPED_SOL_MINT {
                    if asset.decimals != 9 {
                        return Err("SOL 精度与链身份不符".into());
                    }
                    native_in_pair = true;
                    Ok(native)
                } else {
                    verify_solana_decimals(&tx, asset)?;
                    solana::token_change(&tx, &basis.wallet, &asset.address).map(Some)
                }
            };
            let changes = basis
                .assets
                .iter()
                .map(change)
                .collect::<Result<Vec<_>, String>>()?;
            (changes, native, native_in_pair, false)
        }
    } else {
        let tx = rpc::rpc_result_with_limit(
            client,
            url,
            "eth_getTransactionReceipt",
            serde_json::json!([basis.transaction_id]),
            762,
            EVM_EVIDENCE_MAX_BYTES,
        )
        .await?;
        if !tx["transactionHash"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(&basis.transaction_id))
            || (basis.require_sender
                && !tx["from"]
                    .as_str()
                    .is_some_and(|from| from.eq_ignore_ascii_case(&basis.wallet)))
            || !matches!(tx["status"].as_str(), Some("0x0" | "0x1"))
        {
            return Err("EVM 回执尚未取得，或哈希、发送方、执行结果不匹配".into());
        }
        if evm_confirmations(client, url, &tx).await.unwrap_or(0) < 2 {
            return Err("EVM 回执尚未取得两个确认，或原区块已发生重组".into());
        }
        row.block_ref = tx["blockHash"].as_str().map(str::to_owned);
        row.network_cost = network_cost::evm(client, url, &basis.chain, &tx).await;
        let failed = tx["status"] == "0x0";
        if failed {
            (vec![Some(0); basis.assets.len()], Some(0), false, true)
        } else {
            let scope = CreditScope {
                destination: basis.wallet.clone(),
                asset_address: EVM_NATIVE_TOKEN_ADDRESS.into(),
                expected_raw: 1,
                required_confirmations: 2,
            };
            let native = native::change(client, url, &basis.transaction_id, &tx, &scope)
                .await
                .map_err(|error| {
                    native_problem = Some(match error {
                        native::CreditError::Invalid(e) | native::CreditError::Unavailable(e) => e,
                    })
                })
                .ok();
            let mut native_in_pair = false;
            let change = |asset: &shared_types::OnchainExecutionToken| {
                if asset.address.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS)
                    || asset.address == "0x0000000000000000000000000000000000000000"
                {
                    if asset.decimals != 18 {
                        return Err("EVM 原生币精度不符".into());
                    }
                    native_in_pair = true;
                    Ok(native)
                } else {
                    evm_token_change(
                        &tx,
                        &CreditScope {
                            asset_address: asset.address.clone(),
                            destination: basis.wallet.clone(),
                            expected_raw: 1,
                            required_confirmations: 2,
                        },
                    )
                    .map(Some)
                }
            };
            let changes = basis
                .assets
                .iter()
                .map(change)
                .collect::<Result<Vec<_>, String>>()?;
            (changes, native, native_in_pair, false)
        }
    };
    row.observed_at_ms = Some(common::time::now_ms());
    row.asset_changes_raw = changes.iter().map(|v| v.map(|v| v.to_string())).collect();
    row.additional_native_change_raw = native_change
        .filter(|_| !native_in_pair)
        .map(|v| v.to_string());
    if failed {
        row.status = Status::ReviewRequired;
        row.problem = Some("链上交易失败；资产交易未发生，网络费仍需计入成本".into());
    } else if let Some(problem) = native_problem {
        row.problem = Some(format!(
            "已保留可确认的资产数量；原生币、退款或额外费用仍待核算：{problem}"
        ));
    } else if row.asset_changes_raw.iter().any(Option::is_none) {
        row.problem = Some("实际资产变化尚未取得".into());
    } else if row
        .network_cost
        .as_ref()
        .is_none_or(|cost| cost.total_fee_exact.is_none() || cost.problem.is_some())
    {
        row.problem = Some("实际资产数量已取得，完整网络费仍待核算".into());
    } else {
        row.status = Status::Complete;
        row.problem = None;
    }
    Ok(row)
}

fn verify_solana_decimals(
    tx: &Value,
    asset: &shared_types::OnchainExecutionToken,
) -> Result<(), String> {
    let mut found = false;
    for rows in [
        &tx["meta"]["preTokenBalances"],
        &tx["meta"]["postTokenBalances"],
    ] {
        for row in rows.as_array().ok_or("Solana 代币余额缺失")? {
            if row["mint"].as_str() == Some(&asset.address) {
                found = true;
                if row
                    .pointer("/uiTokenAmount/decimals")
                    .and_then(Value::as_u64)
                    != Some(u64::from(asset.decimals))
                {
                    return Err("Solana 实际代币精度与构建时不一致".into());
                }
            }
        }
    }
    if found {
        Ok(())
    } else {
        Err("Solana 交易中没有所选 Mint 的余额证据".into())
    }
}

#[cfg(test)]
mod tests;
