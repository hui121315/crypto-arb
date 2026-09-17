use super::*;

pub(in crate::services::backpack_stocks) fn validate(
    plan: &StockFundingPlan,
) -> Result<(), String> {
    let r = &plan.request;
    let t = &plan.terms;
    stock_inventory::validate_owner(&r.wallet_address)?;
    stock_inventory::validate_owner(&t.destination)?;
    let (quantity, budget, raw) = quantities(r, &t.need, &t.mint)?;
    let available = decimal(t.need.available.as_deref().ok_or("目标余额未知")?)?;
    let required = decimal(t.need.required.as_deref().ok_or("目标数量未知")?)?;
    let short = decimal(t.need.shortfall.as_deref().ok_or("目标缺口未知")?)?;
    let spare = decimal(t.need.source_spare.as_deref().ok_or("来源可调余额未知")?)?;
    let source = decimal(t.need.source_available.as_deref().ok_or("来源余额未知")?)?;
    if !(16..=128).contains(&r.request_id.len())
        || !r
            .request_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        || plan.plan_id != plan_id(r, t)?
        || t.account_fingerprint.is_empty()
        || r.security_asset != t.security.asset
        || t.need.asset != r.funding_asset
        || t.need.target != r.target.label()
        || t.need.source
            != match r.target {
                StockFundingTarget::Backpack => "Solana",
                StockFundingTarget::Solana => "Backpack",
            }
        || t.need.token.as_ref() != Some(&t.token)
        || t.token.blockchain != "Solana"
        || t.created_at_ms <= 0
        || t.valid_until_ms <= t.created_at_ms
        || t.valid_until_ms > t.created_at_ms.saturating_add(30_000)
        || !fresh(r.preflight_at_ms, t.created_at_ms, 30_000)
        || !fresh(t.mint.checked_at_ms, t.created_at_ms, 60_000)
        || t.valid_until_ms > t.mint.checked_at_ms.saturating_add(60_000)
        || t.valid_until_ms > t.mint.next_change_at_ms.unwrap_or(i64::MAX)
        || !fresh(t.need.metadata_at_ms.unwrap_or(0), t.created_at_ms, 30_000)
        || t.valid_until_ms > t.need.metadata_at_ms.unwrap_or(0).saturating_add(30_000)
        || plan.updated_at_ms < t.created_at_ms
        || plan.revision == 0
        || plan.phase == StockFundingPlanPhase::Expired
        || t.quantity != exact(quantity)
        || t.source_budget != exact(budget)
        || t.minimum_credit_raw != raw.to_string()
        || raw == 0
        || required.checked_sub(available) != Some(short)
        || spare < budget
        || source < spare
    {
        return Err("补库计划身份、金额或证据无效".into());
    }
    let mut stock_token = t.token.clone();
    stock_token.contract_address = Some(t.mint.address.clone());
    stock_token.native_decimals = Some(t.mint.decimals);
    comparison::issuer(&StockMarketSnapshot {
        security: Some(t.security.clone()),
        tokens: vec![stock_token],
        ..Default::default()
    })?;
    let (contract, decimals) = match r.funding_asset.as_str() {
        "USDC" => (shared_types::stocks::comparison::SOLANA_USDC, 6),
        "SOL" => ("So1", 9),
        asset if asset == r.security_asset => (t.mint.address.as_str(), t.mint.decimals),
        _ => return Err("补库资产身份未核实".into()),
    };
    if t.token.contract_address.as_deref() != Some(contract)
        || t.token.native_decimals != Some(decimals)
    {
        return Err("补库网络、合约或精度不匹配".into());
    }
    match r.target {
        StockFundingTarget::Backpack => {
            let a = t.deposit_address.as_ref().ok_or("缺少官方充值地址")?;
            if t.withdrawal_capacity.is_some()
                || a.asset != r.security_asset
                || a.blockchain != "Solana"
                || a.account_fingerprint != t.account_fingerprint
                || a.address != t.destination
                || a.address == r.wallet_address
                || !fresh(a.checked_at_ms, t.created_at_ms, 30_000)
                || t.valid_until_ms > a.checked_at_ms.saturating_add(30_000)
            {
                return Err("官方充值地址身份或时效不一致".into());
            }
        }
        StockFundingTarget::Solana => {
            let c = t.withdrawal_capacity.as_ref().ok_or("缺少可提上限")?;
            if t.deposit_address.is_some()
                || t.destination != r.wallet_address
                || c.asset != r.funding_asset
                || decimal(&c.quantity)? < budget
                || !fresh(c.checked_at_ms, t.created_at_ms, 30_000)
                || t.valid_until_ms > c.checked_at_ms.saturating_add(30_000)
            {
                return Err("可提上限、目标钱包或时效不一致".into());
            }
        }
    }
    super::super::funding_withdrawal::validate(plan)
}

fn fresh(at: i64, now: i64, age: i64) -> bool {
    at > 0 && now >= at && now.saturating_sub(at) <= age
}
