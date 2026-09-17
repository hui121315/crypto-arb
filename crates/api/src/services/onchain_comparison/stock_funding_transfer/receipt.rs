use super::*;

pub(crate) async fn read_with(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
) -> Result<StockFundingTransferReceipt, String> {
    mainnet(client, url).await?;
    let t = plan.transfer.as_ref().ok_or("补库转账记录缺失")?;
    let signature = t.transaction_hash.as_deref().ok_or("补库尚未记录提交")?;
    let statuses = rpc(
        client,
        url,
        "getSignatureStatuses",
        json!([[signature],{"searchTransactionHistory":true}]),
    )
    .await?;
    let rows = statuses["value"]
        .as_array()
        .filter(|a| a.len() == 1)
        .ok_or("补库原签名状态不完整")?;
    let status = &rows[0];
    if status.is_null() || status["confirmationStatus"] != "finalized" {
        return Err("原补库转账尚未最终确认；没有重新广播".into());
    }
    let tx=rpc(client,url,"getTransaction",json!([signature,{"encoding":"base64","commitment":"finalized","maxSupportedTransactionVersion":0}])).await?;
    parse(plan, status, &tx, common::time::now_ms())
}

pub(super) fn parse(
    plan: &StockFundingPlan,
    status: &Value,
    tx: &Value,
    now: i64,
) -> Result<StockFundingTransferReceipt, String> {
    let transfer = plan.transfer.as_ref().ok_or("补库转账记录缺失")?;
    let submitted = transfer.submitted_at_ms.ok_or("补库未提交")?;
    let p = &transfer.preparation;
    let signature = transfer
        .transaction_hash
        .as_deref()
        .ok_or("补库原签名未知")?;
    if tx["transaction"][1] != "base64"
        || signed_identity(
            plan,
            p,
            tx["transaction"][0].as_str().ok_or("原转账消息未知")?,
        )? != signature
    {
        return Err("上链消息与已确认的原补库交易不一致".into());
    }
    let slot = tx["slot"]
        .as_u64()
        .filter(|s| *s >= p.slot)
        .ok_or("原补库区块早于构建")?;
    let block_time_ms = tx["blockTime"]
        .as_i64()
        .and_then(|t| t.checked_mul(1000))
        .ok_or("原补库区块时间缺失")?;
    let meta = &tx["meta"];
    let error = meta.get("err").ok_or("原补库终态未知")?;
    if status["confirmationStatus"] != "finalized"
        || status["slot"].as_u64() != Some(slot)
        || status.get("err") != Some(error)
        || block_time_ms < submitted.saturating_sub(5000)
        || block_time_ms > now.saturating_add(5000)
    {
        return Err("原补库终态、签名或时序不一致".into());
    }
    for name in ["readonly", "writable"] {
        if meta["loadedAddresses"][name]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        {
            return Err("补库原交易不使用地址查找表".into());
        }
    }
    let native = plan.request.funding_asset == "SOL";
    let count = if native {
        3
    } else if p.account_creation.is_some() {
        8
    } else {
        5
    };
    let balances = |name: &str| -> Result<Vec<u64>, String> {
        meta[name]
            .as_array()
            .filter(|v| v.len() == count)
            .ok_or("原补库 SOL 收支不完整")?
            .iter()
            .map(|v| v.as_u64().ok_or("原补库 SOL 收支无效".into()))
            .collect()
    };
    let pre = balances("preBalances")?;
    let post = balances("postBalances")?;
    let network_fee_lamports = meta["fee"].as_u64().ok_or("实际网络费未知")?;
    let wallet_debit_lamports = pre[0]
        .checked_sub(post[0])
        .ok_or("来源钱包 SOL 收支方向异常")?;
    let succeeded = error.is_null();
    let mut account_creation_lamports = 0;
    let (source_debit_raw, destination_credit_raw) = if native {
        if pre[2] != post[2]
            || !meta["preTokenBalances"]
                .as_array()
                .is_some_and(Vec::is_empty)
            || !meta["postTokenBalances"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            return Err("原生 SOL 转账出现额外账户收支".into());
        }
        (
            wallet_debit_lamports
                .checked_sub(network_fee_lamports)
                .ok_or("SOL 扣账少于网络费")?,
            post[1]
                .checked_sub(pre[1])
                .ok_or("充值地址 SOL 收支方向异常")?,
        )
    } else {
        if p.account_creation.is_some() {
            account_creation_lamports = post[2]
                .checked_sub(pre[2])
                .ok_or("接收 ATA 的 SOL 收支方向异常")?;
        }
        if pre[1] != post[1]
            || pre[3..] != post[3..]
            || p.account_creation.is_none() && pre[2] != post[2]
            || wallet_debit_lamports
                != network_fee_lamports
                    .checked_add(account_creation_lamports)
                    .ok_or("实际 SOL 收支溢出")?
        {
            return Err("Token 转账出现未计划的 SOL 收支".into());
        }
        let before = tokens(plan, &meta["preTokenBalances"])?;
        let after = tokens(plan, &meta["postTokenBalances"])?;
        // Missing pre-token rows mean zero only when initialization is proven in this transaction.
        let destination_before = match before[1] {
            Some(n) => n,
            None if p.account_creation.is_some()
                && succeeded
                && initialized_destination(meta, &plan.terms.destination) =>
            {
                0
            }
            None if p.account_creation.is_some()
                && !succeeded
                && after[1].is_none()
                && account_creation_lamports == 0 =>
            {
                0
            }
            _ => return Err("接收 Token 原余额未知，缺少原交易的账户创建证据".into()),
        };
        let destination_after = after[1]
            .or_else(|| {
                (!succeeded && before[1].is_none() && p.account_creation.is_some()).then_some(0)
            })
            .ok_or("接收 Token 终态余额未知")?;
        (
            before[0]
                .ok_or("来源 Token 原余额未知")?
                .checked_sub(after[0].ok_or("来源 Token 终态余额未知")?)
                .ok_or("来源 Token 收支方向异常")?,
            destination_after
                .checked_sub(destination_before)
                .ok_or("充值 Token 收支方向异常")?,
        )
    };
    let expected = if succeeded { amount(plan)? } else { 0 };
    let within_plan = source_debit_raw == expected
        && destination_credit_raw == expected
        && network_fee_lamports <= p.network_fee_lamports
        && account_creation_lamports
            <= p.account_creation
                .as_ref()
                .map_or(0, |c| c.rent_budget_lamports)
        && (succeeded || account_creation_lamports == 0)
        && (plan.request.funding_asset != plan.request.security_asset
            || !succeeded
            || plan
                .terms
                .mint
                .next_change_at_ms
                .is_none_or(|at| block_time_ms < at));
    Ok(StockFundingTransferReceipt {
        transaction_hash: signature.into(),
        succeeded,
        within_plan,
        source_debit_raw,
        destination_credit_raw,
        wallet_debit_lamports,
        network_fee_lamports,
        account_creation_lamports,
        slot,
        block_time_ms,
        checked_at_ms: now,
    })
}

fn initialized_destination(meta: &Value, owner: &str) -> bool {
    let Ok(owner) = key(owner) else {
        return false;
    };
    let mut data = vec![18];
    data.extend_from_slice(&owner);
    meta["innerInstructions"].as_array().is_some_and(|groups| {
        groups
            .iter()
            .filter(|g| g["index"] == 0)
            .flat_map(|g| g["instructions"].as_array().into_iter().flatten())
            .filter(|i| {
                i["programIdIndex"] == 4
                    && i["accounts"] == json!([2, 3])
                    && i["stackHeight"].as_u64().is_none_or(|n| n == 2)
                    && i["data"]
                        .as_str()
                        .and_then(|s| bs58::decode(s).into_vec().ok())
                        .as_deref()
                        == Some(data.as_slice())
            })
            .count()
            == 1
    })
}

fn tokens(plan: &StockFundingPlan, rows: &Value) -> Result<[Option<u64>; 2], String> {
    let rows = rows
        .as_array()
        .filter(|r| (1..=2).contains(&r.len()))
        .ok_or("原补库 Token 前后收支缺失")?;
    let mut amounts = [None, None];
    for row in rows {
        let i = row["accountIndex"]
            .as_u64()
            .filter(|i| [1, 2].contains(i))
            .ok_or("原补库 Token 索引不符")? as usize
            - 1;
        let owner = if i == 0 {
            &plan.request.wallet_address
        } else {
            &plan.terms.destination
        };
        if amounts[i].is_some()
            || row["mint"].as_str() != plan.terms.token.contract_address.as_deref()
            || row["owner"].as_str() != Some(owner)
            || row["uiTokenAmount"]["decimals"].as_u64()
                != plan.terms.token.native_decimals.map(u64::from)
        {
            return Err("原补库 Token 收支的所有者、合约、精度或重复行不符".into());
        }
        amounts[i] = Some(
            row["uiTokenAmount"]["amount"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or("原补库 Token 数量未知")?,
        );
    }
    Ok(amounts)
}
