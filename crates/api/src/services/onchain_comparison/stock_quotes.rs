//! Public, quote-only stock reads. This path never supplies a taker or requests a transaction.
use super::{quote, rpc_target};
use serde::Deserialize;
use serde_json::Value;
use shared_types::stocks::{comparison::*, StockDexQuote, StockMintEvidence};

const RPC: &str = "https://api.mainnet-beta.solana.com";
const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const CLOCK: &str = "SysvarC1ock11111111111111111111111111111111";

// Production always uses the existing public endpoints and provider-wide quota.
// Only test builds can replace transports; parsers and rate limiting stay shared.
#[derive(Default)]
pub(crate) struct Source {
    #[cfg(test)]
    fixture: Option<(reqwest::Client, String)>,
}

impl Source {
    pub(crate) async fn mint(&self, address: &str, decimals: u8) -> Result<StockMintEvidence, String> {
        #[cfg(test)]
        if self.fixture.is_some() {
            return self.batch_mints(&[(address.into(), decimals)]).await?
                .pop().ok_or("股票合约响应缺失")?;
        }
        mint(address, decimals).await
    }

    #[cfg(test)]
    pub(crate) fn fixture(root: &str) -> Self {
        assert!(root.starts_with("http://127.0.0.1:"));
        Self { fixture: Some((reqwest::Client::builder().no_proxy()
            .redirect(reqwest::redirect::Policy::none()).build().unwrap(), root.into())) }
    }

    pub(crate) async fn batch_mints(&self, targets: &[(String, u8)])
        -> Result<Vec<Result<StockMintEvidence, String>>, String>
    {
        #[cfg(test)]
        if let Some((client, root)) = &self.fixture {
            return batch_mints_at(client, &format!("{root}/rpc"), targets).await;
        }
        batch_mints(targets).await
    }

    pub(crate) async fn jupiter(&self, keyed: bool, input: &str, output: &str, raw: &str)
        -> Result<StockDexQuote, String>
    {
        #[cfg(test)]
        if let Some((client, root)) = &self.fixture {
            if keyed { return Err("fixture never reads provider credentials".into()); }
            let quote = quote::fetch_timed_jupiter_quote_at(client, &format!("{root}/quote"), None, input, output, raw).await?;
            return parse_quote(&quote.body, input, output, raw, quote.requested_at_ms, common::time::now_ms());
        }
        jupiter(keyed, input, output, raw).await
    }
}

pub(crate) fn interval_ms(keyed: bool) -> i64 {
    super::provider_runtime::jupiter_rate_profile(keyed).quote_interval_ms
}

pub(crate) async fn mint(address: &str, decimals: u8) -> Result<StockMintEvidence, String> {
    mint_at_min_slot(address, decimals, 0).await
}

pub(crate) async fn batch_mints(
    targets: &[(String, u8)],
) -> Result<Vec<Result<StockMintEvidence, String>>, String> {
    if targets.is_empty() || targets.len() > shared_types::stocks::STOCK_BATCH_LIMIT {
        return Err("批量合约数量无效".into());
    }
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| RPC.into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    batch_mints_at(&client, url.as_str(), targets).await
}

async fn batch_mints_at(client: &reqwest::Client, url: &str, targets: &[(String, u8)])
    -> Result<Vec<Result<StockMintEvidence, String>>, String>
{
    if targets.is_empty() || targets.len() > shared_types::stocks::STOCK_BATCH_LIMIT {
        return Err("批量合约数量无效".into());
    }
    let addresses = targets.iter().map(|(address, _)| address.as_str())
        .chain([SOLANA_USDC, CLOCK]).collect::<Vec<_>>();
    let mut response = client.post(url).json(&serde_json::json!({
        "jsonrpc":"2.0", "id":1, "method":"getMultipleAccounts",
        "params":[addresses, {"encoding":"jsonParsed", "commitment":"finalized"}]
    })).send().await.map_err(|_| "批量股票合约读取失败")?;
    if !response.status().is_success() { return Err(format!("Solana RPC HTTP {}", response.status())); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "批量合约响应不完整")? {
        if bytes.len() + chunk.len() > 512 * 1024 { return Err("批量合约响应过大".into()); }
        bytes.extend_from_slice(&chunk);
    }
    parse_batch_mints(&bytes, targets, common::time::now_ms())
}

fn parse_batch_mints(bytes: &[u8], targets: &[(String, u8)], now: i64)
    -> Result<Vec<Result<StockMintEvidence, String>>, String>
{
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "批量合约响应无效")?;
    if value.get("error").is_some_and(|v| !v.is_null()) { return Err("批量合约 RPC 返回错误".into()); }
    let rows = value["result"]["value"].as_array().ok_or("批量合约响应缺失")?;
    if rows.len() != targets.len() + 2 { return Err("批量合约返回数量不匹配".into()); }
    Ok(targets.iter().enumerate().map(|(i, (address, decimals))| {
        // Reuse the exact single-Mint checks, including extensions and the same finalized clock.
        let item = serde_json::json!({"result": {
            "context": value["result"]["context"],
            "value": [rows[i], rows[targets.len()], rows[targets.len()+1]]
        }});
        parse_mint(&serde_json::to_vec(&item).map_err(|_| "批量合约编码失败")?, address, *decimals, now)
    }).collect())
}

pub(crate) async fn mint_at_min_slot(address: &str, decimals: u8, minimum_slot: u64) -> Result<StockMintEvidence, String> {
    let endpoint = crate::services::onchain_rpc_registry::configured_url("solana").unwrap_or_else(||RPC.into());
    let (url, client) = rpc_target::rpc_target(&endpoint).await?;
    let mut response = client.post(url).json(&serde_json::json!({
        "jsonrpc":"2.0", "id":1, "method":"getMultipleAccounts",
        "params":[[address, SOLANA_USDC, CLOCK], {"encoding":"jsonParsed", "commitment":"finalized", "minContextSlot":minimum_slot}]
    })).send().await.map_err(|_| "股票 Mint / USDC / 链时钟读取失败")?;
    if !response.status().is_success() {
        return Err(format!("Solana RPC HTTP {}", response.status()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Solana RPC 响应不完整")?
    {
        if bytes.len() + chunk.len() > 128 * 1024 {
            return Err("Solana RPC 响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let mint = parse_mint(&bytes, address, decimals, common::time::now_ms())?;
    if mint.slot < minimum_slot {return Err("股票 Mint 响应早于原成交区块".into());}
    Ok(mint)
}

#[derive(Deserialize)]
struct RpcEnvelope {
    result: Option<RpcResult>,
    error: Option<Value>,
}
#[derive(Deserialize)]
struct RpcResult {
    context: RpcContext,
    value: Vec<Option<Account>>,
}
#[derive(Deserialize)]
struct RpcContext {
    slot: u64,
}
#[derive(Deserialize)]
struct Account {
    owner: String,
    executable: bool,
    data: AccountData,
}
#[derive(Deserialize)]
struct AccountData {
    parsed: Parsed,
}
#[derive(Deserialize)]
struct Parsed {
    #[serde(rename = "type")]
    kind: String,
    info: Value,
}

pub(super) fn parse_mint(
    bytes: &[u8],
    address: &str,
    decimals: u8,
    now: i64,
) -> Result<StockMintEvidence, String> {
    let envelope: RpcEnvelope =
        serde_json::from_slice(bytes).map_err(|_| "Solana Mint 响应结构异常")?;
    if envelope.error.is_some() {
        return Err("Solana Mint 查询返回 RPC 错误".into());
    }
    let result = envelope.result.ok_or("Solana Mint 响应缺失")?;
    if result.value.len() != 3 {
        return Err("Mint / USDC / 链时钟响应未齐".into());
    }
    let stock = result.value[0].as_ref().ok_or("链上股票 Mint 不存在")?;
    let usdc = result.value[1].as_ref().ok_or("USDC Mint 不存在")?;
    let clock = result.value[2].as_ref().ok_or("链时钟不存在")?;
    for account in [stock, usdc] {
        if account.executable
            || account.data.parsed.kind != "mint"
            || ![TOKEN, TOKEN_2022].contains(&account.owner.as_str())
            || account.data.parsed.info["isInitialized"] != true
        {
            return Err("链上账户不是已初始化的 SPL Mint".into());
        }
    }
    if decimals > 12
        || stock.data.parsed.info["decimals"].as_u64() != Some(u64::from(decimals))
        || (address == shared_types::stocks::STOCK_SOLANA_USDT && stock.owner != TOKEN)
        || usdc.owner != TOKEN
        || usdc.data.parsed.info["decimals"].as_u64() != Some(6)
    {
        return Err("链上精度与官方资产目录不一致".into());
    }
    if clock.owner != "Sysvar1111111111111111111111111111111111111"
        || clock.data.parsed.kind != "clock"
    {
        return Err("链时钟身份不正确".into());
    }
    let time = clock.data.parsed.info["unixTimestamp"]
        .as_i64()
        .ok_or("链时钟缺失")?;
    let chain_time_ms = time.checked_mul(1000).ok_or("链时钟超出范围")?;
    if now.abs_diff(chain_time_ms) > 60_000 {
        return Err("链时钟与本机差距过大，暂停股数换算".into());
    }
    let mut multiplier = "1".to_owned();
    let mut next_change_at_ms = None;
    let mut extensions = Vec::new();
    let raw_extensions = stock.data.parsed.info.get("extensions");
    if stock.owner == TOKEN_2022 && !raw_extensions.is_some_and(Value::is_array) {
        return Err("Token-2022 扩展未完整返回".into());
    }
    if let Some(entries) = raw_extensions.and_then(Value::as_array) {
        for entry in entries {
            let kind = entry["extension"].as_str().ok_or("Mint 扩展类型缺失")?;
            if extensions.iter().any(|e| e == kind) {
                return Err("Mint 扩展重复".into());
            }
            let state = &entry["state"];
            match kind {
                "scaledUiAmountConfig" => {
                    let effective = state["newMultiplierEffectiveTimestamp"]
                        .as_i64()
                        .ok_or("倍率生效时间缺失")?;
                    let current = state[if time >= effective {
                        "newMultiplier"
                    } else {
                        "multiplier"
                    }]
                    .as_str()
                    .filter(|s| positive(s).is_some())
                    .ok_or("股票显示数量倍率无效")?;
                    multiplier = current.to_owned();
                    if effective > time {
                        next_change_at_ms =
                            Some(effective.checked_mul(1000).ok_or("倍率生效时间无效")?);
                    }
                }
                "pausableConfig" if state["paused"] == false => {}
                "transferHook" if state.get("programId") == Some(&Value::Null) => {}
                "defaultAccountState" if state["accountState"] == "initialized" => {}
                "tokenMetadata" if state["mint"].as_str() == Some(address) => {}
                "metadataPointer"
                | "permanentDelegate"
                | "confidentialTransferMint"
                | "mintCloseAuthority" => {}
                _ => return Err(format!("股票 Mint 扩展 {kind} 尚不可用于当前股数/费用比较")),
            }
            extensions.push(kind.to_owned());
        }
    }
    Ok(StockMintEvidence {
        address: address.into(),
        decimals,
        ui_multiplier: multiplier,
        slot: result.context.slot,
        chain_time_ms,
        checked_at_ms: now,
        next_change_at_ms,
        extensions,
    })
}

pub(crate) async fn jupiter(
    keyed: bool,
    input: &str,
    output: &str,
    raw: &str,
) -> Result<StockDexQuote, String> {
    let key = if keyed {
        Some(
            super::provider_runtime::env_key("JUPITER_API_KEY")
                .ok_or("请先在设置中配置 Jupiter API Key")?,
        )
    } else {
        None
    };
    let quote = quote::fetch_timed_jupiter_quote_at(
        quote::quote_client(), quote::JUPITER_ORDER_ENDPOINT, key.as_deref(), input, output, raw,
    ).await?;
    parse_quote(&quote.body, input, output, raw, quote.requested_at_ms, common::time::now_ms())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadOnlyOrder {
    input_mint: String,
    output_mint: String,
    in_amount: String,
    out_amount: String,
    other_amount_threshold: String,
    swap_mode: String,
    router: String,
    fee_bps: Option<u16>,
    fee_mint: Option<String>,
    expire_at: Option<String>,
    transaction: Option<String>,
    taker: Option<String>,
    error_code: Option<Value>,
}

fn parse_quote(
    body: &str,
    input: &str,
    output: &str,
    raw: &str,
    requested: i64,
    received: i64,
) -> Result<StockDexQuote, String> {
    parse_order_quote(body, input, output, raw, None, requested, received)
}

pub(super) fn parse_order_quote(
    body: &str,
    input: &str,
    output: &str,
    raw: &str,
    taker: Option<&str>,
    requested: i64,
    received: i64,
) -> Result<StockDexQuote, String> {
    if body.len() > 2 * 1024 * 1024 {
        return Err("Jupiter 股票询价响应过大".into());
    }
    let order: ReadOnlyOrder =
        serde_json::from_str(body).map_err(|_| "Jupiter 股票询价缺少金额或最低到账字段")?;
    if order.input_mint != input
        || order.output_mint != output
        || order.in_amount != raw
        || order.swap_mode != "ExactIn"
        || order.taker.as_deref() != taker
        || if taker.is_some() {
            order.transaction.as_deref().is_none_or(|t| t.is_empty())
        } else {
            order.transaction.is_some()
        }
        || order.error_code.is_some()
    {
        return Err("Jupiter 返回的合约、数量或只读报价边界不匹配".into());
    }
    let amount = raw
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("输入原始数量无效")?;
    let out = order
        .out_amount
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("输出数量无效")?;
    let minimum = order
        .other_amount_threshold
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && *n <= out)
        .ok_or("最低到账数量无效")?;
    let expiry = order
        .expire_at
        .as_deref()
        .map(|t| {
            chrono::DateTime::parse_from_rfc3339(t)
                .map(|t| t.timestamp_millis())
                .map_err(|_| "Jupiter 报价到期时间无效")
        })
        .transpose()?;
    if received < requested
        || received.saturating_sub(requested) > STOCK_QUOTE_MAX_AGE_MS
        || expiry.is_some_and(|t| t <= received)
    {
        return Err("Jupiter 询价在返回前已陈旧或过期".into());
    }
    Ok(StockDexQuote {
        input_mint: input.into(),
        output_mint: output.into(),
        input_raw: amount.to_string(),
        output_raw: out.to_string(),
        minimum_output_raw: minimum.to_string(),
        router: order.router,
        fee_bps: order.fee_bps,
        fee_mint: order.fee_mint,
        requested_at_ms: requested,
        received_at_ms: received,
        expires_at_ms: expiry,
    })
}

#[cfg(test)]
mod tests;
