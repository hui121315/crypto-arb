//! Exact stock-inventory transfers. No quote, swap, key read, or broadcast while preparing.
use super::{
    replenishment_chain_transfer as compile, rpc::rpc_result_with_limit, rpc_target, stock_quotes,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use shared_types::{stocks::*, OnchainUnsignedTransaction};
use std::collections::BTreeSet;

mod creation;
mod receipt;
pub(crate) use receipt::read_with as receipt_with;

const MAINNET: &str = super::rpc::SOLANA_MAINNET_GENESIS_HASH;
const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

pub(crate) async fn endpoint() -> Result<(reqwest::Client, String), String> {
    let configured = crate::services::onchain_rpc_registry::configured_url("solana")
        .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".into());
    let (url, client) = rpc_target::rpc_target(&configured).await?;
    Ok((client, url.to_string()))
}

async fn rpc(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    rpc_result_with_limit(client, url, method, params, 1, 512 * 1024).await
}

async fn mainnet(client: &reqwest::Client, url: &str) -> Result<(), String> {
    if rpc(client, url, "getGenesisHash", json!([]))
        .await?
        .as_str()
        != Some(MAINNET)
    {
        return Err("股票补库 RPC 不是 Solana 主网".into());
    }
    Ok(())
}

fn key(value: &str) -> Result<[u8; 32], String> {
    super::stock_inventory::validate_owner(value)?;
    bs58::decode(value)
        .into_vec()
        .map_err(|_| "转账公钥无效")?
        .try_into()
        .map_err(|_| "转账公钥长度无效".into())
}

fn slot(value: &Value, minimum: u64) -> Result<u64, String> {
    value["context"]["slot"]
        .as_u64()
        .filter(|n| *n >= minimum && *n > 0)
        .ok_or("补库 RPC 样本早于已保存的股票证据".into())
}

pub(crate) fn amount(plan: &StockFundingPlan) -> Result<u64, String> {
    plan.terms
        .minimum_credit_raw
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("补库原始数量无效".into())
}

pub(crate) fn message(
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<Vec<u8>, String> {
    if plan.request.target != StockFundingTarget::Backpack
        || plan.terms.destination == plan.request.wallet_address
    {
        return Err("补库转账方向或目标地址无效".into());
    }
    let raw = amount(plan)?;
    let mint = plan
        .terms
        .token
        .contract_address
        .as_deref()
        .ok_or("补库合约缺失")?;
    if mint == "So1" {
        if [
            key(&plan.request.wallet_address)?,
            key(&plan.terms.destination)?,
            [0; 32],
        ]
        .iter()
        .collect::<BTreeSet<_>>()
        .len()
            != 3
        {
            return Err("SOL 转账账户不能重复或指向系统程序".into());
        }
        if p.source_token_account.is_some()
            || p.destination_token_account.is_some()
            || p.token_program.is_some()
            || p.account_creation.is_some()
        {
            return Err("原生 SOL 不能伪装成 Token 转账".into());
        }
        return compile::compile_solana_native_message(
            &plan.request.wallet_address,
            &plan.terms.destination,
            &p.blockhash,
            raw,
        );
    }
    let program = p.token_program.as_deref().ok_or("Token Program 未核实")?;
    if ![TOKEN, TOKEN_2022].contains(&program)
        || (mint == comparison::SOLANA_USDC && program != TOKEN)
    {
        return Err("补库 Token Program 与资产身份不一致".into());
    }
    let source = p
        .source_token_account
        .as_deref()
        .ok_or("来源 Token Account 缺失")?;
    let destination = p
        .destination_token_account
        .as_deref()
        .ok_or("接收 Token Account 缺失")?;
    let keys = [
        key(&plan.request.wallet_address)?,
        key(source)?,
        key(destination)?,
        key(mint)?,
        key(program)?,
    ];
    if keys.iter().collect::<BTreeSet<_>>().len() != keys.len() {
        return Err("转账账户不能重复".into());
    }
    let input = compile::SolanaTokenMessage {
        authority: keys[0],
        source: keys[1],
        destination: keys[2],
        mint: keys[3],
        token_program: keys[4],
        blockhash: key(&p.blockhash)?,
        amount: raw,
        decimals: plan.terms.token.native_decimals.ok_or("精度缺失")?,
    };
    if p.account_creation.is_some() {
        creation::validate(plan, p)?;
        let owner = key(&plan.terms.destination)?;
        let all = [
            keys[0],
            keys[1],
            keys[2],
            keys[3],
            keys[4],
            owner,
            [0; 32],
            key(creation::PROGRAM)?,
        ];
        if all.iter().collect::<BTreeSet<_>>().len() != all.len() {
            return Err("创建接收账户的地址不能重复".into());
        }
        compile::compile_solana_token_create_message(input, owner)
    } else {
        compile::compile_solana_token_message(input)
    }
}

pub(crate) fn validate_preparation(
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<(), String> {
    if p.prepared_at_ms < plan.terms.created_at_ms
        || p.prepared_at_ms >= plan.terms.valid_until_ms
        || p.slot < plan.terms.mint.slot
        || p.slot == 0
        || p.last_valid_block_height == 0
        || p.transaction_base64.len() > 1644
        || STANDARD.encode(compile::legacy_transaction(&message(plan, p)?)) != p.transaction_base64
    {
        return Err("原补库转账消息、数量、费用或有效期无效".into());
    }
    Ok(())
}

pub(crate) fn unsigned(
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<OnchainUnsignedTransaction, String> {
    validate_preparation(plan, p)?;
    Ok(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64: p.transaction_base64.clone(),
        request_id: plan.plan_id.clone(),
        router: "solana-rpc".into(),
        mode: "exact-transfer".into(),
        last_valid_block_height: Some(p.last_valid_block_height),
        expire_at_ms: Some(plan.terms.valid_until_ms),
    })
}

pub(crate) fn signed_identity(
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
    signed: &str,
) -> Result<String, String> {
    validate_preparation(plan, p)?;
    if signed.len() > 1644 {
        return Err("补库签名交易过大".into());
    }
    let bytes = STANDARD.decode(signed).map_err(|_| "补库签名编码无效")?;
    let original = STANDARD
        .decode(&p.transaction_base64)
        .map_err(|_| "原补库编码无效")?;
    if bytes.len() != original.len()
        || bytes.first() != Some(&1)
        || bytes[65..] != original[65..]
        || bytes[1..65].iter().all(|n| *n == 0)
    {
        return Err("补库签名改变了原消息或缺少钱包签名".into());
    }
    Ok(bs58::encode(&bytes[1..65]).into_string())
}

async fn mint_context(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
) -> Result<(u64, String), String> {
    let m = &plan.terms.mint;
    let result = rpc(client, url, "getMultipleAccounts", json!([[m.address, comparison::SOLANA_USDC,
        "SysvarC1ock11111111111111111111111111111111"],{"encoding":"jsonParsed","commitment":"finalized","minContextSlot":m.slot}])).await?;
    let program = result["value"][0]["owner"]
        .as_str()
        .ok_or("股票程序身份未知")?
        .to_owned();
    let mint = stock_quotes::parse_mint(
        &serde_json::to_vec(&json!({"result":result})).map_err(|_| "Mint 无法解析")?,
        &m.address,
        m.decimals,
        common::time::now_ms(),
    )?;
    if mint.slot < m.slot
        || mint.ui_multiplier != m.ui_multiplier
        || mint.next_change_at_ms != m.next_change_at_ms
    {
        return Err("股票份额或公司行为已变化，未转账".into());
    }
    Ok((mint.slot, program))
}

async fn token_accounts(
    client: &reqwest::Client,
    url: &str,
    owner: &str,
    mint: &str,
    program: &str,
    decimals: u8,
    minimum: u64,
) -> Result<Vec<(String, u64)>, String> {
    let result = rpc(client, url, "getTokenAccountsByOwner", json!([owner,{"mint":mint},{"encoding":"jsonParsed","commitment":"confirmed","minContextSlot":minimum}])).await?;
    slot(&result, minimum)?;
    parse_token_accounts(&result["value"], owner, mint, program, decimals)
}

fn parse_token_accounts(
    value: &Value,
    owner: &str,
    mint: &str,
    program: &str,
    decimals: u8,
) -> Result<Vec<(String, u64)>, String> {
    let rows = value
        .as_array()
        .filter(|r| r.len() <= 1024)
        .ok_or("Token Account 列表无效或过大")?;
    let mut seen = BTreeSet::new();
    let mut parsed: Vec<(String, u64)> = vec![];
    for row in rows {
        let address = row["pubkey"].as_str().ok_or("Token Account 地址缺失")?;
        key(address)?;
        let a = &row["account"];
        let info = &a["data"]["parsed"]["info"];
        if !seen.insert(address)
            || a["owner"].as_str() != Some(program)
            || a["executable"] != false
            || a["data"]["parsed"]["type"] != "account"
            || info["owner"].as_str() != Some(owner)
            || info["mint"].as_str() != Some(mint)
            || info["tokenAmount"]["decimals"].as_u64() != Some(u64::from(decimals))
        {
            return Err("Token Account 的钱包、程序、合约或精度不匹配".into());
        }
        if info["state"] == "frozen" {
            continue;
        }
        if info["state"] != "initialized" {
            return Err("Token Account 不可转账".into());
        }
        if let Some(extensions) = info.get("extensions") {
            for e in extensions.as_array().ok_or("Token Account 扩展未知")? {
                match e["extension"].as_str() {
                    Some("immutableOwner" | "pausableAccount") => {}
                    Some("memoTransfer") if e["state"]["requireIncomingTransferMemos"] == false => {
                    }
                    Some("cpiGuard") if e["state"]["lockCpi"] == false => {}
                    _ => return Err("Token Account 有未支持的转账扩展，未生成转账".into()),
                }
            }
        }
        let raw = info["tokenAmount"]["amount"]
            .as_str()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or("Token Account 原始余额未知")?;
        parsed.push((address.into(), raw));
    }
    parsed.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if !rows.is_empty() && parsed.is_empty() {
        return Err("对应 Token Account 已存在但均被冻结，不能按首次充值创建其他账户".into());
    }
    Ok(parsed)
}

async fn funds_and_simulation(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<(), String> {
    if p.account_creation.is_some() {
        creation::check(client, url, plan, p).await?;
    }
    let fee = rpc(client, url, "getFeeForMessage", json!([STANDARD.encode(message(plan,p)?),{"commitment":"confirmed","minContextSlot":p.slot}])).await?;
    slot(&fee, p.slot)?;
    let fee = fee["value"]
        .as_u64()
        .ok_or("原转账区块已过期或网络费未知")?;
    if fee > p.network_fee_lamports {
        return Err("网络费超过已确认预算，请重建补库计划".into());
    }
    let rent = rpc(
        client,
        url,
        "getMinimumBalanceForRentExemption",
        json!([0,{"commitment":"confirmed"}]),
    )
    .await?
    .as_u64()
    .ok_or("钱包免租保留额未知")?;
    let balance = rpc(
        client,
        url,
        "getBalance",
        json!([plan.request.wallet_address,{"commitment":"confirmed","minContextSlot":p.slot}]),
    )
    .await?;
    slot(&balance, p.slot)?;
    let required = p
        .retained_sol_lamports
        .max(rent)
        .checked_add(p.network_fee_lamports)
        .and_then(|n| {
            n.checked_add(
                p.account_creation
                    .as_ref()
                    .map_or(0, |c| c.rent_budget_lamports),
            )
        })
        .and_then(|n| {
            n.checked_add(if plan.request.funding_asset == "SOL" {
                amount(plan).ok()?
            } else {
                0
            })
        })
        .ok_or("SOL 备款溢出")?;
    if balance["value"].as_u64().is_none_or(|n| n < required) {
        return Err("SOL 不足以覆盖转账、接收账户创建、网络费和原套利备款".into());
    }
    let simulated = rpc(client, url, "simulateTransaction", json!([p.transaction_base64,{"encoding":"base64","sigVerify":false,"replaceRecentBlockhash":false,"commitment":"confirmed","minContextSlot":p.slot}])).await?;
    slot(&simulated, p.slot)?;
    if simulated["value"].get("err") != Some(&Value::Null) {
        return Err("原补库转账模拟失败，没有签名或广播".into());
    }
    Ok(())
}

pub(crate) async fn prepare_with(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    retained_sol_lamports: u64,
) -> Result<StockFundingTransferPreparation, String> {
    mainnet(client, url).await?;
    let (minimum, stock_program) = mint_context(client, url, plan).await?;
    let mint = plan
        .terms
        .token
        .contract_address
        .as_deref()
        .ok_or("补库合约缺失")?;
    let mut p = StockFundingTransferPreparation {
        transaction_base64: String::new(),
        blockhash: String::new(),
        last_valid_block_height: 0,
        source_token_account: None,
        destination_token_account: None,
        token_program: None,
        account_creation: None,
        network_fee_lamports: 0,
        retained_sol_lamports,
        slot: minimum,
        prepared_at_ms: common::time::now_ms(),
    };
    if mint != "So1" {
        let program = if mint == comparison::SOLANA_USDC {
            TOKEN
        } else {
            stock_program.as_str()
        };
        let decimals = plan.terms.token.native_decimals.ok_or("补库精度未知")?;
        let source = token_accounts(
            client,
            url,
            &plan.request.wallet_address,
            mint,
            program,
            decimals,
            minimum,
        )
        .await?;
        let target = token_accounts(
            client,
            url,
            &plan.terms.destination,
            mint,
            program,
            decimals,
            minimum,
        )
        .await?;
        p.source_token_account = Some(
            source
                .into_iter()
                .find(|r| r.1 >= amount(plan).unwrap_or(u64::MAX))
                .ok_or("没有单个可用 Token Account 覆盖转账数量，未转账")?
                .0,
        );
        p.destination_token_account = target.into_iter().next().map(|r| r.0);
        p.token_program = Some(program.into());
    }
    let block = rpc(
        client,
        url,
        "getLatestBlockhash",
        json!([{"commitment":"confirmed","minContextSlot":minimum}]),
    )
    .await?;
    p.slot = slot(&block, minimum)?;
    p.blockhash = block["value"]["blockhash"]
        .as_str()
        .ok_or("转账区块哈希缺失")?
        .into();
    p.last_valid_block_height = block["value"]["lastValidBlockHeight"]
        .as_u64()
        .ok_or("转账区块有效期缺失")?;
    if let Some(program) = p.token_program.as_deref() {
        if p.destination_token_account.is_none() {
            p.destination_token_account =
                Some(creation::address(&plan.terms.destination, mint, program)?);
            p.account_creation = Some(creation::prepare(client, url, plan, &p).await?);
        }
    }
    let message = message(plan, &p)?;
    let fee = rpc(
        client,
        url,
        "getFeeForMessage",
        json!([STANDARD.encode(&message),{"commitment":"confirmed","minContextSlot":p.slot}]),
    )
    .await?;
    slot(&fee, p.slot)?;
    p.network_fee_lamports = fee["value"].as_u64().ok_or("转账网络费未知")?;
    p.transaction_base64 = STANDARD.encode(compile::legacy_transaction(&message));
    funds_and_simulation(client, url, plan, &p).await?;
    p.prepared_at_ms = common::time::now_ms();
    validate_preparation(plan, &p)?;
    Ok(p)
}

pub(crate) async fn check_with(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<(), String> {
    validate_preparation(plan, p)?;
    mainnet(client, url).await?;
    let (_, program) = mint_context(client, url, plan).await?;
    if plan.request.funding_asset == plan.request.security_asset
        && p.token_program.as_deref() != Some(&program)
    {
        return Err("股票 Token Program 已变化".into());
    }
    if let Some(program) = p.token_program.as_deref() {
        let mint = plan
            .terms
            .token
            .contract_address
            .as_deref()
            .ok_or("补库合约未知")?;
        let decimals = plan.terms.token.native_decimals.ok_or("补库精度未知")?;
        let source = token_accounts(
            client,
            url,
            &plan.request.wallet_address,
            mint,
            program,
            decimals,
            p.slot,
        )
        .await?;
        let target = token_accounts(
            client,
            url,
            &plan.terms.destination,
            mint,
            program,
            decimals,
            p.slot,
        )
        .await?;
        if !source.iter().any(|(a, n)| {
            Some(a) == p.source_token_account.as_ref() && *n >= amount(plan).unwrap_or(u64::MAX)
        }) || p.account_creation.is_none()
            && !target
                .iter()
                .any(|(a, _)| Some(a) == p.destination_token_account.as_ref())
        {
            return Err("原 Token Account 的余额、所有者或可转账状态已变化".into());
        }
    }
    let height = rpc(
        client,
        url,
        "getBlockHeight",
        json!([{"commitment":"confirmed","minContextSlot":p.slot}]),
    )
    .await?
    .as_u64()
    .ok_or("当前区块高度未知")?;
    if height > p.last_valid_block_height || common::time::now_ms() >= plan.terms.valid_until_ms {
        return Err("原补库交易已过期，未提交".into());
    }
    funds_and_simulation(client, url, plan, p).await
}

pub(crate) async fn send_with(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
    signed: &str,
) -> Result<(), String> {
    let hash = signed_identity(plan, p, signed)?;
    let ack=rpc(client,url,"sendTransaction",json!([signed,{"encoding":"base64","skipPreflight":false,"preflightCommitment":"confirmed","maxRetries":0,"minContextSlot":p.slot}])).await?;
    if ack.as_str() != Some(&hash) {
        return Err("补库广播回复不匹配，只查询原交易".into());
    }
    Ok(())
}
