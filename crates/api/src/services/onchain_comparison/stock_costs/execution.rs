use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use shared_types::OnchainUnsignedTransaction;

mod receipt;

pub(crate) fn validate_artifact(cost: &StockChainCost) -> Result<(), String> {
    let Some(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64,
        request_id,
        router,
        mode,
        expire_at_ms,
        last_valid_block_height,
    }) = &cost.transaction
    else {
        return Err("旧试算未保存原始交易，请重新试算".into());
    };
    if request_id.is_empty()
        || request_id.len() > 256
        || router != &cost.quote.router
        || !["ultra", "manual"].contains(&mode.as_str())
        || *expire_at_ms != cost.quote.expires_at_ms
        || *last_valid_block_height == Some(0)
        || transaction::inspect(transaction_base64, &cost.wallet_address)?.fingerprint
            != cost.transaction_fingerprint
    {
        return Err("原始链上交易与费用试算不一致".into());
    }
    Ok(())
}

// Compare the exact message and preserve every already-populated provider signature.
// JupiterZ may add a previously empty MM/fee-payer signature during execute.
fn signed_identity(
    cost: &StockChainCost,
    encoded: &str,
) -> Result<(String, Option<String>), String> {
    validate_artifact(cost)?;
    let Some(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64, ..
    }) = &cost.transaction
    else {
        unreachable!()
    };
    let inspected = transaction::inspect(transaction_base64, &cost.wallet_address)?;
    if encoded.len() > 1644 {
        return Err("已签名交易长度无效".into());
    }
    let original = STANDARD
        .decode(transaction_base64)
        .map_err(|_| "原交易编码无效")?;
    let signed = STANDARD.decode(encoded).map_err(|_| "签名交易编码无效")?;
    let (count, prefix) = crate::services::onchain_signer::decode_short_vec(&original, 0)?;
    let offset = prefix + count * 64;
    if original.len() != signed.len()
        || original[..prefix] != signed[..prefix]
        || original[offset..] != signed[offset..]
    {
        return Err("签名改变了原计划消息或区块有效期".into());
    }
    for i in 0..count {
        let range = prefix + i * 64..prefix + (i + 1) * 64;
        if original[range.clone()].iter().any(|b| *b != 0)
            && original[range.clone()] != signed[range]
        {
            return Err("签名改变了已有 Provider 签名".into());
        }
    }
    let wallet =
        &signed[prefix + inspected.wallet_index * 64..prefix + (inspected.wallet_index + 1) * 64];
    if wallet.iter().all(|b| *b == 0) {
        return Err("没有所选钱包的签名".into());
    }
    let first = &signed[prefix..prefix + 64];
    Ok((
        bs58::encode(wallet).into_string(),
        first
            .iter()
            .any(|b| *b != 0)
            .then(|| bs58::encode(first).into_string()),
    ))
}

pub(crate) fn valid_signature(s: &str) -> bool {
    s.len() <= 88
        && bs58::decode(s)
            .into_vec()
            .is_ok_and(|v| v.len() == 64 && v.iter().any(|b| *b != 0))
}

pub(crate) fn intent(
    cost: &StockChainCost,
    signed: &str,
    now: i64,
) -> Result<StockChainSubmission, String> {
    let (wallet_signature, transaction_id) = signed_identity(cost, signed)?;
    Ok(StockChainSubmission {
        submitted_at_ms: now,
        wallet_signature,
        transaction_id,
        provider_transaction_id: None,
        provider_acknowledged: false,
        receipt: None,
        recheck_attempts: 0,
        next_recheck_at_ms: now,
        search_before: None,
        problem: None,
    })
}

pub(crate) fn validate_record(
    cost: &StockChainCost,
    row: &StockChainSubmission,
) -> Result<(), String> {
    validate_artifact(cost)?;
    if !valid_signature(&row.wallet_signature)
        || [
            &row.transaction_id,
            &row.provider_transaction_id,
            &row.search_before,
        ]
        .into_iter()
        .flatten()
        .any(|s| !valid_signature(s))
        || row.next_recheck_at_ms < row.submitted_at_ms
        || row.problem.as_ref().is_some_and(|p| p.len() > 2048)
    {
        return Err("链上提交记录的身份或核对状态无效".into());
    }
    if let Some(r) = &row.receipt {
        let Some(OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64, ..
        }) = &cost.transaction
        else {
            unreachable!()
        };
        let tx = transaction::inspect(transaction_base64, &cost.wallet_address)?;
        if row.transaction_id.as_ref() != Some(&r.transaction_id)
            || r.fee_payer != tx.fee_payer
            || r.slot < cost.simulation_slot.unwrap_or(cost.mint.slot)
            || r.network_fee_lamports.parse::<u64>().is_err()
            || r.wallet_native_change_lamports.parse::<i128>().is_err()
            || r.asset_changes.len() > 256
            || r.problems.len() > 8
            || r.within_plan && (!r.succeeded || !r.problems.is_empty())
            || r.asset_changes
                .iter()
                .any(|a| a.raw_change.parse::<i128>().is_err())
        {
            return Err("链上回执未绑定原交易或费用无效".into());
        }
    }
    Ok(())
}

pub(crate) fn transition(
    old: Option<&StockChainSubmission>,
    next: Option<&StockChainSubmission>,
) -> bool {
    match (old, next) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => {
            a.submitted_at_ms == b.submitted_at_ms
                && a.wallet_signature == b.wallet_signature
                && a.transaction_id
                    .as_ref()
                    .is_none_or(|id| b.transaction_id.as_ref() == Some(id))
                && a.provider_transaction_id
                    .as_ref()
                    .is_none_or(|id| b.provider_transaction_id.as_ref() == Some(id))
                && (!a.provider_acknowledged || b.provider_acknowledged)
                && a.receipt
                    .as_ref()
                    .is_none_or(|r| b.receipt.as_ref() == Some(r))
                && b.recheck_attempts >= a.recheck_attempts
                && b.next_recheck_at_ms >= a.next_recheck_at_ms
        }
    }
}

pub(crate) async fn submit(cost: &StockChainCost, signed: &str) -> Result<Option<String>, String> {
    let key = super::super::provider_runtime::env_key("JUPITER_API_KEY");
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, String>> =
        std::sync::OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(8))
                .build()
                .map_err(|_| "股票提交连接初始化失败".to_owned())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    submit_with(
        client,
        "https://api.jup.ag/swap/v2/execute",
        key.as_deref(),
        cost,
        signed,
    )
    .await
}

pub(crate) async fn submit_with(
    client: &reqwest::Client,
    url: &str,
    key: Option<&str>,
    cost: &StockChainCost,
    signed: &str,
) -> Result<Option<String>, String> {
    signed_identity(cost, signed)?;
    let Some(OnchainUnsignedTransaction::SolanaVersioned {
        request_id,
        last_valid_block_height,
        ..
    }) = &cost.transaction
    else {
        unreachable!()
    };
    let mut body = json!({"signedTransaction":signed,"requestId":request_id});
    if let Some(height) = last_valid_block_height {
        body["lastValidBlockHeight"] = height.to_string().into();
    }
    let mut request = client.post(url).timeout(Duration::from_secs(8)).json(&body);
    if let Some(key) = key {
        request = request.header("x-api-key", key);
    }
    // One transport call only. Every error remains recoverable against the original intent.
    let mut response = request
        .send()
        .await
        .map_err(|_| "链上提交回复未收到，只核对原交易，不重发")?;
    if !response.status().is_success() {
        return Err("Provider 未确认提交，只核对原交易，不重发".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "链上提交回复不完整，只核对原交易")?
    {
        if bytes.len() + chunk.len() > 32 * 1024 {
            return Err("链上提交回复超过上限，只核对原交易".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "链上提交回复格式无效，只核对原交易")?;
    // Provider status/amounts are not a finalized receipt or realized profit.
    Ok(value["signature"]
        .as_str()
        .filter(|s| valid_signature(s))
        .map(str::to_owned))
}

pub(crate) async fn recheck_original(cost: &StockChainCost) -> Result<(), String> {
    if super::super::provider_runtime::env_key("JUPITER_API_KEY").is_none() {
        return Err("实盘兑换需配置 Jupiter API Key；未签名或提交".into());
    }
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    recheck_original_with(&client, url.as_str(), cost).await
}

pub(crate) async fn recheck_original_with(
    client: &reqwest::Client,
    url: &str,
    cost: &StockChainCost,
) -> Result<(), String> {
    validate_artifact(cost)?;
    let now = common::time::now_ms();
    if now < cost.checked_at_ms || now >= cost.valid_until_ms {
        return Err("原兑换报价已过期，没有签名或发送".into());
    }
    let Some(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64, ..
    }) = &cost.transaction
    else {
        unreachable!()
    };
    let tx = transaction::inspect(transaction_base64, &cost.wallet_address)?;
    // Re-simulate the saved message, never replace its blockhash, quote or destination.
    let sample = super::simulate_rpc(client, url, transaction_base64, &tx, cost).await?;
    let minimum_slot = sample
        .minimum_slot
        .max(cost.simulation_slot.unwrap_or(cost.mint.slot));
    let native_topup = cost.quote.input_mint == shared_types::stocks::comparison::SOLANA_USDC
        && cost.quote.output_mint == shared_types::stocks::STOCK_WRAPPED_SOL;
    let (fee, debit, _) = if native_topup {
        let change =
            super::simulation::native_change(&sample.result, &tx, cost, sample.fee, minimum_slot)?;
        let (outflow, _) = sample.funding(&tx, 0)?;
        let minimum = cost
            .quote
            .minimum_output_raw
            .parse::<u64>()
            .map_err(|_| "SOL 最低到账未知")?;
        let budget = cost
            .wallet_debit_lamports
            .as_deref()
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or("SOL 补回费用未知")?;
        let net_minimum = minimum
            .checked_sub(budget)
            .filter(|n| *n > 0)
            .ok_or("SOL 补回净到账无效")?;
        if change < i128::from(net_minimum) {
            return Err("原补回交易扣费后 SOL 到账不足，未发送".into());
        }
        (sample.fee, outflow, minimum_slot)
    } else {
        super::simulation::check(&sample.result, &tx, cost, sample.fee, minimum_slot)?
    };
    let (_, required) = sample.funding(&tx, debit)?;
    let available =
        super::funding::system_wallet(&sample.wallet.as_ref().map_err(Clone::clone)?["value"])?;
    let original_required = cost
        .wallet_required_lamports
        .as_deref()
        .and_then(|n| n.parse::<u64>().ok())
        .ok_or("原交易周转预算未知")?;
    let total = if native_topup {
        original_required
    } else {
        cost.total_native_required_lamports(common::time::now_ms())
            .ok_or("SOL 补回预算过期或未知")?
    };
    if common::time::now_ms() >= cost.valid_until_ms
        || cost
            .network_fee_lamports
            .as_deref()
            .and_then(|n| n.parse::<u64>().ok())
            .is_none_or(|n| fee > n)
        || cost
            .wallet_debit_lamports
            .as_deref()
            .and_then(|n| n.parse::<u64>().ok())
            .is_none_or(|n| debit > n)
        || required > original_required
        || available < total
    {
        return Err("原兑换已过期、费用增加或 SOL 备款不足，请重新试算；未发送".into());
    }
    Ok(())
}

pub(crate) struct Lookup {
    pub receipt: Option<StockChainReceipt>,
    pub before: Option<String>,
}

pub(crate) async fn lookup(
    cost: &StockChainCost,
    row: &StockChainSubmission,
) -> Result<Lookup, String> {
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    lookup_with(&client, url.as_str(), cost, row).await
}

pub(crate) async fn lookup_with(
    client: &reqwest::Client,
    url: &str,
    cost: &StockChainCost,
    row: &StockChainSubmission,
) -> Result<Lookup, String> {
    tokio::time::timeout(Duration::from_secs(14), async {
        validate_record(cost, row)?;
        let genesis = rpc_result_with_limit(client, url, "getGenesisHash", json!([]), 1, 4096)
            .await
            .map_err(|_| "股票回执 RPC 网络读取失败")?;
        if genesis.as_str()
            != Some(crate::services::onchain_comparison::rpc::SOLANA_MAINNET_GENESIS_HASH)
        {
            return Err("股票回执 RPC 不是 Solana 主网".into());
        }
        if let Some(id) = &row.transaction_id {
            return Ok(Lookup {
                receipt: read_receipt(client, url, cost, row, id).await?,
                before: None,
            });
        }
        if let Some(id) = &row.provider_transaction_id {
            // A provider hint must match the original wallet signature and message.
            if let Some(receipt) = read_receipt(client, url, cost, row, id).await? {
                return Ok(Lookup {
                    receipt: Some(receipt),
                    before: None,
                });
            }
        }
        let mut config = json!({"commitment":"finalized","limit":8});
        if let Some(before) = &row.search_before {
            config["before"] = before.clone().into();
        }
        let value = rpc_result_with_limit(
            client,
            url,
            "getSignaturesForAddress",
            json!([cost.wallet_address, config]),
            2,
            32768,
        )
        .await
        .map_err(|_| "股票原交易索引读取失败")?;
        let candidates = value
            .as_array()
            .filter(|a| a.len() <= 8)
            .ok_or("股票交易索引格式无效")?;
        let mut before = None;
        for candidate in candidates {
            let id = candidate["signature"]
                .as_str()
                .filter(|s| valid_signature(s))
                .ok_or("股票交易索引缺少签名")?;
            let slot = candidate["slot"].as_u64().ok_or("股票交易索引缺少 slot")?;
            if slot < cost.simulation_slot.unwrap_or(cost.mint.slot) {
                return Ok(Lookup {
                    receipt: None,
                    before: None,
                });
            }
            if let Some(receipt) = read_receipt(client, url, cost, row, id).await? {
                return Ok(Lookup {
                    receipt: Some(receipt),
                    before: None,
                });
            }
            before = Some(id.to_owned());
        }
        Ok(Lookup {
            receipt: None,
            before: if candidates.len() == 8 { before } else { None },
        })
    })
    .await
    .map_err(|_| "股票回执查询超时，保留原计划，下次继续核对")?
}

async fn read_receipt(
    client: &reqwest::Client,
    url: &str,
    cost: &StockChainCost,
    row: &StockChainSubmission,
    id: &str,
) -> Result<Option<StockChainReceipt>, String> {
    let value = rpc_result_with_limit(client, url, "getTransaction", json!([id, {"encoding":"base64","commitment":"finalized","maxSupportedTransactionVersion":0}]), 3, 1024 * 1024).await
        .map_err(|_| "股票原交易回执读取失败")?;
    if value.is_null() {
        return Ok(None);
    }
    receipt::read(cost, row, id, &value)
}

#[cfg(test)]
pub(crate) mod tests;
