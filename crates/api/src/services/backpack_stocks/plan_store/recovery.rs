use super::*;
use crate::services::backpack_stocks::recovery as logic;

pub(in crate::services::backpack_stocks) fn available(
    plan: &StockExecutionPlan,
    now: i64,
) -> Result<(), String> {
    if plan.recoveries.len() >= 8
        || !plan.native_topups.is_empty()
        || plan.recoveries.last().is_some_and(|r| {
            r.submission.as_ref().is_some_and(|s| s.receipt.is_none())
                || (r.submission.is_none()
                    && r.cancelled_at_ms.is_none()
                    && now < r.cost.valid_until_ms)
        })
    {
        return Err("已有补偿待提交/核对，或已进入 SOL 收尾；不能另建补偿".into());
    }
    Ok(())
}

impl PlanStore {
    pub(in crate::services::backpack_stocks) fn prepare_recovery(
        &self,
        id: &str,
        row: StockRecovery,
    ) -> Result<StockExecutionPlan, String> {
        self.change_tail(id, row.prepared_at_ms, |plan| {
            available(plan, row.prepared_at_ms)?;
            if row.source_revision != plan.revision
                || row.submission.is_some()
                || row.cancelled_at_ms.is_some()
            {
                return Err("补偿版本或初始状态无效".into());
            }
            logic::validate(plan, &row, row.prepared_at_ms)?;
            plan.recoveries.push(row);
            Ok(())
        })
    }

    pub(in crate::services::backpack_stocks) fn cancel_recovery(
        &self,
        request: &StockRecoveryActionRequest,
        now: i64,
    ) -> Result<StockExecutionPlan, String> {
        self.change_tail(&request.plan_id, now, |plan| {
            let row = plan
                .recoveries
                .get_mut(request.index)
                .ok_or("补偿计划不存在")?;
            if row.cancelled_at_ms.is_some() {
                return Ok(());
            }
            if request.revision != plan.revision || row.submission.is_some() {
                return Err("补偿已变化或已提交，不能取消原交易".into());
            }
            row.cancelled_at_ms = Some(now.max(plan.updated_at_ms));
            Ok(())
        })
    }

    pub(in crate::services::backpack_stocks) fn begin_recovery(
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
                return Err("补偿属于其他账户".into());
            }
            let row = plan.recoveries.get(index).ok_or("补偿计划不存在")?;
            if row.submission.is_some() {
                return Ok(());
            }
            if index + 1 != plan.recoveries.len()
                || row.cancelled_at_ms.is_some()
                || now < row.prepared_at_ms
                || now >= row.cost.valid_until_ms
            {
                return Err("补偿已取消或过期，未发送".into());
            }
            let intent = chain::intent(&row.cost, signed, now)?;
            plan.recoveries[index].submission = Some(intent);
            send = true;
            Ok(())
        })?;
        Ok((plan, send))
    }

    pub(in crate::services::backpack_stocks) fn change_recovery(
        &self,
        id: &str,
        index: usize,
        now: i64,
        apply: impl FnOnce(&mut StockChainSubmission) -> Result<(), String>,
    ) -> Result<StockExecutionPlan, String> {
        self.change_tail(id, now, |plan| {
            apply(
                plan.recoveries
                    .get_mut(index)
                    .and_then(|r| r.submission.as_mut())
                    .ok_or("补偿尚未提交")?,
            )
        })
    }
}

pub(super) fn validate_history(plan: &StockExecutionPlan) -> Result<(), String> {
    if plan.recoveries.len() > 8 {
        return Err("补偿历史超出上限".into());
    }
    let mut prefix = plan.clone();
    prefix.recoveries.clear();
    prefix.native_topups.clear();
    prefix.settlement = None;
    if prefix.phase == StockPlanPhase::Settled {
        prefix.phase = StockPlanPhase::SubmissionUnknown;
    }
    let mut seen =
        std::collections::BTreeSet::from([plan.terms.chain_cost.transaction_fingerprint.clone()]);
    for row in &plan.recoveries {
        if row.source_revision >= plan.revision
            || row.prepared_at_ms < plan.terms.created_at_ms
            || row.prepared_at_ms > plan.updated_at_ms
            || !seen.insert(row.cost.transaction_fingerprint.clone())
        {
            return Err("补偿版本、时间或交易消息重复".into());
        }
        available(&prefix, row.prepared_at_ms)?;
        logic::validate(&prefix, row, row.prepared_at_ms)?;
        if row.cancelled_at_ms.is_some_and(|t| {
            row.submission.is_some() || t < row.prepared_at_ms || t > plan.updated_at_ms
        }) {
            return Err("补偿取消记录无效".into());
        }
        if let Some(s) = &row.submission {
            if s.submitted_at_ms < row.prepared_at_ms
                || s.submitted_at_ms >= row.cost.valid_until_ms
                || s.submitted_at_ms > plan.updated_at_ms
            {
                return Err("补偿提交时间无效".into());
            }
            chain::validate_record(&row.cost, s)?;
        }
        prefix.recoveries.push(row.clone());
    }
    Ok(())
}

pub(super) fn transition(old: &StockExecutionPlan, new: &StockExecutionPlan) -> bool {
    if new.recoveries.len() < old.recoveries.len()
        || new.recoveries.len() > old.recoveries.len() + 1
    {
        return false;
    }
    for (index, (a, b)) in old.recoveries.iter().zip(&new.recoveries).enumerate() {
        let mut same = b.clone();
        same.submission = a.submission.clone();
        same.cancelled_at_ms = a.cancelled_at_ms;
        if &same != a
            || !chain::transition(a.submission.as_ref(), b.submission.as_ref())
            || (a.cancelled_at_ms.is_some() && a.cancelled_at_ms != b.cancelled_at_ms)
            || (a.cancelled_at_ms != b.cancelled_at_ms
                && (a.submission.is_some() || b.submission.is_some()))
            || (a.submission.is_none()
                && b.submission.as_ref().is_some_and(|s| {
                    index + 1 != old.recoveries.len()
                        || s.submitted_at_ms < old.updated_at_ms
                        || a.cancelled_at_ms.is_some()
                }))
        {
            return false;
        }
    }
    if let Some(row) = new.recoveries.get(old.recoveries.len()) {
        if row.source_revision != old.revision
            || row.prepared_at_ms < old.updated_at_ms
            || row.submission.is_some()
            || row.cancelled_at_ms.is_some()
        {
            return false;
        }
    }
    if old.recoveries != new.recoveries
        && (old.cex_order != new.cex_order
            || old.chain_submission != new.chain_submission
            || old.rfq_acceptance != new.rfq_acceptance
            || old.native_topups != new.native_topups
            || old.settlement != new.settlement)
    {
        return false;
    }
    true
}
