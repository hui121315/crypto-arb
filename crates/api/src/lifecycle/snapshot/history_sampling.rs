use super::SnapshotSinks;
use tracing::warn;

const OPPORTUNITY_HISTORY_APPEND_INTERVAL_MS: i64 = 30_000;
/// append 在 `scan_lock` 持有期间执行且位于 WS 推送之前：必须有上界，
/// 否则 Postgres 半死（TCP 重传分钟级）会冻结整条 arbitrage 实时链路。
const OPPORTUNITY_HISTORY_APPEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Default)]
pub(super) struct OpportunityHistorySchedule {
    next_due_at_ms: i64,
}

impl OpportunityHistorySchedule {
    fn is_due(&self, now_ms: i64) -> bool {
        now_ms >= self.next_due_at_ms
    }

    fn record_success(&mut self, now_ms: i64) {
        self.next_due_at_ms = now_ms.saturating_add(OPPORTUNITY_HISTORY_APPEND_INTERVAL_MS);
    }
}

pub(super) async fn append_history_if_due(
    sinks: &SnapshotSinks,
    opportunities: &[shared_types::ArbitrageOpportunityDto],
    schedule: &mut OpportunityHistorySchedule,
    now_ms: i64,
) -> bool {
    if !schedule.is_due(now_ms) {
        return true;
    }
    let appended = append_history(sinks, opportunities).await;
    if appended {
        schedule.record_success(now_ms);
    }
    appended
}

async fn append_history(
    sinks: &SnapshotSinks,
    opportunities: &[shared_types::ArbitrageOpportunityDto],
) -> bool {
    let outcome = tokio::time::timeout(
        OPPORTUNITY_HISTORY_APPEND_TIMEOUT,
        sinks.history.append_opportunities(opportunities),
    )
    .await;
    record_append_outcome(outcome)
}

fn record_append_outcome(
    outcome: Result<Result<(), realtime::history::HistoryError>, tokio::time::error::Elapsed>,
) -> bool {
    let error = match outcome {
        Ok(Ok(())) => return true,
        Ok(Err(error)) => error.to_string(),
        Err(_) => format!(
            "timed out after {}ms",
            OPPORTUNITY_HISTORY_APPEND_TIMEOUT.as_millis()
        ),
    };
    warn!(%error, "opportunity history append failed; degrading without blocking publish");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_samples_without_slowing_live_scans() {
        let mut schedule = OpportunityHistorySchedule::default();

        assert!(schedule.is_due(1_000));
        schedule.record_success(1_000);
        assert!(!schedule.is_due(30_999));
        assert!(schedule.is_due(31_000));
    }
}
