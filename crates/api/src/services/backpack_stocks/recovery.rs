use super::*;
use crate::services::onchain_comparison::{stock_costs, stock_inventory, stock_quotes};
use rust_decimal::Decimal;

pub(super) fn number(s: &str) -> Result<Decimal, String> {
    Decimal::from_str_exact(s).map_err(|_| "补偿数量格式无效".into())
}

pub(super) fn minimum_slot(plan: &StockExecutionPlan) -> Result<u64, String> {
    plan.chain_submission
        .as_ref()
        .and_then(|s| s.receipt.as_ref())
        .map(|r| r.slot)
        .into_iter()
        .chain(
            plan.recoveries
                .iter()
                .filter_map(|r| r.submission.as_ref()?.receipt.as_ref().map(|r| r.slot)),
        )
        .max()
        .ok_or("原交易最终区块未知".into())
}

pub(super) fn native_budget(plan: &StockExecutionPlan) -> Result<Decimal, String> {
    let original = plan
        .terms
        .chain_cost
        .complete_native_usdc_budget(plan.terms.created_at_ms)
        .ok_or("原 SOL 备款未知")?;
    plan.recoveries
        .iter()
        .filter(|r| r.submission.is_some())
        .try_fold(number(&original)?, |sum, r| {
            let n = r
                .cost
                .complete_native_usdc_budget(r.prepared_at_ms)
                .ok_or("补偿 SOL 备款未知")?;
            sum.checked_add(number(&n)?).ok_or("SOL 备款溢出".into())
        })
}

pub(super) fn validate(
    plan: &StockExecutionPlan,
    row: &StockRecovery,
    now: i64,
) -> Result<(), String> {
    let target = plan.recovery_target()?;
    let limit =
        stock_recovery_loss_limit(&row.max_loss_usdc).ok_or("损失上限应为 0–100000 USDC")?;
    let c = &row.cost;
    let m = &plan.terms.chain_cost.mint;
    if row.target != target
        || c.asset != plan.request.asset
        || c.direction != target.direction
        || c.wallet_address != plan.request.wallet_address
        || c.mint.address != m.address
        || c.mint.decimals != m.decimals
        || number(&c.mint.ui_multiplier)? != number(&m.ui_multiplier)?
        || c.mint.checked_at_ms > now
        || now - c.mint.checked_at_ms > 60_000
        || c.mint.next_change_at_ms.is_some_and(|t| now >= t)
        || c.checked_at_ms > now
        || now >= c.valid_until_ms
        || !c.simulation_passed
        || !c.problems.is_empty()
        || c.simulation_slot
            .is_none_or(|s| s < minimum_slot(plan).unwrap_or(u64::MAX))
    {
        return Err("补偿的股票身份、份额、原回执或模拟已变化".into());
    }
    stock_costs::execution::validate_artifact(c)?;
    let raw = number(&target.stock_raw)?;
    let input = number(&c.quote.input_raw)?;
    let output = number(&c.quote.minimum_output_raw)?;
    let cash = match target.direction {
        StockChainDirection::Buy
            if c.quote.input_mint == shared_types::stocks::comparison::SOLANA_USDC
                && c.quote.output_mint == m.address
                && output >= raw =>
        {
            -input / Decimal::from(1_000_000)
        }
        StockChainDirection::Sell
            if c.quote.input_mint == m.address
                && c.quote.output_mint == shared_types::stocks::comparison::SOLANA_USDC
                && input == raw =>
        {
            output / Decimal::from(1_000_000)
        }
        _ => return Err("补偿金额或最低股票到账未覆盖实际差额".into()),
    };
    let gas = number(
        &c.complete_native_usdc_budget(now)
            .ok_or("补偿 Gas 与补回报价未齐全")?,
    )?;
    let current = number(
        &plan
            .accounting()
            .net_usdc_change
            .ok_or("实际 USDC 收支未知")?,
    )?;
    let all_gas = native_budget(plan)?
        .checked_add(gas)
        .ok_or("补偿费用溢出")?;
    let minimum = current
        .checked_add(cash)
        .and_then(|n| n.checked_sub(all_gas))
        .ok_or("补偿净额溢出")?;
    if number(&row.minimum_net_usdc)? != minimum || minimum < -limit {
        return Err(format!(
            "补偿后保守 USDC 净变化为 {minimum}，超出损失上限或预览已变化"
        ));
    }
    let w = &row.wallet;
    if w.owner != plan.request.wallet_address
        || w.mint != m.address
        || !w.problems.is_empty()
        || w.checked_at_ms > now
        || now - w.checked_at_ms > 30_000
        || w.usdc_raw
            .as_deref()
            .and_then(|n| n.parse::<u64>().ok())
            .and_then(|n| (Decimal::from(n) / Decimal::from(1_000_000)).checked_add(cash))
            .is_none_or(|after| after < all_gas)
        || w.sol_lamports
            .as_deref()
            .and_then(|n| n.parse::<u64>().ok())
            .zip(c.total_native_required_lamports(now))
            .is_none_or(|(available, required)| available < required)
        || (target.direction == StockChainDirection::Sell
            && w.stock_raw
                .as_deref()
                .and_then(|n| number(n).ok())
                .is_none_or(|n| n < raw))
    {
        return Err("钱包的股票、USDC 或 SOL 周转余额不足，未预留或提交补偿".into());
    }
    Ok(())
}

impl BackpackStocks {
    pub(crate) async fn prepare_recovery(
        &self,
        request: StockRecoveryBuildRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "原交易或补偿正在处理")?;
        let plan = self.plan_store.get(&request.plan_id)?;
        if plan.revision != request.revision
            || stock_recovery_loss_limit(&request.max_loss_usdc).is_none()
        {
            return Err("计划已变化或损失上限无效".into());
        }
        plan_store::recovery::available(&plan, common::time::now_ms())?;
        let target = plan.recovery_target()?;
        let security = self
            .catalog()
            .await?
            .rows
            .into_iter()
            .find(|s| s.asset == plan.request.asset)
            .ok_or("官方证券已不可用")?;
        let tokens = protocol::tokens(&self.read("/api/v1/assets").await?, &plan.request.asset)?;
        let snapshot = StockMarketSnapshot {
            security: Some(security),
            tokens,
            ..Default::default()
        };
        let (address, docs, decimals) = comparison::issuer(&snapshot)?;
        if address != plan.terms.chain_cost.mint.address
            || decimals != plan.terms.chain_cost.mint.decimals
        {
            return Err("官方合约映射发生变化，需人工核对".into());
        }
        let mint = stock_quotes::mint_at_min_slot(&address, decimals, minimum_slot(&plan)?).await?;
        if number(&mint.ui_multiplier)? != number(&plan.terms.chain_cost.mint.ui_multiplier)? {
            return Err("股票份额倍率已变化，请先核对公司行为".into());
        }
        let wallet = stock_inventory::read(&plan.request.wallet_address, &mint).await?;
        let keyed = self
            .snapshot
            .read()
            .comparison
            .as_ref()
            .is_some_and(|c| c.keyed);
        let cost = quote_recovery(
            &plan,
            &target,
            mint,
            docs.into(),
            keyed,
            &request.max_loss_usdc,
        )
        .await?;
        let now = common::time::now_ms();
        let cash = if target.direction == StockChainDirection::Buy {
            -number(&cost.quote.input_raw)?
        } else {
            number(&cost.quote.minimum_output_raw)?
        } / Decimal::from(1_000_000);
        let minimum = number(
            &plan
                .accounting()
                .net_usdc_change
                .ok_or("原 USDC 收支未知")?,
        )?
        .checked_add(cash)
        .and_then(|n| n.checked_sub(native_budget(&plan).ok()?))
        .and_then(|n| {
            n.checked_sub(
                number(
                    &cost
                        .complete_native_usdc_budget(now)
                        .ok_or("补偿费用未知")
                        .ok()?,
                )
                .ok()?,
            )
        })
        .ok_or("补偿费用未知或金额溢出")?;
        self.plan_store.prepare_recovery(
            &plan.plan_id,
            StockRecovery {
                source_revision: plan.revision,
                prepared_at_ms: now,
                max_loss_usdc: request.max_loss_usdc,
                target,
                cost,
                wallet,
                minimum_net_usdc: minimum.normalize().to_string(),
                cancelled_at_ms: None,
                submission: None,
            },
        )?;
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }
}

async fn quote_recovery(
    plan: &StockExecutionPlan,
    target: &StockRecoveryTarget,
    mint: StockMintEvidence,
    docs: String,
    keyed: bool,
    loss: &str,
) -> Result<StockChainCost, String> {
    let raw = target
        .stock_raw
        .parse::<u64>()
        .map_err(|_| "补偿数量无效")?;
    let mut seed = if target.direction == StockChainDirection::Buy {
        stock_quotes::jupiter(
            keyed,
            shared_types::stocks::comparison::SOLANA_USDC,
            &mint.address,
            "1000000",
        )
        .await?
    } else {
        stock_quotes::jupiter(
            keyed,
            &mint.address,
            shared_types::stocks::comparison::SOLANA_USDC,
            &target.stock_raw,
        )
        .await?
    };
    let cap = number(&plan.accounting().net_usdc_change.ok_or("实际收支未知")?)?
        .checked_add(number(loss)?)
        .ok_or("补偿金额溢出")?
        .max(Decimal::ZERO);
    for _ in 0..3 {
        if target.direction == StockChainDirection::Buy {
            let amount = next_buy_input(&seed.input_raw, &seed.minimum_output_raw, raw, cap)?;
            seed.input_raw = amount.to_string();
        }
        let comparison = StockComparison {
            asset: plan.request.asset.clone(),
            issuer_docs: docs.clone(),
            budget_usdc: plan.terms.cex_notional_usdc.clone(),
            keyed,
            mint: mint.clone(),
            buy: seed.clone(),
            sell: Some(seed.clone()),
            sell_problem: None,
            quantity_limit: None,
        };
        let cost = stock_costs::read(
            &StockChainCostRequest {
                asset: plan.request.asset.clone(),
                direction: target.direction,
                wallet_address: plan.request.wallet_address.clone(),
            },
            &comparison,
        )
        .await?;
        if target.direction == StockChainDirection::Sell
            || cost
                .quote
                .minimum_output_raw
                .parse::<u64>()
                .is_ok_and(|n| n >= raw)
        {
            return Ok(cost);
        }
        seed = cost.quote;
    }
    Err("三次精确金额报价仍不能补足股票，未保存或提交".into())
}

mod execution;

pub(super) fn next_buy_input(
    input: &str,
    minimum_output: &str,
    target: u64,
    cap: Decimal,
) -> Result<u64, String> {
    let input = input
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("报价输入无效")?;
    let output = minimum_output
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("最低到账无效")?;
    let amount =
        u64::try_from((u128::from(input) * u128::from(target)).div_ceil(u128::from(output)))
            .map_err(|_| "补偿金额溢出")?;
    if target == 0 || Decimal::from(amount) / Decimal::from(1_000_000) > cap {
        return Err("补买金额无效或已超过累计损失上限，未构建交易".into());
    }
    Ok(amount)
}

#[cfg(test)]
#[test]
fn stock_recovery_exact_input_rounds_up_from_minimum_output_and_never_exceeds_loss_cap() {
    assert_eq!(
        next_buy_input("1000000", "3", 10, number("4").unwrap()).unwrap(),
        3_333_334
    );
    assert!(next_buy_input("1000000", "3", 10, number("3.333333").unwrap()).is_err());
    assert!(next_buy_input("1000000", "0", 10, number("4").unwrap()).is_err());
    assert!(next_buy_input("1000000", "1", 0, number("4").unwrap()).is_err());
    assert!(next_buy_input(&u64::MAX.to_string(), "1", u64::MAX, Decimal::MAX).is_err());
}
