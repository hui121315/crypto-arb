use super::super::*;
use super::support::*;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::StrategyKind;
use std::sync::Arc;

#[test]
fn pr_dx_shared_opportunity_runtime_survives_futures_remount() {
    Owner::new().with(|| {
        let opportunities =
            crate::panels::modules::opportunities::data::create_opportunities_runtime();
        let runtime = create_futures_runtime(opportunities);
        runtime.filter.update(|filter| filter.query = "sym1".into());
        let projected = Arc::clone(&row(1, StrategyKind::PerpCross, 12.0, 50_000.0).view);
        runtime.search.rows.set(vec![projected]);
        runtime.search.page.set(Some(page("snapshot-search")));
        runtime.search.state.set(LoadState::Ready(()));
        runtime.search.last_query.set("SYM1".into());
        runtime.search.cursor.set(Some("cursor-next".into()));

        let remounted_runtime = runtime;

        assert_eq!(remounted_runtime.filter.get_untracked().query, "sym1");
        assert_eq!(opportunities.search.rows.get_untracked().len(), 1);
        assert_eq!(remounted_runtime.search.rows.get_untracked().len(), 1);
        assert_eq!(
            remounted_runtime
                .search
                .page
                .get_untracked()
                .as_ref()
                .map(|page| page.snapshot_id.as_str()),
            Some("snapshot-search")
        );
        assert!(matches!(
            remounted_runtime.search.state.get_untracked(),
            LoadState::Ready(())
        ));
        assert_eq!(
            remounted_runtime.search.last_query.get_untracked().as_str(),
            "SYM1"
        );
        assert_eq!(
            remounted_runtime.search.cursor.get_untracked().as_deref(),
            Some("cursor-next")
        );
    });
}

#[test]
fn stale_strategy_and_page_results_are_rejected() {
    assert!(futures_list_request_is_current(
        StrategyKind::PerpCross,
        None,
        StrategyKind::PerpCross,
        None,
    ));
    assert!(!futures_list_request_is_current(
        StrategyKind::PerpCross,
        None,
        StrategyKind::SpotPerp,
        None,
    ));
    assert!(!futures_list_request_is_current(
        StrategyKind::PerpCross,
        Some("page-1"),
        StrategyKind::PerpCross,
        Some("page-2"),
    ));
}
