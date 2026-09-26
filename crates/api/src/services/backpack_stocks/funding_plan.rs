use super::*;
use crate::services::onchain_comparison::stock_inventory;
use rust_decimal::{prelude::ToPrimitive, Decimal, RoundingStrategy};
mod service;
mod validation;
pub(super) use validation::validate;

#[cfg(test)]
pub(super) mod tests;

pub(super) fn decimal(s: &str) -> Result<Decimal, String> {
    Decimal::from_str_exact(s)
        .ok()
        .filter(|n| *n >= Decimal::ZERO)
        .ok_or("补库数量无效".into())
}
fn exact(n: Decimal) -> String {
    n.normalize().to_string()
}

pub(super) fn prepare(
    request: StockFundingPlanRequest,
    snapshot: &StockMarketSnapshot,
    account: &StockAccountEvidence,
    wallet: &StockWalletEvidence,
    directions: &[StockPreflightDirection],
    capacity: Option<StockWithdrawalCapacity>,
    address: Option<StockDepositAddress>,
    now: i64,
) -> Result<StockFundingPlan, String> {
    stock_inventory::validate_owner(&request.wallet_address)?;
    let security = snapshot.security.clone().ok_or("尚未选择股票")?;
    let c = snapshot.comparison.as_ref().ok_or("股票映射与份额未核实")?;
    let (mint, _, decimals) = comparison::issuer(snapshot)?;
    let source_plan = request.source_plan.as_ref().map(|source| {
        snapshot.plans.iter().find(|p| p.inventory_source() == *source)
            .cloned().map(Box::new).ok_or("原归档计划缺失")
    }).transpose()?;
    let source_report = source_plan.as_ref().map(|p|p.restock_report(snapshot, account, wallet, now)).transpose()?;
    let directions = source_report.as_ref().map_or(directions, |p|p.directions.as_slice());
    if security.asset != request.security_asset
        || c.asset != request.security_asset
        || c.mint.address != mint
        || c.mint.decimals != decimals
        || wallet.owner != request.wallet_address
        || now < request.preflight_at_ms
        || now - request.preflight_at_ms > 30_000
        || now < c.mint.checked_at_ms
        || now - c.mint.checked_at_ms > 60_000
        || c.mint.next_change_at_ms.is_some_and(|t| now >= t)
    {
        return Err("股票身份、钱包或原补库检查已变化，请重新检查库存".into());
    }
    let mut funding = shared_types::stocks::funding::evaluate_funding(
        snapshot,
        directions,
        Some(account),
        Some(wallet),
        now,
    );
    let need = funding
        .iter_mut()
        .find(|r| r.direction == request.direction)
        .and_then(|r| {
            r.needs
                .iter()
                .find(|n| n.asset == request.funding_asset && n.target == request.target.label())
        })
        .cloned()
        .ok_or("当前方向没有该资产缺口，无需补库")?;
    let short = decimal(need.shortfall.as_deref().ok_or("目标缺口未知")?)?;
    let spare = decimal(need.source_spare.as_deref().ok_or("来源可调余额未知")?)?;
    if short <= Decimal::ZERO {
        return Err("没有需要补充的数量".into());
    }
    let token = need
        .token
        .clone()
        .ok_or("官方同链同合约充提证据未知或陈旧")?;
    let (quantity, budget, minimum_raw) = quantities(&request, &need, &c.mint)?;
    let destination = match request.target {
        StockFundingTarget::Backpack => {
            let a = address.as_ref().ok_or("缺少官方充值地址")?;
            if a.asset != request.security_asset
                || a.blockchain != "Solana"
                || a.account_fingerprint != account.fingerprint
                || now < a.checked_at_ms
                || now - a.checked_at_ms > 30_000
            {
                return Err("官方充值地址已变化或过期".into());
            }
            stock_inventory::validate_owner(&a.address)?;
            if a.address == wallet.owner {
                return Err("来源钱包不能与交易所充值地址相同".into());
            }
            a.address.clone()
        }
        StockFundingTarget::Solana => {
            let cap = capacity.as_ref().ok_or("尚未读取账户可提上限")?;
            if cap.asset != request.funding_asset
                || now < cap.checked_at_ms
                || now - cap.checked_at_ms > 30_000
                || decimal(&cap.quantity)? < budget
            {
                return Err("账户不借款可提上限不足或未知，不能把可用余额当可提余额".into());
            }
            wallet.owner.clone()
        }
    };
    if spare < budget {
        return Err(format!(
            "来源可调 {}，低于补库保守备款 {}；不占用另一腿交易资金",
            exact(spare),
            exact(budget)
        ));
    }
    let valid_until_ms = [
        now.saturating_add(30_000),
        account.balances_at_ms.saturating_add(30_000),
        wallet.checked_at_ms.saturating_add(30_000),
        c.mint.checked_at_ms.saturating_add(60_000),
        c.mint.next_change_at_ms.unwrap_or(i64::MAX),
        snapshot
            .token_metadata_at_ms
            .unwrap_or(0)
            .saturating_add(30_000),
        address
            .as_ref()
            .map_or(i64::MAX, |a| a.checked_at_ms.saturating_add(30_000)),
        capacity
            .as_ref()
            .map_or(i64::MAX, |c| c.checked_at_ms.saturating_add(30_000)),
    ]
    .into_iter()
    .min()
    .unwrap();
    if valid_until_ms <= now {
        return Err("补库证据已到期".into());
    }
    let terms = StockFundingPlanTerms {
        source_plan,
        account_fingerprint: account.fingerprint.clone(),
        security,
        mint: c.mint.clone(),
        token,
        need,
        destination,
        deposit_address: address,
        withdrawal_capacity: capacity,
        quantity: exact(quantity),
        minimum_credit_raw: minimum_raw.to_string(),
        source_budget: exact(budget),
        created_at_ms: now,
        valid_until_ms,
    };
    let plan = StockFundingPlan {
        followup: None,
        withdrawal: None,
        transfer: None,
        plan_id: plan_id(&request, &terms)?,
        request,
        terms,
        phase: StockFundingPlanPhase::Reserved,
        revision: 1,
        updated_at_ms: now,
    };
    validate(&plan)?;
    Ok(plan)
}

pub(super) fn plan_id(
    request: &StockFundingPlanRequest,
    terms: &StockFundingPlanTerms,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(request, terms)).map_err(|_| "补库计划无法编码")?;
    let hash = common::signing::hmac_sha256_hex(b"stock-funding-plan-v1", &bytes);
    Ok(format!("stock-funding-{}", &hash[..32]))
}

fn quantities(
    r: &StockFundingPlanRequest,
    need: &StockFundingNeed,
    mint: &StockMintEvidence,
) -> Result<(Decimal, Decimal, u64), String> {
    let token = need.token.as_ref().ok_or("充提证据未知")?;
    let short = decimal(need.shortfall.as_deref().ok_or("目标缺口未知")?)?;
    let decimals = token.native_decimals.ok_or("原生精度未知")?;
    let scale = Decimal::from(
        10_u64
            .checked_pow(u32::from(decimals))
            .ok_or("精度超出支持范围")?,
    );
    let multiplier = if r.funding_asset == r.security_asset {
        decimal(&mint.ui_multiplier)?
    } else {
        Decimal::ONE
    };
    if multiplier <= Decimal::ZERO || short <= Decimal::ZERO {
        return Err("倍率或缺口无效".into());
    }
    let raw = |n: Decimal| {
        n.checked_div(multiplier)
            .and_then(|n| n.checked_mul(scale))
            .map(|n| n.ceil())
            .and_then(|n| n.to_u64())
            .ok_or("补库数量无法转为原始单位".to_owned())
    };
    let shares = |n: u64| {
        Decimal::from(n)
            .checked_div(scale)
            .and_then(|n| n.checked_mul(multiplier))
            .ok_or("股数计算溢出".to_owned())
    };
    match r.target {
        StockFundingTarget::Backpack => {
            if token.deposit_enabled != Some(true) {
                return Err("Backpack 当前未开放该资产的 Solana 充值".into());
            }
            let min = decimal(token.minimum_deposit.as_deref().ok_or("最低充值量未知")?)?;
            let units = raw(short.max(min))?;
            let quantity = shares(units)?;
            Ok((quantity, quantity, units))
        }
        StockFundingTarget::Solana => {
            if token.withdraw_enabled != Some(true) {
                return Err("Backpack 当前未开放该资产的 Solana 提现".into());
            }
            let fee = decimal(token.withdrawal_fee.as_deref().ok_or("提现费未知")?)?;
            let min = decimal(
                token
                    .minimum_withdrawal
                    .as_deref()
                    .ok_or("最低提现量未知")?,
            )?;
            let units = raw(short)?;
            // First cover one whole native unit, then add the conservative fee reserve.
            let quantity = shares(units)?
                .checked_add(fee)
                .ok_or("提现数量溢出")?
                .max(min)
                .round_dp_with_strategy(u32::from(decimals), RoundingStrategy::ToPositiveInfinity);
            if token
                .maximum_withdrawal
                .as_deref()
                .map(decimal)
                .transpose()?
                .is_some_and(|max| quantity > max)
            {
                return Err("本次数量超过官方单笔提现上限".into());
            }
            Ok((
                quantity,
                quantity.checked_add(fee).ok_or("备款溢出")?,
                units,
            ))
        }
    }
}
