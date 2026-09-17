use shared_types::MissedOpportunity;

pub fn filter_recent_missed(
    rows: &[MissedOpportunity],
    now_ms: i64,
    days: u32,
) -> Vec<MissedOpportunity> {
    let min_ms = now_ms - i64::from(days.max(1)) * 24 * 60 * 60_000;
    rows.iter()
        .filter(|row| row.detected_at_ms >= min_ms)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{MissReason, StrategyKind};

    #[test]
    fn filters_old_misses() {
        let now_ms = 3 * 24 * 60 * 60_000;
        let rows = vec![miss(0), miss(now_ms)];
        let recent = filter_recent_missed(&rows, now_ms, 1);

        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].detected_at_ms, now_ms);
    }

    fn miss(detected_at_ms: i64) -> MissedOpportunity {
        MissedOpportunity {
            id: "m".into(),
            opportunity_id: "o".into(),
            strategy: StrategyKind::PerpCross,
            symbol: "BTC".into(),
            detected_at_ms,
            expected_pnl_usd: 1.0,
            reason: MissReason::ManualSkip,
            detail: "skip".into(),
        }
    }
}
