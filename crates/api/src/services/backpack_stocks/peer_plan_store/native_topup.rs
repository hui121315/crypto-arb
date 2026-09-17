use super::*;

impl Store {
    pub(in crate::services::backpack_stocks) fn prepare_native_topup(
        &self,
        id: &str,
        row: StockPeerNativeTopup,
    ) -> Result<StockPeerPlan, String> {
        self.update(id, |p| {
            if row.terms.source_revision != p.revision
                || row.terms.submission.is_some()
                || row.cancelled_at_ms.is_some()
            {
                return Err("SOL 补回版本变化或已有提交".into());
            }
            row.validate(p, row.terms.prepared_at_ms)?;
            execution::validate_artifact(&row.cost(p)?)?;
            p.native_topups.push(row);
            Ok(())
        })
    }
    pub(in crate::services::backpack_stocks) fn cancel_native_topup(
        &self,
        r: &StockRecoveryActionRequest,
    ) -> Result<StockPeerPlan, String> {
        self.update(&r.plan_id, |p| {
            let row = p.native_topups.get_mut(r.index).ok_or("SOL 补回不存在")?;
            if row.cancelled_at_ms.is_some() {
                return Ok(());
            }
            if r.revision != p.revision || row.terms.submission.is_some() {
                return Err("补回已变化或已提交，不能取消".into());
            }
            row.cancelled_at_ms = Some(common::time::now_ms());
            Ok(())
        })
    }
    pub(in crate::services::backpack_stocks) fn begin_native_topup(
        &self,
        r: &StockPeerRecoverySubmitRequest,
        signed: &str,
    ) -> Result<StockPeerPlan, String> {
        self.update(&r.plan_id, |p| {
            let row = p.native_topups.get(r.index).ok_or("SOL 补回不存在")?;
            if !r.confirm_live
                || r.revision != p.revision
                || r.index + 1 != p.native_topups.len()
                || row.cancelled_at_ms.is_some()
                || row.terms.submission.is_some()
            {
                return Err("SOL 补回未确认、已变化或已提交".into());
            }
            let mut prefix = p.clone();
            prefix.native_topups.truncate(r.index);
            let now = common::time::now_ms();
            row.validate(&prefix, now)?;
            let intent = execution::intent(&row.cost(p)?, signed, now)?;
            p.native_topups[r.index].terms.submission = Some(intent);
            Ok(())
        })
    }
    pub(in crate::services::backpack_stocks) fn change_native_topup(
        &self,
        id: &str,
        index: usize,
        f: impl FnOnce(&mut StockChainSubmission) -> Result<(), String>,
    ) -> Result<StockPeerPlan, String> {
        self.update(id, |p| {
            f(p.native_topups
                .get_mut(index)
                .and_then(|r| r.terms.submission.as_mut())
                .ok_or("SOL 补回尚未提交")?)
        })
    }
}

pub(super) fn validate_history(p: &StockPeerPlan) -> Result<(), String> {
    if p.native_topups.len() > MAX_STOCK_PEER_NATIVE_TOPUPS
        || !p.native_topups.is_empty() && p.phase != StockPeerPlanPhase::SubmissionUnknown
    {
        return Err("SOL 补回历史数量或父计划状态无效".into());
    }
    let mut seen = std::collections::BTreeSet::from([p
        .terms
        .basis
        .chain_cost
        .transaction_fingerprint
        .clone()]);
    seen.extend(
        p.recoveries
            .iter()
            .map(|r| r.cost.transaction_fingerprint.clone()),
    );
    for r in &p.native_topups {
        let t = &r.terms;
        let prefix = historical_prefix(p, t.source_revision);
        let cost = r.cost(p)?;
        if t.source_revision >= p.revision
            || t.prepared_at_ms < p.terms.created_at_ms
            || t.prepared_at_ms > p.updated_at_ms
            || !seen.insert(cost.transaction_fingerprint.clone())
        {
            return Err("SOL 补回历史版本、时序或交易消息重复".into());
        }
        r.validate(&prefix, t.prepared_at_ms)?;
        execution::validate_artifact(&cost)?;
        if r.cancelled_at_ms.is_some_and(|at| {
            t.submission.is_some() || at < t.prepared_at_ms || at > p.updated_at_ms
        }) {
            return Err("SOL 补回取消记录无效".into());
        }
        if let Some(s) = &t.submission {
            if s.submitted_at_ms < t.prepared_at_ms
                || s.submitted_at_ms >= cost.valid_until_ms
                || s.submitted_at_ms > p.updated_at_ms
            {
                return Err("SOL 补回提交时间无效".into());
            }
            execution::validate_record(&cost, s)?;
        }
    }
    Ok(())
}

pub(super) fn transition(a: &StockPeerPlan, b: &StockPeerPlan) -> bool {
    if b.native_topups.len() < a.native_topups.len()
        || b.native_topups.len() > a.native_topups.len() + 1
    {
        return false;
    }
    for (i, (old, new)) in a.native_topups.iter().zip(&b.native_topups).enumerate() {
        let mut base = new.clone();
        base.terms.submission = old.terms.submission.clone();
        base.cancelled_at_ms = old.cancelled_at_ms;
        if base != *old
            || !execution::transition(old.terms.submission.as_ref(), new.terms.submission.as_ref())
            || old
                .cancelled_at_ms
                .is_some_and(|t| new.cancelled_at_ms != Some(t))
            || old.cancelled_at_ms != new.cancelled_at_ms
                && (old.terms.submission.is_some() || new.terms.submission.is_some())
        {
            return false;
        }
        if old.terms.submission.is_none() && new.terms.submission.is_some() {
            let mut prefix = a.clone();
            prefix.native_topups.truncate(i);
            if i + 1 != a.native_topups.len()
                || old.cancelled_at_ms.is_some()
                || old.validate(&prefix, b.updated_at_ms).is_err()
            {
                return false;
            }
        }
    }
    if b.native_topups.get(a.native_topups.len()).is_some_and(|r| {
        r.terms.source_revision != a.revision
            || r.terms.prepared_at_ms < a.updated_at_ms
            || r.terms.submission.is_some()
            || r.cancelled_at_ms.is_some()
            || r.validate(a, r.terms.prepared_at_ms).is_err()
    }) {
        return false;
    }
    a.native_topups == b.native_topups
        || (a.cex_order == b.cex_order
            && a.chain_submission == b.chain_submission
            && a.recoveries == b.recoveries
            && a.conversions == b.conversions
            && a.inventory_orders == b.inventory_orders)
}
