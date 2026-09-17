use super::*;
use serde_json::json;
use shared_types::stocks::{StockFundingPlan, StockFundingReceipt};

const MAINNET_GENESIS: &str = crate::services::onchain_comparison::rpc::SOLANA_MAINNET_GENESIS_HASH;

pub(crate) async fn read(
    plan: &StockFundingPlan,
    signature: &str,
) -> Result<StockFundingReceipt, String> {
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    read_with(&client, url.as_str(), plan, signature).await
}

pub(crate) async fn read_with(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    signature: &str,
) -> Result<StockFundingReceipt, String> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(12), async {
        if bs58::decode(signature).into_vec().ok().is_none_or(|v| v.len() != 64) { return Err("原提现链上签名无效".into()); }
        let genesis = rpc::rpc_result(client, url, "getGenesisHash", json!([]), 1).await?;
        if genesis.as_str() != Some(MAINNET_GENESIS) { return Err("补库 RPC 不是 Solana 主网".into()); }
        let status = rpc::rpc_result(client, url, "getSignatureStatuses", json!([[signature],{"searchTransactionHistory":true}]), 2).await?;
        let rows = status["value"].as_array().filter(|r|r.len()==1).ok_or("原提现签名状态响应不完整")?;
        let status = &rows[0];
        if status.is_null() || status["confirmationStatus"] != "finalized" { return Err("原提现尚未达到链上最终确认".into()); }
        let transaction = rpc::rpc_result_with_limit(client, url, "getTransaction",
            json!([signature,{"encoding":"jsonParsed","commitment":"finalized","maxSupportedTransactionVersion":0}]), 3, SOLANA_TRANSACTION_MAX_BYTES).await?;
        parse(plan, signature, status, &transaction, common::time::now_ms())
    }).await;
    result.map_err(|_| "链上到账核验超时，没有重新提现".to_owned())?
}

fn parse(
    plan: &StockFundingPlan,
    signature: &str,
    status: &Value,
    tx: &Value,
    now: i64,
) -> Result<StockFundingReceipt, String> {
    let submitted = plan
        .withdrawal
        .as_ref()
        .ok_or("提现提交记录缺失")?
        .submitted_at_ms;
    let slot = tx["slot"]
        .as_u64()
        .filter(|s| *s > 0 && *s >= plan.terms.mint.slot)
        .ok_or("原提现区块未知或早于计划")?;
    let block_time_ms = tx["blockTime"]
        .as_i64()
        .and_then(|v| v.checked_mul(1000))
        .ok_or("原提现链上时间缺失")?;
    if status["confirmationStatus"] != "finalized"
        || status["slot"].as_u64() != Some(slot)
        || status.get("err") != Some(&Value::Null)
        || tx["meta"].get("err") != Some(&Value::Null)
        || tx
            .pointer("/transaction/signatures/0")
            .and_then(Value::as_str)
            != Some(signature)
        || block_time_ms < submitted.saturating_sub(5_000)
        || block_time_ms > now.saturating_add(5_000)
    {
        return Err("原提现链上身份、时序或成功终态未核实".into());
    }
    let mint = plan
        .terms
        .token
        .contract_address
        .as_deref()
        .ok_or("补库合约未知")?;
    let decimals = plan.terms.token.native_decimals.ok_or("补库精度未知")?;
    let key = tx
        .pointer("/transaction/message/accountKeys/0")
        .ok_or("链上付费方未知")?;
    let payer = key
        .as_str()
        .or_else(|| key["pubkey"].as_str())
        .ok_or("链上付费方未知")?;
    let fee = tx["meta"]["fee"].as_u64().ok_or("链上网络费未知")?;
    for side in ["preTokenBalances", "postTokenBalances"] {
        for row in tx["meta"][side].as_array().ok_or("代币余额明细缺失")? {
            if row["mint"].as_str() == Some(mint)
                && row["uiTokenAmount"]["decimals"].as_u64() != Some(u64::from(decimals))
            {
                return Err("原提现合约精度与计划不符".into());
            }
        }
    }
    let destination = &plan.terms.destination;
    let amount = if mint == "So1" {
        // Generic transfer accounting adds back the recipient's Gas; funding inventory needs net credit.
        let mut amount = solana::native_credit(tx, destination)?;
        if payer == destination {
            amount = amount
                .checked_sub(u128::from(fee))
                .ok_or("SOL 到账不足以覆盖付费")?;
        }
        amount
    } else {
        solana::token_credit(tx, destination, mint)?
    };
    let amount = u64::try_from(amount)
        .ok()
        .filter(|v| *v > 0)
        .ok_or("原提现没有该钱包该资产的正向到账")?;
    Ok(StockFundingReceipt {
        transaction_hash: signature.into(),
        destination: destination.clone(),
        mint: mint.into(),
        decimals,
        credited_raw: amount.to_string(),
        slot,
        block_time_ms,
        network_fee_lamports: fee,
        fee_payer: payer.into(),
        checked_at_ms: now,
    })
}
