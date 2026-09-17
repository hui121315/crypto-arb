use super::*;

fn stop_reason(run: &OnchainCrossChainRun, now_ms: i64) -> Option<&'static str> {
    if !matches!(
        run.status,
        OnchainCrossChainRunStatus::AwaitingSourceFinality
            | OnchainCrossChainRunStatus::AwaitingDestinationEvidence
    ) {
        return None;
    }
    let leg = run.active_leg()?;
    if leg.recovery_started_at_ms.is_some() {
        return (leg.recovery_checks >= OnchainCrossChainLegProgress::RECOVERY_CHECK_LIMIT)
            .then_some("只读核验已达 12 轮，尚未核齐原交易收支；已暂停，不重复转账");
    }
    match run.automatic_check_deadline_ms() {
        None => Some("原交易提交时间缺失，无法确定自动核验窗口；请手动只读核验"),
        Some(deadline) if now_ms >= deadline => {
            Some("自动核验窗口已到，已暂停；保留实际收支，不代表转账失败，可手动只读核验")
        }
        _ => None,
    }
}

fn pause(run: &mut OnchainCrossChainRun, reason: &str, now_ms: i64) {
    if let Ok(leg) = active_leg_mut(run) {
        leg.status = OnchainCrossChainLegRunStatus::Paused;
        leg.last_checked_at_ms = Some(now_ms);
        leg.problem = Some(reason.into());
    }
    run.status = OnchainCrossChainRunStatus::Paused;
    run.problem = Some(reason.into());
    run.next_action = "核对已记录资产与费用；重新核验仅查询原交易，不重发转账".into();
}

impl OnchainCrossChainRunStore {
    pub(crate) fn claim_reconciliation(
        &self,
        run_id: &str,
        now_ms: i64,
    ) -> Result<Option<OnchainCrossChainRun>, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let mut run = self
            .runs
            .get(run_id)
            .ok_or("跨链记录不存在")?
            .value()
            .clone();
        if !run.reconciliation_due(now_ms) {
            return Ok(None);
        }
        if let Some(reason) = stop_reason(&run, now_ms) {
            pause(&mut run, reason, now_ms);
        } else {
            // Persist the whole query round before I/O, including failed or interrupted reads.
            let leg = active_leg_mut(&mut run)?;
            leg.last_checked_at_ms = Some(now_ms);
            if leg.recovery_started_at_ms.is_some() {
                leg.recovery_checks = leg.recovery_checks.saturating_add(1);
            }
        }
        run.updated_at_ms = now_ms;
        self.persist_run_unlocked(&mut run)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(Some(run))
    }

    pub(crate) fn finish_reconciliation(
        &self,
        run_id: &str,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                if let Some(reason) = stop_reason(run, now_ms) {
                    pause(run, reason, now_ms);
                }
                Ok(())
            },
            now_ms,
        )
    }
}

#[cfg(test)]
pub(super) mod tests;
