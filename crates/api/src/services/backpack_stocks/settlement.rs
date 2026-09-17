use super::*;
use crate::services::onchain_comparison::{stock_costs, stock_inventory};
use rust_decimal::Decimal;

fn number(s: &str) -> Option<Decimal> {
    Decimal::from_str_exact(s).ok()
}

pub(super) fn native_target(plan: &StockExecutionPlan) -> Result<(u64, u64), String> {
    let report = plan.accounting();
    if plan.phase != StockPlanPhase::SubmissionUnknown
        || plan.two_leg_started_at_ms.is_none()
        || report.status != StockAccountingStatus::LegsReconciled
        || report.fee_basis_matched != Some(true)
        || plan
            .native_topups
            .iter()
            .any(|t| t.submission.as_ref().is_some_and(|s| s.receipt.is_none()))
    {
        return Err("先核对两腿及既有补回交易，不能在未明收支上再次补仓".into());
    }
    let lamports = report
        .net_sol_change
        .as_deref()
        .and_then(number)
        .and_then(|n| n.checked_mul(Decimal::from(1_000_000_000)))
        .filter(|n| *n < Decimal::ZERO && n.fract().is_zero())
        .and_then(|n| (-n).normalize().to_string().parse::<u64>().ok())
        .ok_or("没有需要补回的已核实 SOL 扣款")?;
    let slot = plan
        .chain_submission
        .as_ref()
        .and_then(|s| s.receipt.as_ref())
        .map(|r| r.slot)
        .into_iter()
        .chain(
            plan.recoveries
                .iter()
                .filter_map(|r| r.submission.as_ref()?.receipt.as_ref().map(|r| r.slot)),
        )
        .chain(
            plan.native_topups
                .iter()
                .filter_map(|t| t.submission.as_ref()?.receipt.as_ref().map(|r| r.slot)),
        )
        .max()
        .ok_or("原交易缺少最终区块")?;
    Ok((lamports, slot))
}

pub(super) fn native_cost(
    plan: &StockExecutionPlan,
    row: &StockNativeTopup,
) -> Result<StockChainCost, String> {
    native_cost_from(&plan.terms.chain_cost, &row.valuation)
}

pub(super) fn native_cost_from(
    context: &StockChainCost,
    valuation: &StockNativeValuation,
) -> Result<StockChainCost, String> {
    let cost = valuation.execution_cost(context)?;
    stock_costs::execution::validate_artifact(&cost)?;
    Ok(cost)
}

pub(super) fn validate_topup(
    plan: &StockExecutionPlan,
    row: &StockNativeTopup,
    now: i64,
) -> Result<(), String> {
    let (target, minimum_slot) = native_target(plan)?;
    let budget = row
        .valuation
        .complete_budget(&target.to_string(), &plan.request.wallet_address, now)
        .as_deref()
        .and_then(number)
        .ok_or("SOL 补回目标、模拟或报价已失效")?;
    let original = super::recovery::native_budget(plan)?;
    let spent = plan
        .native_topups
        .iter()
        .filter_map(|t| t.submission.as_ref()?.receipt.as_ref())
        .try_fold(Decimal::ZERO, |sum, r| {
            let c = r
                .asset_changes
                .iter()
                .find(|a| a.mint == shared_types::stocks::comparison::SOLANA_USDC)?;
            let n = stock_chain_quantity(&c.raw_change, c.decimals)
                .as_deref()
                .and_then(number)?;
            sum.checked_sub(n)
        })
        .ok_or("既有补回交易的 USDC 支出未知")?;
    if budget + spent > original {
        return Err("SOL 补回费用超过原计划备款，需要另行确认处置预算".into());
    }
    let proof = row.valuation.replenishment.as_ref().ok_or("补回模拟缺失")?;
    if proof.simulation_slot < minimum_slot
        || row.prepared_at_ms != now
        || row.wallet.owner != plan.request.wallet_address
        || row.wallet.mint != plan.terms.chain_cost.mint.address
        || !row.wallet.problems.is_empty()
        || row.wallet.checked_at_ms > now
        || now - row.wallet.checked_at_ms > 30_000
        || row
            .wallet
            .usdc_raw
            .as_deref()
            .and_then(|s| s.parse::<u128>().ok())
            .zip(row.valuation.quote.input_raw.parse::<u128>().ok())
            .is_none_or(|(a, b)| a < b)
        || row
            .wallet
            .sol_lamports
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .zip(proof.wallet_required_lamports.parse::<u64>().ok())
            .is_none_or(|(a, b)| a < b)
    {
        return Err("SOL 补回的当前钱包余额或模拟区块不完整".into());
    }
    native_cost(plan, row)?;
    Ok(())
}

impl BackpackStocks {
    pub(crate) async fn prepare_native_topup(
        &self,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "原交易或补回正在处理")?;
        let plan = self.plan_store.get(&request.plan_id)?;
        if plan.revision != request.revision {
            return Err("计划已更新，请使用最新回执重新构建".into());
        }
        let (native, slot) = native_target(&plan)?;
        let mut context = plan.terms.chain_cost.clone();
        context.mint.slot = context.mint.slot.max(slot);
        let wallet = stock_inventory::read(&context.wallet_address, &context.mint).await?;
        let valuation = stock_costs::read_native_replacement(&context, native).await?;
        self.plan_store.prepare_topup(
            &plan.plan_id,
            StockNativeTopup {
                source_revision: plan.revision,
                prepared_at_ms: common::time::now_ms(),
                valuation,
                wallet,
                submission: None,
            },
        )?;
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }

    pub(crate) fn settle_plan(
        &self,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        // A manual receipt refresh can discover a conflict; do not release funds mid-query.
        let _order = self
            .order_lock
            .try_lock()
            .map_err(|_| "股票订单或回执正在核对，暂不能释放预留")?;
        let _rfq = self
            .rfq_lock
            .try_lock()
            .map_err(|_| "股票 RFQ 正在核对，暂不能释放预留")?;
        let _chain = self
            .chain_lock
            .try_lock()
            .map_err(|_| "原链上交易正在核对，暂不能释放预留")?;
        let _preflight = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票预检进行中")?;
        let _quote = self.quote_lock.try_lock().map_err(|_| "股票询价进行中")?;
        let was_settled = self.plan_store.get(&request.plan_id)?.phase == StockPlanPhase::Settled;
        self.plan_store
            .settle(&request.plan_id, request.revision, common::time::now_ms())?;
        if was_settled {
            return Ok(self.snapshot());
        }
        // Do not let a new reservation reuse pre-trade balances after releasing the wallet.
        self.account.write().evidence = None;
        {
            let mut s = self.snapshot.write();
            s.preflight = None;
            s.chain_costs.clear();
        }
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }
}
