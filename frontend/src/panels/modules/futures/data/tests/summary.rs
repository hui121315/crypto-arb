use super::super::*;
use super::support::row;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use shared_types::StrategyKind;
use std::sync::Arc;

#[test]
fn summary_keeps_ticket_bound_spot_perp_profit_visible_when_build_is_ready() {
    let mut candidate = row(1, StrategyKind::SpotPerp, 12.0, 50_000.0);
    let candidate_mut = Arc::make_mut(&mut candidate);
    candidate_mut.one_cycle_net_bps = 26.4;
    candidate_mut.one_cycle_net = "+0.264%".into();
    candidate_mut.execution_eligible = true;
    candidate_mut.execution_blockers =
        vec![shared_types::DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned()];

    let summary = summarize_rows(
        &[candidate],
        &FuturesFilter {
            strategy: StrategyFilter::SpotPerp,
            ..FuturesFilter::default()
        },
        &OpportunityCountMeta::default(),
    );

    assert_eq!(summary.executable_candidates, 1);
    assert_eq!(summary.best_monitor_net_bps, Some(26.4));
    assert_eq!(summary.best_monitor_pair, "SYM1");
    assert!(summary.best_monitor_preview_ready);
}
