use super::*;
use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;

pub(super) fn expired(run: &OnchainReplenishmentRun, limit_ms: i64, now_ms: i64) -> bool {
    // Manual recovery is bounded by durable round counts, not the original deadline.
    !run.read_only_recovery
        && run.transfers.last().is_some_and(|transfer| {
            now_ms.saturating_sub(transfer.submission_attempted_at_ms) >= limit_ms
        })
}

pub(super) fn record_withdrawal_check(
    store: &OnchainReplenishmentPlanStore,
    run: &OnchainReplenishmentRun,
    outcome: Result<Option<exchange::WithdrawalStatusEvidence>, exchange::ExchangeError>,
    now_ms: i64,
) -> Result<OnchainReplenishmentRun, String> {
    let updated = match outcome {
        Ok(Some(evidence)) => store.record_withdrawal_status(&run.run_id, evidence, now_ms),
        Ok(None) if run.status == OnchainReplenishmentRunStatus::Submitting => store
            .pause_submission(
                &run.run_id,
                "交易所历史未找到提交占位对应记录；为防重复提币已暂停自动恢复".into(),
                now_ms,
            ),
        Ok(None) if expired(run, SOURCE_HISTORY_GRACE_MS, now_ms) => store.pause_submission(
            &run.run_id,
            "交易所已确认接收提币，但 10 分钟内未返回历史记录；已暂停并等待人工核对".into(),
            now_ms,
        ),
        Ok(None) => store.record_source_check_problem(
            &run.run_id,
            "交易所提币历史尚未索引原提币号".into(),
            now_ms,
        ),
        Err(error) => store.record_source_check_problem(
            &run.run_id,
            format!("交易所提币终态查询失败：{error}"),
            now_ms,
        ),
    }?;
    let now_ms = now_ms.max(updated.updated_at_ms);
    if matches!(
        updated.status,
        OnchainReplenishmentRunStatus::Submitting
            | OnchainReplenishmentRunStatus::AwaitingSourceFinality
    ) && expired(
        &updated,
        shared_types::ONCHAIN_REPLENISHMENT_SOURCE_WAIT_MS,
        now_ms,
    ) {
        let detail = updated.problem.as_deref().unwrap_or("交易所仍报告处理中");
        return store.pause_submission(&run.run_id,
            format!("提币超过本次 2 小时自动核验窗口：{detail}；保留原提币号，仅暂停核验，不认定转账失败，也不会重发"), now_ms);
    }
    Ok(updated)
}

#[cfg(test)]
mod tests;
