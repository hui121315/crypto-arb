use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SYSTEM: &str = "11111111111111111111111111111111";
const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const WRAPPED_2022: &str = "9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP";

struct TokenBalance<'a> {
    mint: &'a str,
    owner: Option<&'a str>,
    amount: u64,
}

fn wrapped(mint: &str) -> bool {
    matches!(mint, super::SOLANA_WRAPPED_SOL_MINT | WRAPPED_2022)
}

fn account_keys(transaction: &Value) -> Result<Vec<&str>, String> {
    let keys = transaction
        .pointer("/transaction/message/accountKeys")
        .and_then(Value::as_array)
        .ok_or("Solana 账户列表缺失")?
        .iter()
        .map(|key| {
            key.as_str()
                .or_else(|| key["pubkey"].as_str())
                .filter(|key| !key.is_empty())
                .ok_or("Solana 账户地址缺失".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if keys.iter().collect::<BTreeSet<_>>().len() != keys.len() {
        return Err("Solana 账户地址重复".into());
    }
    Ok(keys)
}

fn token_balances<'a>(
    rows: &'a Value,
    key_count: usize,
) -> Result<BTreeMap<usize, TokenBalance<'a>>, String> {
    let mut result = BTreeMap::new();
    for row in rows.as_array().ok_or("Solana 代币余额记录缺失")? {
        let index = row["accountIndex"]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < key_count)
            .ok_or("Solana 代币账户索引无效")?;
        let balance = TokenBalance {
            mint: row["mint"].as_str().ok_or("Solana Mint 身份缺失")?,
            owner: row["owner"].as_str(),
            amount: row
                .pointer("/uiTokenAmount/amount")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or("Solana 原始代币数量无效")?,
        };
        if result.insert(index, balance).is_some() {
            return Err("Solana 代币账户索引重复，不能重复累计".into());
        }
    }
    Ok(result)
}

fn instructions(transaction: &Value) -> Result<Vec<&Value>, String> {
    let outer = transaction
        .pointer("/transaction/message/instructions")
        .and_then(Value::as_array)
        .ok_or("Solana 交易指令缺失")?;
    let mut result = outer.iter().collect::<Vec<_>>();
    if let Some(inner) = transaction
        .pointer("/meta/innerInstructions")
        .and_then(Value::as_array)
    {
        let mut seen = BTreeSet::new();
        for group in inner {
            let index = group["index"]
                .as_u64()
                .filter(|index| *index < outer.len() as u64)
                .ok_or("Solana 内部指令索引无效")?;
            if !seen.insert(index) {
                return Err("Solana 内部指令分组重复".into());
            }
            result.extend(
                group["instructions"]
                    .as_array()
                    .ok_or("Solana 内部指令明细缺失")?,
            );
        }
    } else if outer
        .iter()
        .any(|instruction| instruction["programId"].as_str() != Some(SYSTEM))
    {
        return Err("Solana 内部指令未记录，不能区分兑换到账与账户退款".into());
    }
    Ok(result)
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Solana 指令缺少 {name}"))
}

fn amount_change(instructions: &[&Value], account: &str, mint: &str) -> Result<i128, String> {
    let mut change = 0_i128;
    for instruction in instructions {
        if !matches!(instruction["programId"].as_str(), Some(TOKEN | TOKEN_2022)) {
            continue;
        }
        let parsed = &instruction["parsed"];
        if !matches!(
            parsed["type"].as_str(),
            Some("transfer" | "transferChecked")
        ) {
            continue;
        }
        let info = &parsed["info"];
        let from = field(info, "source")?;
        let to = field(info, "destination")?;
        if from != account && to != account {
            continue;
        }
        if parsed["type"] == "transferChecked" && field(info, "mint")? != mint {
            return Err("WSOL 转账 Mint 与原账户不一致".into());
        }
        let amount = info
            .get("amount")
            .or_else(|| info.pointer("/tokenAmount/amount"))
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or("WSOL 转账原始数量缺失")?;
        if from == account {
            change -= i128::from(amount);
        }
        if to == account {
            change += i128::from(amount);
        }
    }
    Ok(change)
}

pub(super) fn native_credit(transaction: &Value, destination: &str) -> Result<u128, String> {
    Ok(native_delta(transaction, destination, false)?.max(0) as u128)
}

pub(super) fn native_change(transaction: &Value, destination: &str) -> Result<i128, String> {
    native_delta(transaction, destination, true)
}

pub(super) fn recipient_native_change(
    transaction: &Value,
    destination: &str,
) -> Result<i128, String> {
    let keys = account_keys(transaction)?;
    if keys.contains(&destination) {
        return native_change(transaction, destination);
    }
    // A transfer into an existing token account need not include its owner.
    // The owner's SOL account cannot change, but its wrapped SOL accounts can.
    for path in ["/meta/preBalances", "/meta/postBalances"] {
        if transaction
            .pointer(path)
            .and_then(Value::as_array)
            .is_none_or(|rows| {
                rows.len() != keys.len() || rows.iter().any(|v| v.as_u64().is_none())
            })
        {
            return Err("Solana 余额与账户列表不匹配".into());
        }
    }
    let before = token_balances(&transaction["meta"]["preTokenBalances"], keys.len())?;
    let after = token_balances(&transaction["meta"]["postTokenBalances"], keys.len())?;
    for (index, token) in &after {
        if before
            .get(index)
            .is_some_and(|old| old.mint != token.mint || old.owner != token.owner)
        {
            return Err("交易内代币账户身份变化，需单独核验到账".into());
        }
    }
    let total = |rows: &BTreeMap<usize, TokenBalance<'_>>| -> Result<u128, String> {
        let mut total = 0_u128;
        for token in rows.values().filter(|token| wrapped(token.mint)) {
            let owner = token.owner.ok_or("WSOL 余额缺少持有人身份")?;
            if owner == destination {
                total += u128::from(token.amount);
            }
        }
        Ok(total)
    };
    super::signed_change(total(&after)?, total(&before)?)
}

fn native_delta(
    transaction: &Value,
    destination: &str,
    include_wrapped: bool,
) -> Result<i128, String> {
    let keys = account_keys(transaction)?;
    let index = keys
        .iter()
        .position(|key| *key == destination)
        .ok_or("交易未包含目标钱包")?;
    let balances = |path| -> Result<Vec<u64>, String> {
        let rows = transaction
            .pointer(path)
            .and_then(Value::as_array)
            .ok_or("Solana SOL 余额记录缺失")?;
        if rows.len() != keys.len() {
            return Err("Solana 余额与账户列表不匹配".into());
        }
        rows.iter()
            .map(|row| row.as_u64().ok_or("Solana lamports 无效".to_owned()))
            .collect()
    };
    let pre = balances("/meta/preBalances")?;
    let post = balances("/meta/postBalances")?;
    let before = token_balances(&transaction["meta"]["preTokenBalances"], keys.len())?;
    let after = token_balances(&transaction["meta"]["postTokenBalances"], keys.len())?;
    let instructions = instructions(transaction)?;
    let mut creations = BTreeMap::new();
    let mut closures = BTreeMap::new();
    for instruction in &instructions {
        let program = instruction["programId"].as_str();
        let parsed = &instruction["parsed"];
        let info = &parsed["info"];
        if matches!(program, Some(TOKEN | TOKEN_2022)) && !parsed.is_object() {
            return Err("代币指令未解析，无法核对 SOL 解包与退租".into());
        }
        if program == Some(SYSTEM)
            && matches!(
                parsed["type"].as_str(),
                Some("createAccount" | "createAccountWithSeed")
            )
        {
            let account = field(info, "newAccount")?;
            let funding = info["lamports"].as_u64().ok_or("Solana 建户金额缺失")?;
            if creations
                .insert(account, (field(info, "source")?, funding))
                .is_some()
            {
                return Err("同一临时账户被重复创建，需单独核验".into());
            }
        }
        if matches!(program, Some(TOKEN | TOKEN_2022)) && parsed["type"] == "closeAccount" {
            let account = field(info, "account")?;
            if closures
                .insert(account, field(info, "destination")?)
                .is_some()
            {
                return Err("同一账户被重复关闭，需单独核验".into());
            }
        }
    }
    let mut credited = i128::from(post[index]) - i128::from(pre[index]);
    if index == 0 {
        // Gas is accounted for separately in execution economics. Only this
        // wallet's actual network fee is added back to its received quantity.
        credited += i128::from(
            transaction["meta"]["fee"]
                .as_u64()
                .ok_or("Solana 网络费记录缺失")?,
        );
    }
    for index in before.keys() {
        if post[*index] == 0 && !closures.contains_key(keys[*index]) {
            return Err("代币账户已关闭，但缺少可信的关闭指令，不能将退款计入 SOL 到账".into());
        }
    }
    for (account, receiver) in closures {
        if receiver != destination {
            continue;
        }
        let index = keys
            .iter()
            .position(|key| *key == account)
            .ok_or("关闭账户不在交易明细中")?;
        if post[index] != 0 {
            return Err("关闭账户仍有余额，不能确认退租金额".into());
        }
        if pre[index] == 0 {
            // Wallet-funded temporary accounts return the same rent within
            // this transaction. Sponsored rent needs separate attribution.
            if creations.get(account).map(|creation| creation.0) != Some(destination) {
                return Err(
                    "临时账户由其他地址提供租金，需核对 SOL 到账与租金退款，不能合并计入".into(),
                );
            }
            continue;
        }
        let token = before
            .get(&index)
            .ok_or("关闭账户缺少原始代币记录，无法区分本金与退租")?;
        let owner = token.owner.ok_or("关闭账户缺少原持有人身份")?;
        let principal = if wrapped(token.mint) { token.amount } else { 0 };
        let rent = pre[index]
            .checked_sub(principal)
            .ok_or("WSOL 本金超过账户 lamports")?;
        credited -= i128::from(rent);
        if !include_wrapped && wrapped(token.mint) && owner == destination {
            let remaining =
                (i128::from(principal) + amount_change(&instructions, account, token.mint)?).max(0);
            // Unwrapping previously owned WSOL is a relocation of existing
            // funds, not fresh output from this swap or bridge.
            credited -= i128::from(principal).min(remaining);
        }
    }
    for (index, token) in &after {
        if let Some(previous) = before.get(index) {
            if previous.mint != token.mint || previous.owner != token.owner {
                return Err("交易内代币账户身份变化，需单独核验到账".into());
            }
        }
        if token.owner != Some(destination) || pre[*index] != 0 {
            continue;
        }
        if let Some((source, funding)) = creations
            .get(keys[*index])
            .filter(|creation| creation.0 == destination)
        {
            let principal = if wrapped(token.mint) { token.amount } else { 0 };
            let rent = post[*index]
                .checked_sub(principal)
                .ok_or("新建 WSOL 账户金额不一致")?;
            if *funding < rent {
                return Err(format!(
                    "新账户租金由多个来源提供，{source} 的建户金额不足以覆盖租金，不能全部加回"
                ));
            }
            credited += i128::from(rent);
        }
    }
    let wrapped_total = |rows: &BTreeMap<usize, TokenBalance<'_>>| -> u128 {
        rows.values()
            .filter(|token| token.owner == Some(destination) && wrapped(token.mint))
            .map(|token| u128::from(token.amount))
            .sum()
    };
    let wrapped_gain = wrapped_total(&after).saturating_sub(wrapped_total(&before));
    if include_wrapped {
        return credited
            .checked_add(super::signed_change(
                wrapped_total(&after),
                wrapped_total(&before),
            )?)
            .ok_or_else(|| "SOL 与 WSOL 净变化溢出".into());
    }
    if credited <= 0 && wrapped_gain > 0 {
        return Err(format!("交易已确认，WSOL 代币账户增加 {wrapped_gain} 原始单位，但原生 SOL 未到账；需按 WSOL 核对或解包，不能当作原生 SOL 继续执行"));
    }
    Ok(credited)
}

pub(super) fn token_credit(
    transaction: &Value,
    destination: &str,
    mint: &str,
) -> Result<u128, String> {
    Ok(token_change(transaction, destination, mint)?.max(0) as u128)
}

pub(super) fn token_change(
    transaction: &Value,
    destination: &str,
    mint: &str,
) -> Result<i128, String> {
    let keys = account_keys(transaction)?;
    let before = token_balances(&transaction["meta"]["preTokenBalances"], keys.len())?;
    let after = token_balances(&transaction["meta"]["postTokenBalances"], keys.len())?;
    for (index, token) in &after {
        if let Some(previous) = before.get(index) {
            if (previous.mint == mint || token.mint == mint)
                && (previous.mint != token.mint || previous.owner != token.owner)
            {
                return Err("目标代币账户身份在交易内变化，需单独核验到账".into());
            }
        }
    }
    let total = |rows: &BTreeMap<usize, TokenBalance<'_>>| -> Result<u128, String> {
        rows.values()
            .filter(|token| token.mint == mint)
            .try_fold(0_u128, |sum, token| {
                let owner = token
                    .owner
                    .filter(|owner| !owner.is_empty())
                    .ok_or("目标 Mint 的账户缺少持有人，不能将原有余额当作新到账")?;
                if owner != destination {
                    return Ok(sum);
                }
                sum.checked_add(u128::from(token.amount))
                    .ok_or_else(|| "Solana 代币数量溢出".into())
            })
    };
    super::signed_change(total(&after)?, total(&before)?)
}

#[cfg(test)]
mod tests;
