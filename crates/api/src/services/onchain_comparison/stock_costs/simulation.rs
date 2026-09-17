use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn slot(value: &Value, minimum: u64) -> Result<u64, String> {
    value["context"]["slot"]
        .as_u64()
        .filter(|s| *s >= minimum)
        .ok_or("费用 RPC slot 早于报价或 Mint".into())
}

pub(super) fn check(
    result: &Value,
    transaction: &transaction::Inspection,
    cost: &StockChainCost,
    fee: u64,
    minimum_slot: u64,
) -> Result<(u64, u64, u64), String> {
    let slot = slot(result, minimum_slot)?;
    let v = &result["value"];
    if v.get("err") != Some(&Value::Null) {
        return Err("链上交易模拟未通过，没有签名或广播".into());
    }
    if v["fee"].as_u64() != Some(fee) {
        return Err("RPC 未提供一致的模拟网络费，不能完成费用核验".into());
    }
    let pre = balances(&v["preBalances"], transaction.static_accounts)?;
    let post = balances(&v["postBalances"], transaction.static_accounts)?;
    if pre.len() != post.len() {
        return Err("模拟前后 SOL 账户数量不一致".into());
    }
    let debit = pre[transaction.wallet_index]
        .checked_sub(post[transaction.wallet_index])
        .ok_or("股票兑换出现未解释的 SOL 流入，费用不能直接抵扣")?;
    if transaction.fee_payer == cost.wallet_address && debit < fee {
        return Err("钱包 SOL 扣减小于消息网络费，费用结果未闭合".into());
    }
    if cost
        .provider_fees
        .iter()
        .find(|f| f.kind == "signature")
        .and_then(|f| f.payer.as_deref())
        .is_some_and(|p| p != transaction.fee_payer)
    {
        return Err("Provider 网络费付款方与实际交易不一致".into());
    }
    let before = tokens(&v["preTokenBalances"], cost, pre.len())?;
    let after = tokens(&v["postTokenBalances"], cost, post.len())?;
    let input = &cost.quote.input_mint;
    let output = &cost.quote.output_mint;
    let delta = |mint: &str| -> i128 {
        after.get(mint).copied().unwrap_or(0) - before.get(mint).copied().unwrap_or(0)
    };
    let required = cost
        .quote
        .input_raw
        .parse::<u64>()
        .map_err(|_| "输入数量无效")?;
    let minimum = cost
        .quote
        .minimum_output_raw
        .parse::<u64>()
        .map_err(|_| "最低到账无效")?;
    if delta(input) != -i128::from(required) || delta(output) < i128::from(minimum) {
        return Err("模拟钱包实际扣款或最低到账与报价不一致，不能沿用价差".into());
    }
    if before
        .keys()
        .any(|mint| mint != input && mint != output && delta(mint) < 0)
    {
        return Err("交易模拟扣除了报价以外的资产，费用未核清".into());
    }
    Ok((fee, debit, slot))
}

pub(super) fn native_change(
    result: &Value,
    transaction: &transaction::Inspection,
    cost: &StockChainCost,
    fee: u64,
    minimum_slot: u64,
) -> Result<i128, String> {
    slot(result, minimum_slot)?;
    let v = &result["value"];
    if v.get("err") != Some(&Value::Null) || v["fee"].as_u64() != Some(fee) {
        return Err("SOL 补仓模拟失败或网络费未核实".into());
    }
    let pre = balances(&v["preBalances"], transaction.static_accounts)?;
    let post = balances(&v["postBalances"], transaction.static_accounts)?;
    if pre.len() != post.len() {
        return Err("SOL 补仓模拟账户数量不一致".into());
    }
    let change =
        i128::from(post[transaction.wallet_index]) - i128::from(pre[transaction.wallet_index]);
    let before = tokens(&v["preTokenBalances"], cost, pre.len())?;
    let after = tokens(&v["postTokenBalances"], cost, post.len())?;
    let delta =
        |mint: &str| after.get(mint).copied().unwrap_or(0) - before.get(mint).copied().unwrap_or(0);
    let input = cost
        .quote
        .input_raw
        .parse::<u64>()
        .map_err(|_| "SOL 补仓输入数量无效")?;
    if cost.quote.input_mint != comparison::SOLANA_USDC
        || cost.quote.output_mint != STOCK_WRAPPED_SOL
        || delta(comparison::SOLANA_USDC) != -i128::from(input)
        || delta(STOCK_WRAPPED_SOL) != 0
        || before
            .keys()
            .chain(after.keys())
            .any(|mint| mint != comparison::SOLANA_USDC && delta(mint) < 0)
    {
        return Err("SOL 补仓扣款不符、动用了其他资产或只得到 WSOL，不能用于原生 Gas".into());
    }
    Ok(change)
}

fn balances(value: &Value, minimum: usize) -> Result<Vec<u64>, String> {
    value
        .as_array()
        .filter(|a| a.len() >= minimum && a.len() <= 256)
        .ok_or("RPC 未返回完整的模拟前后 SOL 余额")?
        .iter()
        .map(|n| n.as_u64().ok_or("模拟 SOL 余额数值无效".into()))
        .collect()
}

fn tokens(
    value: &Value,
    cost: &StockChainCost,
    accounts: usize,
) -> Result<BTreeMap<String, i128>, String> {
    let rows = value
        .as_array()
        .filter(|a| a.len() <= 256)
        .ok_or("RPC 未返回模拟代币收支")?;
    let mut totals = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        let index = row["accountIndex"]
            .as_u64()
            .filter(|n| *n < accounts as u64)
            .ok_or("代币账户索引无效")?;
        if !seen.insert(index) {
            return Err("重复模拟代币账户，不能累加".into());
        }
        let owner = row["owner"].as_str().ok_or("模拟代币账户所有者未知")?;
        if owner != cost.wallet_address {
            continue;
        }
        let mint = row["mint"].as_str().ok_or("模拟代币合约缺失")?;
        let program = row["programId"].as_str().ok_or("模拟代币程序缺失")?;
        if ![
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
        ]
        .contains(&program)
        {
            return Err("模拟代币程序未知".into());
        }
        let expected = if mint == comparison::SOLANA_USDC {
            Some(6)
        } else if mint == STOCK_WRAPPED_SOL {
            Some(9)
        } else if mint == cost.mint.address {
            Some(u64::from(cost.mint.decimals))
        } else {
            None
        };
        if expected.is_some_and(|d| row["uiTokenAmount"]["decimals"].as_u64() != Some(d)) {
            return Err("模拟代币精度与已核实 Mint 不一致".into());
        }
        let raw = row["uiTokenAmount"]["amount"]
            .as_str()
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or("模拟代币原始余额无效")?;
        *totals.entry(mint.to_owned()).or_insert(0i128) += i128::from(raw);
    }
    Ok(totals)
}
