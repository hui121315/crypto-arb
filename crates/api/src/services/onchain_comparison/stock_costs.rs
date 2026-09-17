//! Unsigned Jupiter builds and read-only RPC simulation. No signer or submission path.
use super::{
    jupiter_quota, quote, rpc::rpc_result_with_limit, rpc_target, stock_inventory, stock_quotes,
};
use serde_json::{json, Value};
use shared_types::stocks::*;
use std::time::Duration;

pub(crate) mod execution;
mod funding;
mod simulation;
mod transaction;
mod valuation;

pub(crate) async fn read_native_replacement(
    context: &StockChainCost,
    native: u64,
) -> Result<StockNativeValuation, String> {
    let key = super::provider_runtime::env_key("JUPITER_API_KEY");
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    read_native_replacement_with(
        quote::quote_client(),
        quote::JUPITER_ORDER_ENDPOINT,
        key.as_deref(),
        &client,
        url.as_str(),
        context,
        native,
    )
    .await
}

async fn read_native_replacement_with(
    http: &reqwest::Client,
    order_url: &str,
    key: Option<&str>,
    rpc: &reqwest::Client,
    rpc_url: &str,
    context: &StockChainCost,
    native: u64,
) -> Result<StockNativeValuation, String> {
    let mut fresh = context.clone();
    // This is a new post-trade quote, not an extension of the expired stock transaction.
    fresh.valid_until_ms = common::time::now_ms().saturating_add(20_000);
    valuation::read_complete_at(
        http,
        order_url,
        key,
        rpc,
        rpc_url,
        &fresh,
        &native.to_string(),
    )
    .await
}

pub(crate) async fn read(
    request: &StockChainCostRequest,
    comparison: &StockComparison,
) -> Result<StockChainCost, String> {
    let seed = request.direction.quote(comparison).ok_or("请先取得这个方向的链上报价")?;
    read_token_swap(request, comparison.keyed, &comparison.mint, seed).await
}

pub(crate) async fn read_token_swap(
    request: &StockChainCostRequest,
    keyed: bool,
    mint: &StockMintEvidence,
    seed: &StockDexQuote,
) -> Result<StockChainCost, String> {
    stock_inventory::validate_owner(&request.wallet_address)?;
    let key = if keyed {
        Some(
            super::provider_runtime::env_key("JUPITER_API_KEY")
                .ok_or("请先配置 Jupiter API Key")?,
        )
    } else {
        None
    };
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    jupiter_quota::wait_for_general_request(key.is_some()).await;
    read_seed_with(
        quote::quote_client(),
        quote::JUPITER_ORDER_ENDPOINT,
        key.as_deref(),
        &client,
        url.as_str(),
        request,
        mint,
        seed,
    )
    .await
}

#[cfg(test)]
async fn read_with(
    http: &reqwest::Client,
    order_url: &str,
    key: Option<&str>,
    rpc: &reqwest::Client,
    rpc_url: &str,
    request: &StockChainCostRequest,
    comparison: &StockComparison,
) -> Result<StockChainCost, String> {
    let seed = request
        .direction
        .quote(comparison)
        .ok_or("请先取得这个方向的链上报价")?;
    read_seed_with(http, order_url, key, rpc, rpc_url, request, &comparison.mint, seed).await
}

async fn read_seed_with(
    http: &reqwest::Client,
    order_url: &str,
    key: Option<&str>,
    rpc: &reqwest::Client,
    rpc_url: &str,
    request: &StockChainCostRequest,
    mint: &StockMintEvidence,
    seed: &StockDexQuote,
) -> Result<StockChainCost, String> {
    stock_inventory::validate_owner(&request.wallet_address)?;
    let requested = common::time::now_ms();
    let mut get = http.get(order_url).timeout(Duration::from_secs(4)).query(&[
        ("inputMint", seed.input_mint.as_str()),
        ("outputMint", seed.output_mint.as_str()),
        ("amount", seed.input_raw.as_str()),
        ("taker", request.wallet_address.as_str()),
    ]);
    if let Some(key) = key {
        get = get.header("x-api-key", key);
    }
    let response = get.send().await.map_err(|_| "Jupiter 费用试算请求失败")?;
    let body = quote::decode_jupiter_general_response(response, key.is_some(), "Jupiter").await?;
    let value: Value = serde_json::from_str(&body).map_err(|_| "Jupiter 费用响应结构异常")?;
    if value.get("error").is_some_and(|e| !e.is_null()) {
        return Err("Jupiter 未能构建费用试算交易".into());
    }
    let now = common::time::now_ms();
    let quote = stock_quotes::parse_order_quote(
        &body,
        &seed.input_mint,
        &seed.output_mint,
        &seed.input_raw,
        Some(&request.wallet_address),
        requested,
        now,
    )?;
    let encoded = value["transaction"].as_str().ok_or("未返回未签名交易")?;
    let tx = transaction::inspect(encoded, &request.wallet_address)?;
    let fees = [
        ("signature", "signatureFeeLamports", "signatureFeePayer"),
        (
            "priority",
            "prioritizationFeeLamports",
            "prioritizationFeePayer",
        ),
        ("rent", "rentFeeLamports", "rentFeePayer"),
    ]
    .into_iter()
    .map(|(kind, amount, payer)| StockNativeFee {
        kind: kind.into(),
        lamports: value[amount].as_u64().map(|n| n.to_string()),
        payer: value[payer]
            .as_str()
            .filter(|p| stock_inventory::validate_owner(p).is_ok())
            .map(str::to_owned),
    })
    .collect();
    let mut cost = StockChainCost {
        transaction: Some(transaction::artifact(&value, &quote)?),
        asset: request.asset.clone(),
        direction: request.direction,
        wallet_address: request.wallet_address.clone(),
        mint: mint.clone(),
        transaction_fingerprint: tx.fingerprint.clone(),
        valid_until_ms: quote
            .expires_at_ms
            .unwrap_or(i64::MAX)
            .min(requested + comparison::STOCK_QUOTE_MAX_AGE_MS),
        quote,
        checked_at_ms: now,
        provider_fees: fees,
        network_fee_lamports: None,
        wallet_debit_lamports: None,
        wallet_budget_lamports: None,
        wallet_required_lamports: None,
        native_valuation: None,
        simulation_slot: None,
        simulation_passed: false,
        problems: vec![],
    };
    if cost
        .provider_fees
        .iter()
        .any(|f| f.lamports.is_none() || (f.lamports.as_deref() != Some("0") && f.payer.is_none()))
    {
        cost.problems
            .push("Provider 部分费用或付款方未返回，其 SOL 预算估算未知".into());
    }
    cost.wallet_budget_lamports =
        wallet_budget(&cost.provider_fees, &request.wallet_address).map(|n| n.to_string());
    match inspect_rpc(rpc, rpc_url, encoded, &tx, &cost).await {
        Ok(RpcCost {
            fee,
            debit,
            slot,
            required,
        }) => {
            cost.network_fee_lamports = Some(fee.to_string());
            cost.wallet_debit_lamports = Some(debit.to_string());
            cost.simulation_slot = Some(slot);
            cost.simulation_passed = true;
            match required {
                Ok(n) => cost.wallet_required_lamports = Some(n.to_string()),
                Err(problem) => cost.problems.push(problem),
            }
        }
        Err(problem) => cost.problems.push(problem),
    }
    if let Some(native) = cost.wallet_debit_lamports.as_deref().filter(|n| *n != "0") {
        let remaining = cost.valid_until_ms.saturating_sub(common::time::now_ms());
        if remaining > 0 {
            match tokio::time::timeout(
                Duration::from_millis(remaining as u64),
                valuation::read_complete_at(http, order_url, key, rpc, rpc_url, &cost, native),
            )
            .await
            {
                Ok(Ok(value)) => cost.native_valuation = Some(value),
                Ok(Err(e)) => cost.problems.push(e),
                Err(_) => cost
                    .problems
                    .push("SOL 补回报价超时，未按零 USDC 计成本".into()),
            }
        }
    }
    cost.checked_at_ms = common::time::now_ms();
    if cost.checked_at_ms >= cost.valid_until_ms {
        cost.problems
            .push("试算返回时已过报价有效期，请重新试算".into());
    }
    Ok(cost)
}

fn wallet_budget(fees: &[StockNativeFee], wallet: &str) -> Option<u128> {
    fees.iter().try_fold(0u128, |sum, f| {
        let n = f.lamports.as_deref()?.parse::<u64>().ok()?;
        if n == 0 {
            return Some(sum);
        }
        sum.checked_add(if f.payer.as_deref()? == wallet {
            u128::from(n)
        } else {
            0
        })
    })
}

struct RpcCost {
    fee: u64,
    debit: u64,
    slot: u64,
    required: Result<u64, String>,
}

async fn inspect_rpc(
    client: &reqwest::Client,
    url: &str,
    encoded: &str,
    transaction: &transaction::Inspection,
    cost: &StockChainCost,
) -> Result<RpcCost, String> {
    let sample = simulate_rpc(client, url, encoded, transaction, cost).await?;
    let (fee, debit, slot) = simulation::check(
        &sample.result,
        transaction,
        cost,
        sample.fee,
        sample.minimum_slot,
    )?;
    Ok(RpcCost {
        fee,
        debit,
        slot,
        required: sample
            .funding(transaction, debit)
            .map(|(_, required)| required),
    })
}

struct ReadonlySimulation {
    result: Value,
    fee: u64,
    minimum_slot: u64,
    wallet: Result<Value, String>,
    rent: Result<u64, String>,
}

impl ReadonlySimulation {
    fn funding(&self, tx: &transaction::Inspection, debit: u64) -> Result<(u64, u64), String> {
        funding::bounds(
            &self.result,
            tx,
            self.wallet.as_ref().map_err(Clone::clone)?,
            *self.rent.as_ref().map_err(Clone::clone)?,
            self.fee,
            debit,
        )
    }
}

async fn simulate_rpc(
    client: &reqwest::Client,
    url: &str,
    encoded: &str,
    transaction: &transaction::Inspection,
    cost: &StockChainCost,
) -> Result<ReadonlySimulation, String> {
    let genesis = rpc_result_with_limit(client, url, "getGenesisHash", json!([]), 11, 4096)
        .await
        .map_err(|_| "费用 RPC 主网身份读取失败")?;
    if genesis.as_str() != Some(super::rpc::SOLANA_MAINNET_GENESIS_HASH) {
        return Err("费用 RPC 不是 Solana 主网".into());
    }
    let fee = rpc_result_with_limit(
        client,
        url,
        "getFeeForMessage",
        json!([transaction.message, {"commitment":"confirmed","minContextSlot":cost.mint.slot}]),
        12,
        8192,
    )
    .await
    .map_err(|_| "交易消息网络费读取失败，未按零处理")?;
    let fee_slot = simulation::slot(&fee, cost.mint.slot)?;
    let fee = fee["value"]
        .as_u64()
        .ok_or("交易消息已过期或网络费未返回")?;
    let wallet = rpc_result_with_limit(
        client, url, "getAccountInfo",
        json!([cost.wallet_address, {"encoding":"base64", "commitment":"confirmed", "minContextSlot":fee_slot}]),
        14, 8192,
    ).await.map_err(|_| "钱包账户读取失败，SOL 周转余额未知".to_owned())
        .and_then(|account| {
            simulation::slot(&account, fee_slot)?;
            funding::system_wallet(&account["value"])?;
            Ok(account)
        });
    let minimum_slot = wallet
        .as_ref()
        .ok()
        .and_then(|a| a["context"]["slot"].as_u64())
        .unwrap_or(fee_slot);
    let simulation = rpc_result_with_limit(
        client,
        url,
        "simulateTransaction",
        json!([encoded, {
            "encoding":"base64", "commitment":"confirmed", "sigVerify":false,
            "replaceRecentBlockhash":false, "minContextSlot":minimum_slot,
            "innerInstructions":true,
            "accounts":{"encoding":"base64","addresses":[cost.wallet_address]}
        }]),
        13,
        1024 * 1024,
    )
    .await
    .map_err(|_| "RPC 交易模拟失败或超时，没有广播交易")?;
    let rent = if wallet.is_ok() {
        rpc_result_with_limit(
            client,
            url,
            "getMinimumBalanceForRentExemption",
            json!([0, {"commitment":"confirmed"}]),
            15,
            4096,
        )
        .await
        .map_err(|_| "钱包免租保留额读取失败".to_owned())
        .and_then(|v| v.as_u64().ok_or("钱包免租保留额未知".into()))
    } else {
        Err("钱包账户未核实，免租余额未知".into())
    };
    Ok(ReadonlySimulation {
        result: simulation,
        fee,
        minimum_slot,
        wallet,
        rent,
    })
}

#[cfg(test)]
mod tests;
