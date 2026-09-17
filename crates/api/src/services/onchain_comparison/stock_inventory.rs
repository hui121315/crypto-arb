use super::{rpc::rpc_result_with_limit, rpc_target};
use serde_json::{json, Value};
use shared_types::stocks::{comparison::SOLANA_USDC, StockMintEvidence, StockWalletEvidence};
use std::collections::BTreeSet;

const MAINNET: &str = super::rpc::SOLANA_MAINNET_GENESIS_HASH;
const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

pub(crate) fn validate_owner(owner: &str) -> Result<(), String> {
    if owner.len() > 44
        || bs58::decode(owner)
            .into_vec()
            .ok()
            .is_none_or(|b| b.len() != 32)
    {
        return Err("Solana 钱包地址须为有效的 32 字节公钥".into());
    }
    Ok(())
}

pub(crate) async fn read(
    owner: &str,
    mint: &StockMintEvidence,
) -> Result<StockWalletEvidence, String> {
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    read_with(&client, url.as_str(), owner, mint).await
}

async fn read_with(
    client: &reqwest::Client,
    url: &str,
    owner: &str,
    mint: &StockMintEvidence,
) -> Result<StockWalletEvidence, String> {
    validate_owner(owner)?;
    let genesis = rpc_result_with_limit(client, url, "getGenesisHash", json!([]), 1, 4096).await?;
    if genesis.as_str() != Some(MAINNET) {
        return Err("股票钱包 RPC 不是 Solana 主网".into());
    }
    let start = common::time::now_ms();
    let mut evidence = StockWalletEvidence {
        owner: owner.into(),
        mint: mint.address.clone(),
        stock_raw: None,
        usdc_raw: None,
        sol_lamports: None,
        checked_at_ms: start,
        problems: vec![],
    };
    for (id, address, decimals) in [
        (2, mint.address.as_str(), mint.decimals),
        (3, SOLANA_USDC, 6),
    ] {
        let result=rpc_result_with_limit(client,url,"getTokenAccountsByOwner",json!([owner,{"mint":address},{"commitment":"confirmed","encoding":"jsonParsed","minContextSlot":mint.slot}]),id,512*1024).await
            .and_then(|v|token_balance(&v,owner,address,decimals,mint.slot));
        match result {
            Ok(raw) => {
                if id == 2 {
                    evidence.stock_raw = Some(raw);
                } else {
                    evidence.usdc_raw = Some(raw);
                }
            }
            Err(_) => evidence.problems.push(format!(
                "{} 钱包余额未核实，未当作零",
                if id == 2 { "股票" } else { "USDC" }
            )),
        }
    }
    let gas = rpc_result_with_limit(
        client,
        url,
        "getBalance",
        json!([owner,{"commitment":"confirmed","minContextSlot":mint.slot}]),
        4,
        8192,
    )
    .await;
    match gas.and_then(|v| {
        check_slot(&v, mint.slot)?;
        v["value"]
            .as_u64()
            .map(|v| v.to_string())
            .ok_or("SOL 余额数值无效".into())
    }) {
        Ok(raw) => evidence.sol_lamports = Some(raw),
        Err(_) => evidence.problems.push("SOL Gas 余额未核实".into()),
    }
    Ok(evidence)
}

fn check_slot(value: &Value, minimum: u64) -> Result<(), String> {
    if value["context"]["slot"]
        .as_u64()
        .is_none_or(|slot| slot < minimum)
    {
        return Err("钱包 RPC slot 早于已核实的 Mint".into());
    }
    Ok(())
}
fn token_balance(
    value: &Value,
    owner: &str,
    mint: &str,
    decimals: u8,
    minimum_slot: u64,
) -> Result<String, String> {
    check_slot(value, minimum_slot)?;
    let rows = value["value"]
        .as_array()
        .filter(|rows| rows.len() <= 1024)
        .ok_or("钱包代币账户列表无效或过大")?;
    let mut seen = BTreeSet::new();
    let mut total = 0u128;
    for row in rows {
        let pubkey = row["pubkey"].as_str().ok_or("代币账户地址缺失")?;
        validate_owner(pubkey)?;
        if !seen.insert(pubkey) {
            return Err("重复代币账户，不能重复累加余额".into());
        }
        let account = &row["account"];
        let program = account["owner"].as_str().ok_or("代币账户程序缺失")?;
        if ![TOKEN, TOKEN_2022].contains(&program)
            || ([SOLANA_USDC, shared_types::stocks::STOCK_SOLANA_USDT].contains(&mint) && program != TOKEN)
            || account["executable"] != false
            || account["data"]["parsed"]["type"] != "account"
        {
            return Err("不是可用的 SPL 代币账户".into());
        }
        let info = &account["data"]["parsed"]["info"];
        if info["owner"].as_str() != Some(owner)
            || info["mint"].as_str() != Some(mint)
            || info["tokenAmount"]["decimals"].as_u64() != Some(u64::from(decimals))
        {
            return Err("代币账户钱包、合约或精度不一致".into());
        }
        let raw = info["tokenAmount"]["amount"]
            .as_str()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or("代币原始余额无效")?;
        match info["state"].as_str() {
            Some("initialized") => {
                total = total.checked_add(u128::from(raw)).ok_or("代币余额溢出")?
            }
            Some("frozen") => {}
            _ => return Err("代币账户状态未核实".into()),
        }
    }
    Ok(total.to_string())
}

#[cfg(test)]
mod tests;
