use super::*;
use rust_decimal::Decimal;
use std::collections::BTreeSet;

pub const STOCK_CONVERSION_COST_LIMIT: usize = 8;

impl StockMarketSnapshot {
    pub fn selected_conversion_fee_usdc(&self, ids: &[String]) -> Result<Decimal, String> {
        if ids.len() > STOCK_CONVERSION_COST_LIMIT
            || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err("兑换费用数量超限或重复".into());
        }
        if !ids.is_empty() {
            if let Some(problem) = &self.exchange_conversion_problem {
                return Err(problem.clone());
            }
        }
        ids.iter().try_fold(Decimal::ZERO, |total, id| {
            if self.claimed_conversion_cost_ids.contains(id) {
                return Err("兑换费用已归入其他计划".into());
            }
            let p = self
                .exchange_conversions
                .iter()
                .find(|p| &p.plan_id == id)
                .ok_or("已选兑换费用不可用")?;
            total
                .checked_add(p.confirmed_fee_usdc()?)
                .ok_or_else(|| "兑换费用合计溢出".into())
        })
    }

    pub fn difference_after_conversion_costs(
        &self,
        ids: &[String],
        budget: &str,
    ) -> Result<String, String> {
        Decimal::from_str_exact(budget)
            .map_err(|_| "交易预算未知")?
            .checked_sub(self.selected_conversion_fee_usdc(ids)?)
            .map(|n| n.normalize().to_string())
            .ok_or_else(|| "费用归集溢出".into())
    }
}

impl StockExchangeConversionPlan {
    pub fn confirmed_fee_usdc(&self) -> Result<Decimal, String> {
        let order = self.order.as_ref().ok_or("兑换尚无成交回执")?;
        if self.cancelled_at_ms.is_some()
            || order.phase != StockCexOrderPhase::Filled
            || order.order_id.as_deref().is_none_or(str::is_empty)
        {
            return Err("只能归入已完整成交的原兑换费用".into());
        }
        self.accounting()?;
        order.fills.iter().try_fold(Decimal::ZERO, |total, fill| {
            let fee = fill.fee.as_ref().ok_or("兑换实际费用缺失")?;
            let n = Decimal::from_str_exact(&fee.quantity).map_err(|_| "兑换费用无效")?;
            if fee.asset != "USDC" && !n.is_zero() {
                return Err("兑换费用不是 USDC，不能隐式换算".into());
            }
            total.checked_add(n).ok_or_else(|| "兑换费用溢出".into())
        })
    }
}

impl StockPlanTerms {
    pub fn conversion_fee_usdc(&self) -> Result<Decimal, String> {
        if self.conversion_costs.len() > STOCK_CONVERSION_COST_LIMIT {
            return Err("最多归入 8 笔兑换费用".into());
        }
        let mut plans = BTreeSet::new();
        let mut orders = BTreeSet::new();
        self.conversion_costs
            .iter()
            .try_fold(Decimal::ZERO, |total, p| {
                let fee = p.confirmed_fee_usdc()?;
                if p.terms.account_fingerprint != self.account_fingerprint
                    || p.updated_at_ms > self.created_at_ms
                    || !plans.insert(&p.plan_id)
                    || !orders.insert(p.order.as_ref().and_then(|o| o.order_id.as_ref()))
                {
                    return Err("兑换费用属于其他账户、尚未发生或重复归入".into());
                }
                total
                    .checked_add(fee)
                    .ok_or_else(|| "兑换费用合计溢出".into())
            })
    }
}

impl StockExecutionPlan {
    pub fn conversion_cost_ids(&self) -> Vec<String> {
        self.terms
            .conversion_costs
            .iter()
            .map(|p| p.plan_id.clone())
            .collect()
    }

    pub fn claims_conversion_cost(&self, source: &StockExchangeConversionPlan, now: i64) -> bool {
        // A completed trade keeps its cost attribution after wallet reservations are released.
        (self.holds_funds(now)
            || self.phase == StockPlanPhase::Settled
            || self.cex_order.is_some()
            || self.rfq_acceptance.is_some()
            || self.chain_submission.is_some())
            && self.terms.conversion_costs.iter().any(|p| {
                p.plan_id == source.plan_id
                    || p.order
                        .as_ref()
                        .and_then(|o| o.order_id.as_deref())
                        .zip(source.order.as_ref().and_then(|o| o.order_id.as_deref()))
                        .is_some_and(|(a, b)| a == b)
            })
    }

    pub fn check_conversion_sources(
        &self,
        sources: &[StockExchangeConversionPlan],
    ) -> Result<(), String> {
        self.terms.conversion_fee_usdc()?;
        for bound in &self.terms.conversion_costs {
            let current = sources
                .iter()
                .find(|p| p.plan_id == bound.plan_id)
                .ok_or("原兑换费用记录缺失，不能沿用旧费用")?;
            current.confirmed_fee_usdc()?;
            let original = bound.order.as_ref().ok_or("原兑换回执缺失")?;
            let actual = current.order.as_ref().ok_or("当前兑换回执缺失")?;
            if current.request != bound.request
                || current.terms != bound.terms
                || actual.order_id != original.order_id
                || actual.executed_quantity != original.executed_quantity
                || actual.executed_quote_quantity != original.executed_quote_quantity
                || actual.fills != original.fills
            {
                return Err("原兑换收支或费用已变化，请核对原记录".into());
            }
        }
        Ok(())
    }
}
