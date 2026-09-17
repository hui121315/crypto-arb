use super::*;

impl Store {
    pub(in crate::services::backpack_stocks) fn prepare_recovery(
        &self,
        id: &str,
        row: StockPeerRecovery,
    ) -> Result<StockPeerPlan, String> {
        self.update(id, |p| {
            if row.source_revision != p.revision
                || row.submission.is_some()
                || row.cancelled_at_ms.is_some()
                || !p.peer_recovery_available(row.prepared_at_ms)
            {
                return Err("补偿版本变化或已有补偿待处理".into());
            }
            row.validate(p, row.prepared_at_ms)?;
            execution::validate_artifact(&row.cost)?;
            p.recoveries.push(row);
            Ok(())
        })
    }
    pub(in crate::services::backpack_stocks) fn cancel_recovery(
        &self,
        r: &StockRecoveryActionRequest,
    ) -> Result<StockPeerPlan, String> {
        self.update(&r.plan_id, |p| {
            let row = p.recoveries.get_mut(r.index).ok_or("补偿不存在")?;
            if row.cancelled_at_ms.is_some() {
                return Ok(());
            }
            if r.revision != p.revision || row.submission.is_some() {
                return Err("补偿已变化或已提交，不能取消".into());
            }
            row.cancelled_at_ms = Some(common::time::now_ms());
            Ok(())
        })
    }
    pub(in crate::services::backpack_stocks) fn begin_recovery(
        &self,
        r: &StockPeerRecoverySubmitRequest,
        signed: &str,
    ) -> Result<StockPeerPlan, String> {
        self.update(&r.plan_id, |p| {
            let row = p.recoveries.get(r.index).ok_or("补偿不存在")?;
            if row.submission.is_some() {
                return Err("原补偿已经提交，只能核对原交易".into());
            }
            if !r.confirm_live
                || r.revision != p.revision
                || r.index + 1 != p.recoveries.len()
                || row.cancelled_at_ms.is_some()
            {
                return Err("本次补偿未确认、已变化或已取消".into());
            }
            let mut prefix = p.clone();
            prefix.recoveries.truncate(r.index);
            let now = common::time::now_ms();
            row.validate(&prefix, now)?;
            let intent = execution::intent(&row.cost, signed, now)?;
            p.recoveries[r.index].submission = Some(intent);
            Ok(())
        })
    }
    pub(in crate::services::backpack_stocks) fn change_recovery(
        &self,
        id: &str,
        index: usize,
        f: impl FnOnce(&mut StockChainSubmission) -> Result<(), String>,
    ) -> Result<StockPeerPlan, String> {
        self.update(id, |p| {
            f(p.recoveries
                .get_mut(index)
                .and_then(|r| r.submission.as_mut())
                .ok_or("补偿尚未提交")?)
        })
    }
}

pub(super) fn validate_history(p: &StockPeerPlan) -> Result<(), String> {
    if p.recoveries.len() > MAX_STOCK_PEER_RECOVERIES
        || (!p.recoveries.is_empty() && p.phase != StockPeerPlanPhase::SubmissionUnknown)
    {
        return Err("补偿历史数量或主计划状态无效".into());
    }
    let mut seen = std::collections::BTreeSet::from([p
        .terms
        .basis
        .chain_cost
        .transaction_fingerprint
        .clone()]);
    seen.extend(p.native_topups.iter().filter_map(|r| {
        r.terms
            .valuation
            .replenishment
            .as_ref()
            .map(|v| v.transaction_fingerprint.clone())
    }));
    for r in &p.recoveries {
        let prefix = historical_prefix(p, r.source_revision);
        if r.source_revision >= p.revision
            || r.prepared_at_ms < p.terms.created_at_ms
            || r.prepared_at_ms > p.updated_at_ms
            || !seen.insert(r.cost.transaction_fingerprint.clone())
            || !prefix.peer_recovery_available(r.prepared_at_ms)
        {
            return Err("补偿历史版本、时序或交易消息重复".into());
        }
        r.validate(&prefix, r.prepared_at_ms)?;
        execution::validate_artifact(&r.cost)?;
        if r.cancelled_at_ms
            .is_some_and(|t| r.submission.is_some() || t < r.prepared_at_ms || t > p.updated_at_ms)
        {
            return Err("补偿取消记录无效".into());
        }
        if let Some(s) = &r.submission {
            if s.submitted_at_ms < r.prepared_at_ms
                || s.submitted_at_ms >= r.cost.valid_until_ms
                || s.submitted_at_ms > p.updated_at_ms
            {
                return Err("补偿提交时间无效".into());
            }
            execution::validate_record(&r.cost, s)?;
        }
    }
    Ok(())
}

pub(super) fn transition(a: &StockPeerPlan, b: &StockPeerPlan) -> bool {
    if b.recoveries.len() < a.recoveries.len() || b.recoveries.len() > a.recoveries.len() + 1 {
        return false;
    }
    for (index, (old, new)) in a.recoveries.iter().zip(&b.recoveries).enumerate() {
        let mut base = new.clone();
        base.submission = old.submission.clone();
        base.cancelled_at_ms = old.cancelled_at_ms;
        if base != *old
            || !execution::transition(old.submission.as_ref(), new.submission.as_ref())
            || old
                .cancelled_at_ms
                .is_some_and(|t| new.cancelled_at_ms != Some(t))
            || (old.cancelled_at_ms != new.cancelled_at_ms
                && (old.submission.is_some() || new.submission.is_some()))
            || (old.submission.is_none()
                && new.submission.as_ref().is_some_and(|s| {
                    index + 1 != a.recoveries.len()
                        || s.submitted_at_ms < a.updated_at_ms
                        || old.cancelled_at_ms.is_some()
                }))
        {
            return false;
        }
    }
    if b.recoveries.get(a.recoveries.len()).is_some_and(|r| {
        r.source_revision != a.revision
            || r.prepared_at_ms < a.updated_at_ms
            || r.submission.is_some()
            || r.cancelled_at_ms.is_some()
            || r.validate(a, r.prepared_at_ms).is_err()
    }) {
        return false;
    }
    if a.recoveries != b.recoveries
        && (a.cex_order != b.cex_order
            || a.chain_submission != b.chain_submission
            || a.native_topups != b.native_topups
            || a.inventory_orders != b.inventory_orders
            || a.conversions != b.conversions)
    {
        return false;
    }
    true
}
