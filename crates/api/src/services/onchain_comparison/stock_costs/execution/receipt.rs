use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn read(
    cost: &StockChainCost,
    row: &StockChainSubmission,
    id: &str,
    value: &Value,
) -> Result<Option<StockChainReceipt>, String> {
    if value["transaction"][1] != "base64" {
        return Err("股票回执未返回原始交易".into());
    }
    let encoded = value["transaction"][0].as_str().ok_or("股票回执交易缺失")?;
    let Ok((signature, first)) = signed_identity(cost, encoded) else {
        return Ok(None);
    };
    if signature != row.wallet_signature || first.as_deref() != Some(id) {
        return Ok(None);
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| "回执交易编码无效")?;
    let (count, prefix) = crate::services::onchain_signer::decode_short_vec(&bytes, 0)?;
    if bytes[prefix..prefix + count * 64]
        .chunks_exact(64)
        .any(|s| s.iter().all(|b| *b == 0))
    {
        return Err("已上链交易仍缺签名".into());
    }
    let Some(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64, ..
    }) = &cost.transaction
    else {
        unreachable!()
    };
    let tx = transaction::inspect(transaction_base64, &cost.wallet_address)?;
    let slot = value["slot"]
        .as_u64()
        .filter(|s| *s >= cost.simulation_slot.unwrap_or(cost.mint.slot))
        .ok_or("回执 slot 早于原交易试算")?;
    let meta = &value["meta"];
    let err = meta.get("err").ok_or("回执未提供交易结果")?;
    let fee = meta["fee"].as_u64().ok_or("回执网络费未知")?;
    let mut keys = tx.keys.clone();
    for (i, name) in ["writable", "readonly"].iter().enumerate() {
        let loaded = meta["loadedAddresses"][name].as_array();
        if loaded.map_or(0, Vec::len) != tx.loaded_counts[i] {
            return Err("回执查找表与原消息不一致".into());
        }
        for key in loaded.into_iter().flatten() {
            let key = key.as_str().ok_or("回执查找表地址无效")?;
            stock_inventory::validate_owner(key)?;
            keys.push(key.into());
        }
    }
    if keys.iter().collect::<BTreeSet<_>>().len() != keys.len() {
        return Err("回执账户地址重复".into());
    }
    let balances = |name: &str| -> Result<Vec<u64>, String> {
        meta[name]
            .as_array()
            .filter(|a| a.len() == keys.len())
            .ok_or("回执 SOL 账户余额不完整")?
            .iter()
            .map(|n| n.as_u64().ok_or("回执 SOL 余额无效".into()))
            .collect()
    };
    let pre = balances("preBalances")?;
    let post = balances("postBalances")?;
    let before = tokens(&meta["preTokenBalances"], cost, keys.len())?;
    let after = tokens(&meta["postTokenBalances"], cost, keys.len())?;
    for (mint, (decimals, _)) in &before {
        if after.get(mint).is_some_and(|(d, _)| d != decimals) {
            return Err("回执前后代币精度冲突".into());
        }
    }
    let mut assets = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    assets.insert(cost.quote.input_mint.clone());
    assets.insert(cost.quote.output_mint.clone());
    let changes: Vec<_> = assets
        .into_iter()
        .map(|mint| {
            let decimals = before
                .get(&mint)
                .or_else(|| after.get(&mint))
                .map(|(d, _)| *d)
                .unwrap_or(if mint == comparison::SOLANA_USDC {
                    6
                } else if mint == STOCK_WRAPPED_SOL {
                    9
                } else {
                    cost.mint.decimals
                });
            let raw =
                after.get(&mint).map_or(0, |(_, n)| *n) - before.get(&mint).map_or(0, |(_, n)| *n);
            StockChainAssetChange {
                mint,
                decimals,
                raw_change: raw.to_string(),
            }
        })
        .collect();
    let delta = |mint: &str| {
        changes
            .iter()
            .find(|a| a.mint == mint)
            .and_then(|a| a.raw_change.parse::<i128>().ok())
            .unwrap_or(0)
    };
    let native = i128::from(post[tx.wallet_index]) - i128::from(pre[tx.wallet_index]);
    let mut problems = Vec::new();
    let succeeded = err.is_null();
    if !succeeded {
        problems.push("链上交易已失败，网络费仍按回执保留".into());
        if before != after
            || pre.iter().zip(&post).enumerate().any(|(i, (a, b))| {
                if i == 0 {
                    a.checked_sub(fee) != Some(*b)
                } else {
                    a != b
                }
            })
        {
            problems.push("失败交易的余额变化不符合回滚结果，需核查".into());
        }
    } else {
        let input = cost
            .quote
            .input_raw
            .parse::<u64>()
            .map_err(|_| "计划输入数量无效")?;
        let output = cost
            .quote
            .minimum_output_raw
            .parse::<u64>()
            .map_err(|_| "计划最低到账无效")?;
        let native_output = cost.quote.input_mint == comparison::SOLANA_USDC
            && cost.quote.output_mint == STOCK_WRAPPED_SOL;
        let output_matches = if native_output {
            cost.wallet_budget_lamports
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|outflow| native >= i128::from(output) - i128::from(outflow))
                && delta(STOCK_WRAPPED_SOL) == 0
        } else {
            delta(&cost.quote.output_mint) >= i128::from(output)
        };
        if delta(&cost.quote.input_mint) != -i128::from(input)
            || !output_matches
            || changes.iter().any(|a| {
                a.mint != cost.quote.input_mint
                    && a.mint != cost.quote.output_mint
                    && a.raw_change.starts_with('-')
            })
        {
            problems.push("实际代币收支不符合原计划，不能沿用预估利润".into());
        }
        if cost
            .wallet_debit_lamports
            .as_deref()
            .and_then(|n| n.parse::<u64>().ok())
            .is_none_or(|budget| native < -i128::from(budget))
        {
            problems.push("实际 SOL 支出超过试算预算，需重算净收益".into());
        }
    }
    Ok(Some(StockChainReceipt {
        transaction_id: id.into(),
        slot,
        succeeded,
        fee_payer: tx.fee_payer,
        network_fee_lamports: fee.to_string(),
        wallet_native_change_lamports: native.to_string(),
        asset_changes: changes,
        within_plan: succeeded && problems.is_empty(),
        problems,
    }))
}

fn tokens(
    value: &Value,
    cost: &StockChainCost,
    count: usize,
) -> Result<BTreeMap<String, (u8, i128)>, String> {
    let mut totals = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in value
        .as_array()
        .filter(|a| a.len() <= 256)
        .ok_or("回执代币余额缺失")?
    {
        let index = row["accountIndex"]
            .as_u64()
            .filter(|n| *n < count as u64)
            .ok_or("回执代币索引无效")?;
        if !seen.insert(index) {
            return Err("回执代币账户重复".into());
        }
        let owner = row["owner"].as_str().ok_or("回执代币所有者未知")?;
        if owner != cost.wallet_address {
            continue;
        }
        let mint = row["mint"].as_str().ok_or("回执代币合约未知")?;
        stock_inventory::validate_owner(mint)?;
        let decimals = row["uiTokenAmount"]["decimals"]
            .as_u64()
            .filter(|d| *d <= 38)
            .ok_or("回执精度未知")? as u8;
        if (mint == comparison::SOLANA_USDC && decimals != 6)
            || (mint == STOCK_WRAPPED_SOL && decimals != 9)
            || (mint == cost.mint.address && decimals != cost.mint.decimals)
            || ![
                "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
            ]
            .contains(&row["programId"].as_str().unwrap_or(""))
        {
            return Err("回执代币程序或精度不符".into());
        }
        let raw = row["uiTokenAmount"]["amount"]
            .as_str()
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or("回执代币原始余额无效")?;
        let entry = totals.entry(mint.into()).or_insert((decimals, 0i128));
        if entry.0 != decimals {
            return Err("同一代币出现不同精度".into());
        }
        entry.1 += i128::from(raw);
    }
    Ok(totals)
}
