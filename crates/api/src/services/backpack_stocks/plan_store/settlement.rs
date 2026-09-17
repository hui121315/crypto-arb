use super::super::settlement::{native_cost, validate_topup};
use super::*;

impl PlanStore {
    pub(super) fn change_tail(
        &self,
        id: &str,
        now: i64,
        apply: impl FnOnce(&mut StockExecutionPlan) -> Result<(), String>,
    ) -> Result<StockExecutionPlan, String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let old = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("股票计划不存在")?;
        let mut plan = old.clone();
        apply(&mut plan)?;
        if plan == old {
            return Ok(old);
        }
        if old.phase != StockPlanPhase::SubmissionUnknown {
            return Err("计划不在成交收尾阶段".into());
        }
        plan.revision = old.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(old.updated_at_ms);
        validate(&plan)?;
        if !transition(&old, &plan) {
            return Err("不能改写既有补回或结算记录".into());
        }
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }

    pub(in crate::services::backpack_stocks) fn prepare_topup(
        &self,
        id: &str,
        row: StockNativeTopup,
    ) -> Result<StockExecutionPlan, String> {
        self.change_tail(id, row.prepared_at_ms, |plan| {
            if row.source_revision != plan.revision
                || row.submission.is_some()
                || plan.native_topups.len() >= 8
            {
                return Err("计划已变化或补回次数已达上限，请重新核对".into());
            }
            if plan.native_topups.last().is_some_and(|t| {
                t.submission.is_none()
                    && t.valuation
                        .replenishment
                        .as_ref()
                        .is_some_and(|p| p.valid_until_ms > row.prepared_at_ms)
            }) {
                return Err("已有有效的 SOL 补回计划，无需重复构建".into());
            }
            validate_topup(plan, &row, row.prepared_at_ms)?;
            plan.native_topups.push(row);
            Ok(())
        })
    }

    pub(in crate::services::backpack_stocks) fn begin_topup(
        &self,
        id: &str,
        index: usize,
        fingerprint: &str,
        signed: &str,
        now: i64,
    ) -> Result<(StockExecutionPlan, bool), String> {
        let mut send = false;
        let plan = self.change_tail(id, now, |plan| {
            if plan.terms.account_fingerprint != fingerprint {
                return Err("股票计划属于其他账户凭证".into());
            }
            let row = plan.native_topups.get(index).ok_or("SOL 补回计划不存在")?;
            if row.submission.is_some() {
                return Ok(());
            }
            if index + 1 != plan.native_topups.len() {
                return Err("只能提交最新补回计划".into());
            }
            let cost = native_cost(plan, row)?;
            if now < row.prepared_at_ms || now >= cost.valid_until_ms {
                return Err("SOL 补回报价已过期，未发送".into());
            }
            let submission = chain::intent(&cost, signed, now)?;
            plan.native_topups[index].submission = Some(submission);
            send = true;
            Ok(())
        })?;
        Ok((plan, send))
    }

    pub(in crate::services::backpack_stocks) fn change_topup(
        &self,
        id: &str,
        index: usize,
        now: i64,
        apply: impl FnOnce(&mut StockChainSubmission) -> Result<(), String>,
    ) -> Result<StockExecutionPlan, String> {
        self.change_tail(id, now, |plan| {
            apply(
                plan.native_topups
                    .get_mut(index)
                    .and_then(|t| t.submission.as_mut())
                    .ok_or("原 SOL 补回尚未提交")?,
            )
        })
    }

    pub(in crate::services::backpack_stocks) fn settle(
        &self,
        id: &str,
        revision: u64,
        now: i64,
    ) -> Result<StockExecutionPlan, String> {
        self.change_tail(id, now, |plan| {
            if plan.phase == StockPlanPhase::Settled {
                return Ok(());
            }
            let accounting = plan.accounting();
            if plan.revision != revision
                || plan.two_leg_started_at_ms.is_none()
                || !accounting.can_settle()
            {
                return Err("回执、费用、股票或 SOL 尚未对齐，不能结束计划或释放占用".into());
            }
            plan.settlement = Some(StockPlanSettlement {
                source_revision: revision,
                settled_at_ms: now.max(plan.updated_at_ms),
                accounting,
            });
            plan.phase = StockPlanPhase::Settled;
            Ok(())
        })
    }
}

pub(super) fn validate_tail(plan: &StockExecutionPlan) -> Result<(), String> {
    super::recovery::validate_history(plan)?;
    if plan.native_topups.len() > 8 {
        return Err("SOL 补回历史超过上限".into());
    }
    let mut prefix = plan.clone();
    prefix.native_topups.clear();
    prefix.settlement = None;
    let mut artifacts =
        std::collections::BTreeSet::from([plan.terms.chain_cost.transaction_fingerprint.clone()]);
    artifacts.extend(plan.recoveries.iter().map(|r|r.cost.transaction_fingerprint.clone()));
    if plan.phase == StockPlanPhase::Settled {
        prefix.phase = StockPlanPhase::SubmissionUnknown;
    }
    for row in &plan.native_topups {
        if row.source_revision >= plan.revision
            || row.prepared_at_ms < plan.terms.created_at_ms
            || row.prepared_at_ms > plan.updated_at_ms
        {
            return Err("SOL 补回未绑定原计划版本".into());
        }
        validate_topup(&prefix, row, row.prepared_at_ms)?;
        if !artifacts.insert(native_cost(&prefix, row)?.transaction_fingerprint) {
            return Err("SOL 补回重复使用了原交易消息，不能重复计费或提交".into());
        }
        if let Some(s) = &row.submission {
            let cost = native_cost(&prefix, row)?;
            if s.submitted_at_ms < row.prepared_at_ms
                || s.submitted_at_ms >= cost.valid_until_ms
                || s.submitted_at_ms > plan.updated_at_ms
            {
                return Err("SOL 补回提交时间无效".into());
            }
            chain::validate_record(&cost, s)?;
        }
        prefix.native_topups.push(row.clone());
    }
    match &plan.settlement {
        Some(s)
            if plan.phase == StockPlanPhase::Settled
                && s.source_revision.checked_add(1) == Some(plan.revision)
                && s.settled_at_ms == plan.updated_at_ms
                && plan.two_leg_started_at_ms.is_some()
                && s.accounting == plan.accounting()
                && s.accounting.can_settle() =>
        {
            Ok(())
        }
        None if plan.phase != StockPlanPhase::Settled => Ok(()),
        _ => Err("股票结算记录与实际回执不一致".into()),
    }
}

pub(super) fn transition(old: &StockExecutionPlan, new: &StockExecutionPlan) -> bool {
    if !super::recovery::transition(old,new) {return false;}
    if old.settlement.is_some() {
        return false;
    }
    if new.native_topups.len() < old.native_topups.len()
        || new.native_topups.len() > old.native_topups.len() + 1
    {
        return false;
    }
    if old
        .native_topups
        .iter()
        .zip(&new.native_topups)
        .enumerate()
        .any(|(index, (a, b))| {
            a.source_revision != b.source_revision
                || a.prepared_at_ms != b.prepared_at_ms
                || a.valuation != b.valuation
                || a.wallet != b.wallet
                || !chain::transition(a.submission.as_ref(), b.submission.as_ref())
                || (a.submission.is_none()
                    && b.submission.as_ref().is_some_and(|s| {
                        index + 1 != old.native_topups.len()
                            || s.submitted_at_ms < old.updated_at_ms
                    }))
        })
    {
        return false;
    }
    if let Some(row) = new.native_topups.get(old.native_topups.len()) {
        if row.source_revision != old.revision
            || row.prepared_at_ms < old.updated_at_ms
            || row.submission.is_some()
            || new.cex_order != old.cex_order
            || new.chain_submission != old.chain_submission
            || new.rfq_acceptance != old.rfq_acceptance
        {
            return false;
        }
    }
    if let Some(s) = &new.settlement {
        if s.source_revision != old.revision
            || new.recoveries != old.recoveries
            || new.native_topups != old.native_topups
            || new.cex_order != old.cex_order
            || new.chain_submission != old.chain_submission
            || new.rfq_acceptance != old.rfq_acceptance
        {
            return false;
        }
    }
    true
}
