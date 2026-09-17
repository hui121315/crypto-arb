use serde_json::Value;

fn unchanged(account: &str, wallet: &str) -> Result<(), String> {
    if account == wallet {
        Err("交易改变钱包账户用途，不能推导 SOL 周转余额".into())
    } else {
        Ok(())
    }
}

fn u64_at(data: &[u8], offset: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        data.get(offset..offset + 8)
            .ok_or("System 指令数据不完整")?
            .try_into()
            .map_err(|_| "System 数量无效")?,
    ))
}

fn seed_length(data: &[u8], offset: usize) -> Result<usize, String> {
    let n = usize::try_from(u64_at(data, offset)?).map_err(|_| "System seed 过长")?;
    if n > 32
        || data
            .get(offset + 8..offset + 8 + n)
            .is_none_or(|s| std::str::from_utf8(s).is_err())
    {
        return Err("System seed 无效".into());
    }
    Ok(n)
}

pub(super) fn compiled(accounts: &[&str], data: &[u8], wallet: &str) -> Result<u64, String> {
    // SystemInstruction uses a little-endian u32 variant and fixed-width bincode fields.
    // Unsupported variants never become zero-cost evidence.
    let kind = u32::from_le_bytes(
        data.get(..4)
            .ok_or("System 指令类型缺失")?
            .try_into()
            .map_err(|_| "System 指令类型无效")?,
    );
    let account = |i: usize| {
        accounts
            .get(i)
            .copied()
            .ok_or("System 指令账户不完整".to_owned())
    };
    let (source, amount, size) = match kind {
        0 => {
            unchanged(account(1)?, wallet)?;
            (account(0)?, u64_at(data, 4)?, 52)
        }
        2 => {
            account(1)?;
            (account(0)?, u64_at(data, 4)?, 12)
        }
        3 => {
            unchanged(account(1)?, wallet)?;
            let n = seed_length(data, 36)?;
            (account(0)?, u64_at(data, 44 + n)?, 92 + n)
        }
        11 => {
            account(2)?;
            let n = seed_length(data, 12)?;
            (account(0)?, u64_at(data, 4)?, 52 + n)
        }
        13 => {
            unchanged(account(0)?, wallet)?;
            let amount = u64_at(data, 4)?;
            (if amount > 0 { account(1)? } else { "" }, amount, 52)
        }
        1 | 8 | 9 | 10 => {
            unchanged(account(0)?, wallet)?;
            let size = match kind {
                1 => 36,
                8 => 12,
                9 => 84 + seed_length(data, 36)?,
                _ => 76 + seed_length(data, 36)?,
            };
            ("", 0, size)
        }
        _ => return Err("存在未支持的 System 指令，SOL 周转余额保持未知".into()),
    };
    if data.len() != size {
        return Err("System 指令长度不匹配".into());
    }
    Ok(if source == wallet { amount } else { 0 })
}

pub(super) fn parsed(value: &Value, wallet: &str, keys: &[String]) -> Result<u64, String> {
    let info = &value["info"];
    let address = |name: &str| {
        info[name]
            .as_str()
            .filter(|a| keys.iter().any(|k| k == a))
            .ok_or("System 指令地址不在交易账户中".to_owned())
    };
    match value["type"].as_str() {
        Some(
            "createAccount"
            | "createAccountWithSeed"
            | "createAccountAllowPrefund"
            | "transfer"
            | "transferWithSeed",
        ) => {
            let source = address("source")?;
            if value["type"]
                .as_str()
                .is_some_and(|s| s.starts_with("create"))
            {
                unchanged(address("newAccount")?, wallet)?;
            } else {
                address("destination")?;
            }
            let amount = info["lamports"]
                .as_u64()
                .ok_or("System 指令 lamports 缺失")?;
            Ok(if source == wallet { amount } else { 0 })
        }
        Some("assign" | "assignWithSeed" | "allocate" | "allocateWithSeed") => {
            unchanged(address("account")?, wallet)?;
            Ok(0)
        }
        _ => Err("存在未支持的 System 内部指令，SOL 周转余额保持未知".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_funding_decodes_seeded_creation_and_transfer_and_rejects_unknown_variants() {
        for kind in [0_u32, 2, 3, 11, 13] {
            let mut data = kind.to_le_bytes().to_vec();
            if kind == 3 {
                data.extend([7; 32]);
                data.extend(3_u64.to_le_bytes());
                data.extend(b"abc");
            }
            data.extend(123_u64.to_le_bytes());
            match kind {
                0 | 3 | 13 => {
                    data.extend(165_u64.to_le_bytes());
                    data.extend([8; 32]);
                }
                11 => {
                    data.extend(3_u64.to_le_bytes());
                    data.extend(b"abc");
                    data.extend([8; 32]);
                }
                _ => {}
            }
            let accounts = if kind == 13 {
                vec!["new", "wallet"]
            } else {
                vec!["wallet", "new", "destination"]
            };
            assert_eq!(compiled(&accounts, &data, "wallet").unwrap(), 123, "{kind}");
            assert_eq!(compiled(&accounts, &data, "sponsor").unwrap(), 0);
            assert!(compiled(&accounts, &data[..data.len() - 1], "wallet").is_err());
        }
        let mut assign = 1_u32.to_le_bytes().to_vec();
        assign.extend([0; 32]);
        assert!(compiled(&["wallet"], &assign, "wallet").is_err());
        assert_eq!(compiled(&["new"], &assign, "wallet").unwrap(), 0);
        assert!(compiled(&["wallet"], &99_u32.to_le_bytes(), "wallet").is_err());
    }
}
