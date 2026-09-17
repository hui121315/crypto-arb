use super::super::*;
use super::support::*;
use crate::state::load_state::LoadState;
use leptos::prelude::*;

#[test]
fn opportunity_symbol_search_local_state_survives_runtime_remount() {
    Owner::new().with(|| {
        let runtime = create_opportunities_runtime();
        runtime.filter.update(|filter| filter.query = "mu".into());
        runtime
            .search
            .rows
            .set(vec![row_ref("1", "MU", "gate", true)]);
        runtime.search.page.set(Some(page("snapshot-search")));
        runtime.search.state.set(LoadState::Ready(()));
        runtime.search.last_query.set("MU".into());
        runtime.search.cursor.set(Some("cursor-next".into()));

        let remounted_runtime = runtime;

        assert_eq!(remounted_runtime.filter.get_untracked().query, "mu");
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
            "MU"
        );
        assert_eq!(
            remounted_runtime.search.cursor.get_untracked().as_deref(),
            Some("cursor-next")
        );
    });
}
