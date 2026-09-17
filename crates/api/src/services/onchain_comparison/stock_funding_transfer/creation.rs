//! ATA identity and size follow the official Associated Token and Token programs.
use super::*;
use solana_pubkey::Pubkey;

pub(super) const PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

pub(super) fn address(owner: &str, mint: &str, program: &str) -> Result<String, String> {
    let seeds = [key(owner)?, key(program)?, key(mint)?];
    Pubkey::try_find_program_address(
        &[&seeds[0], &seeds[1], &seeds[2]],
        &Pubkey::new_from_array(key(PROGRAM)?),
    )
    .map(|(address, _)| address.to_string())
    .ok_or("接收 ATA 地址无法推导".into())
}

pub(super) fn validate(
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<(), String> {
    let creation = p.account_creation.as_ref().ok_or("接收账户创建预算缺失")?;
    let program = p.token_program.as_deref().ok_or("接收账户程序缺失")?;
    let mint = plan
        .terms
        .token
        .contract_address
        .as_deref()
        .ok_or("接收账户 Mint 缺失")?;
    if p.destination_token_account.as_deref()
        != Some(address(&plan.terms.destination, mint, program)?.as_str())
        || creation.rent_budget_lamports == 0
        || !(165..=10_240).contains(&creation.account_size)
        || program == TOKEN && creation.account_size != 165
        || program == TOKEN_2022 && creation.account_size < 170
    {
        return Err("接收 ATA 身份、空间或创建预算无效".into());
    }
    Ok(())
}

async fn inspect(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<(), String> {
    let target = p
        .destination_token_account
        .as_deref()
        .ok_or("接收 ATA 缺失")?;
    let result = rpc(
        client,
        url,
        "getAccountInfo",
        json!([target,{"encoding":"jsonParsed","commitment":"confirmed","minContextSlot":p.slot}]),
    )
    .await?;
    slot(&result, p.slot)?;
    let a = result.get("value").ok_or("接收 ATA 状态缺失")?;
    if a.is_null() {
        return Ok(());
    }
    if a["owner"] == "11111111111111111111111111111111"
        && a["executable"] == false
        && a["lamports"].as_u64().is_some()
        && a["data"] == json!(["", "base64"])
    {
        return Ok(());
    }
    let rows = parse_token_accounts(
        &json!([{"pubkey":target,"account":a}]),
        &plan.terms.destination,
        plan.terms
            .token
            .contract_address
            .as_deref()
            .ok_or("接收 Mint 缺失")?,
        p.token_program.as_deref().ok_or("接收程序缺失")?,
        plan.terms.token.native_decimals.ok_or("接收精度缺失")?,
    )?;
    if rows.len() != 1 {
        return Err("原接收 ATA 已冻结或不可转账".into());
    }
    Ok(())
}

async fn budget(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<StockFundingAccountCreation, String> {
    let program = p.token_program.as_deref().ok_or("接收程序缺失")?;
    let mint = plan
        .terms
        .token
        .contract_address
        .as_deref()
        .ok_or("接收 Mint 缺失")?;
    // GetAccountDataSize with ImmutableOwner (7) includes the mint's required account extensions.
    let message = compile::compile_legacy_message(
        [1, 0, 2],
        &[
            key(&plan.request.wallet_address)?,
            key(mint)?,
            key(program)?,
        ],
        key(&p.blockhash)?,
        2,
        &[1],
        &[21, 7, 0],
    )?;
    let result=rpc(client,url,"simulateTransaction",json!([STANDARD.encode(compile::legacy_transaction(&message)),
        {"encoding":"base64","sigVerify":false,"replaceRecentBlockhash":false,"commitment":"confirmed","minContextSlot":p.slot}])).await?;
    slot(&result, p.slot)?;
    let returned = &result["value"]["returnData"];
    if result["value"].get("err") != Some(&Value::Null)
        || returned["programId"].as_str() != Some(program)
        || returned["data"][1] != "base64"
    {
        return Err("Token 程序未证明接收账户所需空间，未转账".into());
    }
    let bytes = STANDARD
        .decode(returned["data"][0].as_str().ok_or("接收账户空间缺失")?)
        .map_err(|_| "接收账户空间编码无效")?;
    let size = u64::from_le_bytes(bytes.try_into().map_err(|_| "接收账户空间长度无效")?);
    let account_size = u32::try_from(size)
        .ok()
        .filter(|n| (165..=10_240).contains(n))
        .ok_or("接收账户空间超出可核验范围")?;
    if program == TOKEN && account_size != 165 || program == TOKEN_2022 && account_size < 170 {
        return Err("Token 程序返回的空间与账户类型不符".into());
    }
    let rent = rpc(
        client,
        url,
        "getMinimumBalanceForRentExemption",
        json!([account_size,{"commitment":"confirmed"}]),
    )
    .await?
    .as_u64()
    .filter(|n| *n > 0)
    .ok_or("接收账户创建所需 SOL 未知")?;
    Ok(StockFundingAccountCreation {
        account_size,
        rent_budget_lamports: rent,
    })
}

pub(super) async fn prepare(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<StockFundingAccountCreation, String> {
    inspect(client, url, plan, p).await?;
    budget(client, url, plan, p).await
}

pub(super) async fn check(
    client: &reqwest::Client,
    url: &str,
    plan: &StockFundingPlan,
    p: &StockFundingTransferPreparation,
) -> Result<(), String> {
    validate(plan, p)?;
    inspect(client, url, plan, p).await?;
    let current = budget(client, url, plan, p).await?;
    let original = p
        .account_creation
        .as_ref()
        .ok_or("原接收账户创建预算缺失")?;
    if current.account_size != original.account_size
        || current.rent_budget_lamports > original.rent_budget_lamports
    {
        return Err("接收账户空间或创建费用已变化，请重新核算补库".into());
    }
    Ok(())
}
