use super::super::settlement::native_cost_from;
use super::*;
use rust_decimal::{prelude::ToPrimitive, Decimal};

pub(in crate::services::backpack_stocks) fn cost(
    p: &StockStablecoinPlan,
    index: usize,
) -> Result<StockChainCost, String> {
    let row = p.native_topups.get(index).ok_or("补回计划不存在")?;
    native_cost_from(
        p.preview.cost.as_ref().ok_or("原兑换交易缺失")?,
        &row.terms.valuation,
    )
}

pub(in crate::services::backpack_stocks) fn target(
    p: &StockStablecoinPlan,
) -> Result<(u64, u64), String> {
    let report = p.native_accounting()?;
    let native = report
        .net_native_lamports
        .checked_neg()
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or("没有需要补回的 SOL 净扣")?;
    Ok((native, report.minimum_slot))
}

fn validate_topup(p: &StockStablecoinPlan, row: &StockNativeTopup, now: i64) -> Result<(), String> {
    let (target, slot) = target(p)?;
    row.valuation
        .complete_budget(
            &target.to_string(),
            &p.request.conversion.wallet_address,
            now,
        )
        .ok_or("补回目标、原交易或费用模拟过期或不完整")?;
    let report = p.native_accounting()?;
    let input = row
        .valuation
        .quote
        .input_raw
        .parse::<u64>()
        .map_err(|_| "补回 USDC 投入无效")?;
    let budget = p
        .preview
        .native_cost_usdc
        .as_deref()
        .and_then(|s| Decimal::from_str_exact(s).ok())
        .and_then(|n| n.checked_mul(Decimal::from(1_000_000)))
        .filter(|n| n.fract().is_zero())
        .and_then(|n| n.to_i128())
        .ok_or("原 SOL 补回预算未知")?;
    let wanted = i128::from(p.request.conversion.amounts_raw()?.1);
    if report
        .spent_usdc_raw
        .checked_add(i128::from(input))
        .is_none_or(|n| n > budget)
        || report
            .retained_usdc_raw
            .checked_sub(i128::from(input))
            .is_none_or(|n| n < wanted)
    {
        return Err("补回超出原费用预算或会侵占 USDC 补入目标，未增加投入".into());
    }
    let proof = row.valuation.replenishment.as_ref().ok_or("补回模拟缺失")?;
    if row.prepared_at_ms != now
        || proof.simulation_slot < slot
        || row.wallet.owner != p.request.conversion.wallet_address
        || row.wallet.mint != STOCK_SOLANA_USDT
        || !row.wallet.problems.is_empty()
        || row.wallet.checked_at_ms <= 0
        || row.wallet.checked_at_ms > now
        || now - row.wallet.checked_at_ms > 30_000
        || row
            .wallet
            .usdc_raw
            .as_deref()
            .and_then(|s| s.parse::<u128>().ok())
            .is_none_or(|n| n < u128::from(input) + wanted as u128)
        || row
            .wallet
            .sol_lamports
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .zip(proof.wallet_required_lamports.parse::<u64>().ok())
            .is_none_or(|(a, b)| a < b)
    {
        return Err("当前 USDC、SOL 余额或原交易之后的模拟区块未核实".into());
    }
    native_cost_from(
        p.preview.cost.as_ref().ok_or("原兑换交易缺失")?,
        &row.valuation,
    )?;
    Ok(())
}

impl StablecoinStore {
    fn change_tail(
        &self,
        id: &str,
        now: i64,
        change: impl FnOnce(&mut StockStablecoinPlan) -> Result<(), String>,
    ) -> Result<StockStablecoinPlan, String> {
        let mut inner = self.inner.lock();
        if let Some(e) = &inner.problem {
            return Err(e.clone());
        }
        let old = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("兑换计划不存在")?;
        if old.phase != StockStablecoinPlanPhase::Completed {
            return Err("原兑换尚未核实到账".into());
        }
        let mut next = old.clone();
        change(&mut next)?;
        if next == old {
            return Ok(old);
        }
        next.revision = old.revision.checked_add(1).ok_or("兑换版本溢出")?;
        next.updated_at_ms = now.max(old.updated_at_ms);
        super::transition(&old, &next)?;
        self.persist(&mut inner, &next, now)?;
        inner
            .rows
            .insert(next.request.request_id.clone(), next.clone());
        Ok(next)
    }

    pub(in crate::services::backpack_stocks) fn prepare_topup(
        &self,
        id: &str,
        row: StockNativeTopup,
    ) -> Result<StockStablecoinPlan, String> {
        self.change_tail(id, row.prepared_at_ms, |p| {
            if row.source_revision != p.revision
                || row.prepared_at_ms < p.updated_at_ms
                || row.submission.is_some()
                || p.native_topups.len() >= 8
                || p.native_topups
                    .iter()
                    .any(|r| r.holds_funds(row.prepared_at_ms))
            {
                return Err("计划已变化、存在待处理补回或补回次数已达上限".into());
            }
            validate_topup(p, &row, row.prepared_at_ms)?;
            p.native_topups.push(StockStablecoinNativeTopup {
                terms: row,
                cancelled_at_ms: None,
            });
            Ok(())
        })
    }

    pub(in crate::services::backpack_stocks) fn begin_topup(
        &self,
        request: &StockStablecoinSubmitRequest,
        index: usize,
        signed: &str,
        now: i64,
    ) -> Result<(StockStablecoinPlan, bool), String> {
        let mut once = false;
        let plan = self.change_tail(&request.plan_id, now, |p| {
            if !request.confirm_live {
                return Err("请确认本次 SOL 补回".into());
            }
            let row = p.native_topups.get(index).ok_or("补回计划不存在")?;
            if row.terms.submission.is_some() {
                return Ok(());
            }
            if p.revision != request.revision
                || index + 1 != p.native_topups.len()
                || !row.current(now)
            {
                return Err("补回版本变化、已取消或过期，未发送".into());
            }
            let submission = execution::intent(&cost(p, index)?, signed, now)?;
            p.native_topups[index].terms.submission = Some(submission);
            once = true;
            Ok(())
        })?;
        Ok((plan, once))
    }

    pub(in crate::services::backpack_stocks) fn change_topup(
        &self,
        id: &str,
        index: usize,
        now: i64,
        change: impl FnOnce(&mut StockChainSubmission) -> Result<(), String>,
    ) -> Result<StockStablecoinPlan, String> {
        self.change_tail(id, now, |p| {
            change(
                p.native_topups
                    .get_mut(index)
                    .and_then(|r| r.terms.submission.as_mut())
                    .ok_or("补回尚未提交")?,
            )
        })
    }

    pub(in crate::services::backpack_stocks) fn cancel_topup(
        &self,
        id: &str,
        index: usize,
        now: i64,
    ) -> Result<StockStablecoinPlan, String> {
        self.change_tail(id, now, |p| {
            let row = p.native_topups.get_mut(index).ok_or("补回计划不存在")?;
            if row.terms.submission.is_some() {
                return Err("补回已提交，只能核对原交易，不能取消占用".into());
            }
            if row.cancelled_at_ms.is_none() {
                row.cancelled_at_ms = Some(now);
            }
            Ok(())
        })
    }
}

pub(super) fn validate_history(p: &StockStablecoinPlan) -> Result<(), String> {
    if p.native_topups.len() > 8 {
        return Err("补回历史超过上限".into());
    }
    let mut prefix = p.clone();
    prefix.native_topups.clear();
    let mut fingerprints =
        std::collections::BTreeSet::from([p.request.transaction_fingerprint.clone()]);
    for row in &p.native_topups {
        let t = &row.terms;
        if t.source_revision >= p.revision
            || t.prepared_at_ms > p.updated_at_ms
            || t.prepared_at_ms < p.preview.checked_at_ms
            || prefix
                .native_topups
                .iter()
                .any(|r| r.holds_funds(t.prepared_at_ms))
            || row.cancelled_at_ms.is_some_and(|at| {
                t.submission.is_some() || at < t.prepared_at_ms || at > p.updated_at_ms
            })
        {
            return Err("补回时间、版本或取消记录不一致".into());
        }
        validate_topup(&prefix, t, t.prepared_at_ms)?;
        let c = native_cost_from(
            p.preview.cost.as_ref().ok_or("兑换缺少原交易")?,
            &t.valuation,
        )?;
        if !fingerprints.insert(c.transaction_fingerprint.clone()) {
            return Err("不能重复使用既有交易消息补回".into());
        }
        if let Some(s) = &t.submission {
            if s.submitted_at_ms < t.prepared_at_ms
                || s.submitted_at_ms >= c.valid_until_ms
                || s.submitted_at_ms > p.updated_at_ms
            {
                return Err("补回提交时间无效".into());
            }
            execution::validate_record(&c, s)?;
        }
        prefix.native_topups.push(row.clone());
    }
    Ok(())
}

pub(super) fn transition(old: &StockStablecoinPlan, new: &StockStablecoinPlan) -> bool {
    if new.native_topups == old.native_topups {
        return true;
    }
    if old.phase != StockStablecoinPlanPhase::Completed
        || old.submission != new.submission
        || new.native_topups.len() < old.native_topups.len()
        || new.native_topups.len() > old.native_topups.len() + 1
    {
        return false;
    }
    let mut changed = 0;
    for (i, (a, b)) in old.native_topups.iter().zip(&new.native_topups).enumerate() {
        if a == b {
            continue;
        }
        changed += 1;
        let x = &a.terms;
        let y = &b.terms;
        if x.source_revision != y.source_revision
            || x.prepared_at_ms != y.prepared_at_ms
            || x.valuation != y.valuation
            || x.wallet != y.wallet
            || !execution::transition(x.submission.as_ref(), y.submission.as_ref())
            || a.cancelled_at_ms.is_some()
            || (b.cancelled_at_ms.is_some() && y.submission.is_some())
            || b.cancelled_at_ms.is_some_and(|at|at<old.updated_at_ms)
            || (x.submission.is_none()
                && y.submission.is_some()
                && (i + 1 != old.native_topups.len()
                    || y.submission.as_ref().unwrap().submitted_at_ms < old.updated_at_ms))
        {
            return false;
        }
    }
    if let Some(row) = new.native_topups.get(old.native_topups.len()) {
        if row.terms.source_revision != old.revision
            || row.terms.prepared_at_ms < old.updated_at_ms
            || row.terms.submission.is_some()
            || row.cancelled_at_ms.is_some()
        {
            return false;
        }
        changed += 1;
    }
    changed == 1
}
