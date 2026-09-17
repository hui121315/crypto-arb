use super::{simulation, transaction, Value};
use std::collections::BTreeSet;

mod system;

pub(super) const SYSTEM: &str = "11111111111111111111111111111111";

pub(super) fn system_wallet(account: &Value) -> Result<u64, String> {
    if account["owner"].as_str() != Some(SYSTEM)
        || account["executable"].as_bool() != Some(false)
        || account["data"] != serde_json::json!(["", "base64"])
    {
        return Err("钱包不是已核实的普通 System 账户，不能推导 SOL 周转余额".into());
    }
    account["lamports"]
        .as_u64()
        .ok_or("钱包 SOL 余额缺失".into())
}

#[cfg(test)]
pub(super) fn required(
    result: &Value,
    tx: &transaction::Inspection,
    before: &Value,
    rent_reserve: u64,
    fee: u64,
    debit: u64,
) -> Result<u64, String> {
    bounds(result, tx, before, rent_reserve, fee, debit).map(|(_, required)| required)
}

pub(super) fn bounds(
    result: &Value,
    tx: &transaction::Inspection,
    before: &Value,
    rent_reserve: u64,
    fee: u64,
    debit: u64,
) -> Result<(u64, u64), String> {
    let before_slot = simulation::slot(before, 0)?;
    simulation::slot(result, before_slot)?;
    let initial = system_wallet(&before["value"])?;
    let v = &result["value"];
    let accounts = v["accounts"]
        .as_array()
        .filter(|a| a.len() == 1)
        .ok_or("模拟未返回所选钱包账户，SOL 周转余额未知")?;
    let final_balance = system_wallet(&accounts[0])?;
    if v["preBalances"][tx.wallet_index].as_u64() != Some(initial)
        || v["postBalances"][tx.wallet_index].as_u64() != Some(final_balance)
    {
        return Err("钱包状态在读取与模拟之间变化，需重新核对 SOL 周转余额".into());
    }
    let mut keys = tx.keys.clone();
    for (field, count) in ["writable", "readonly"].into_iter().zip(tx.loaded_counts) {
        let loaded = v["loadedAddresses"][field]
            .as_array()
            .filter(|a| a.len() == count)
            .ok_or("模拟查找表地址不完整")?;
        for key in loaded {
            let key = key.as_str().ok_or("模拟查找表地址无效")?;
            super::stock_inventory::validate_owner(key)?;
            keys.push(key.into());
        }
    }
    if keys.iter().collect::<BTreeSet<_>>().len() != keys.len()
        || v["preBalances"].as_array().map(Vec::len) != Some(keys.len())
        || v["postBalances"].as_array().map(Vec::len) != Some(keys.len())
    {
        return Err("模拟账户存在重复或余额数量不一致".into());
    }
    let wallet = &keys[tx.wallet_index];
    let mut outflow = if tx.fee_payer == *wallet { fee } else { 0 };
    let mut add = |amount: u64| -> Result<(), String> {
        outflow = outflow.checked_add(amount).ok_or("SOL 周转预算溢出")?;
        Ok(())
    };
    for instruction in &tx.instructions {
        if keys[instruction.program] == SYSTEM {
            let accounts: Vec<_> = instruction
                .accounts
                .iter()
                .map(|i| keys[*i].as_str())
                .collect();
            add(system::compiled(&accounts, &instruction.data, wallet)?)?;
        }
    }
    let groups = v["innerInstructions"]
        .as_array()
        .ok_or("RPC 未记录内部指令，SOL 周转余额未知")?;
    let mut seen = BTreeSet::new();
    for group in groups {
        let index = group["index"]
            .as_u64()
            .filter(|i| *i < tx.instructions.len() as u64)
            .ok_or("模拟内部指令索引无效")?;
        if !seen.insert(index) {
            return Err("模拟内部指令分组重复".into());
        }
        for instruction in group["instructions"]
            .as_array()
            .ok_or("模拟内部指令明细缺失")?
        {
            let program = instruction["programId"]
                .as_str()
                .filter(|p| keys.iter().any(|k| k == p))
                .ok_or("内部指令程序不在交易账户中")?;
            if program != SYSTEM {
                continue;
            }
            if instruction.get("parsed").is_some() {
                add(system::parsed(&instruction["parsed"], wallet, &keys)?)?;
            } else {
                let accounts = instruction["accounts"]
                    .as_array()
                    .ok_or("System 内部指令账户缺失")?
                    .iter()
                    .map(|a| {
                        a.as_str()
                            .filter(|a| keys.iter().any(|k| k == a))
                            .ok_or("System 内部指令账户无效".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let data = instruction["data"]
                    .as_str()
                    .filter(|s| s.len() <= 1644)
                    .ok_or("System 内部指令数据缺失")?;
                let data = bs58::decode(data)
                    .into_vec()
                    .map_err(|_| "System 指令编码无效")?;
                add(system::compiled(&accounts, &data, wallet)?)?;
            }
        }
    }
    if outflow < debit {
        return Err("钱包净扣超过已核实指令支出，SOL 周转余额未知".into());
    }
    // Ignore credits/refunds, including failed CPI attempts. This is a conservative
    // funding bound for this simulated path, not a fee or an exact intratx peak.
    if outflow == 0 {
        Ok((0, 0))
    } else {
        outflow
            .checked_add(rent_reserve)
            .map(|required| (outflow, required))
            .ok_or("SOL 免租保留额溢出".into())
    }
}
