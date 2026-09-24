use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    OnchainComparisonQuality, OnchainComparisonSnapshot, OnchainCrossChainQuality,
    OnchainDexComparisonQuality,
};

use super::super::{data::OnchainData, draft::OnchainConfigDraft};

pub(super) fn display_snapshot(
    state: &LoadState<OnchainComparisonSnapshot>,
) -> Option<OnchainComparisonSnapshot> {
    let mut snapshot = state.value()?.clone();
    if matches!(state, LoadState::Stale { .. }) {
        snapshot.quality = OnchainComparisonQuality::Stale;
        for item in &mut snapshot.batch.items {
            item.quality = OnchainComparisonQuality::Stale;
            if item.config.dex_comparison.enabled {
                item.dex_quality = OnchainDexComparisonQuality::Stale;
            }
            if item.config.cross_chain.enabled {
                item.cross_chain_quality = OnchainCrossChainQuality::Stale;
            }
        }
    }
    Some(snapshot)
}

// Resolve by identity at click time; a quote update must not retain an old config.
pub(super) fn focus_market(draft: OnchainConfigDraft, data: OnchainData, item_id: &str) -> bool {
    if data.saving.try_get_untracked() != Some(false) {
        return false;
    }
    let config = data.state.with_untracked(|state| {
        state
            .value()?
            .batch
            .items
            .iter()
            .find(|item| item.item_id == item_id)
            .map(|item| item.config.clone())
    });
    let Some(config) = config else {
        return false;
    };
    draft.load_config(&config);
    data.reset_token_states();
    data.update.run(draft.patch());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ApiProblem, OnchainBatchItemSnapshot, OnchainComparisonConfig};

    #[test]
    fn disconnected_queue_keeps_values_but_not_fresh_route_claims() {
        let config = OnchainComparisonConfig::default();
        let mut snapshot = OnchainComparisonSnapshot {
            config: config.clone(),
            ..OnchainComparisonSnapshot::default()
        };
        let mut item = OnchainBatchItemSnapshot::pending("test".into(), config, 1000, 100);
        item.quality = OnchainComparisonQuality::Fresh;
        item.best_net_spread_bps = Some(50.0);
        item.config.dex_comparison.enabled = true;
        item.dex_quality = OnchainDexComparisonQuality::Fresh;
        item.config.cross_chain.enabled = true;
        item.cross_chain_quality = OnchainCrossChainQuality::Fresh;
        snapshot.batch.items.push(item);
        let state = LoadState::Stale {
            value: snapshot,
            problem: ApiProblem::new("OFFLINE", "stream disconnected"),
        };
        let projected = display_snapshot(&state).unwrap();
        assert_eq!(projected.quality, OnchainComparisonQuality::Stale);
        let item = &projected.batch.items[0];
        assert_eq!(item.quality, OnchainComparisonQuality::Stale);
        assert_eq!(item.dex_quality, OnchainDexComparisonQuality::Stale);
        assert_eq!(item.cross_chain_quality, OnchainCrossChainQuality::Stale);
        assert_eq!(item.best_net_spread_bps, Some(50.0));
        assert_eq!(
            state.value().unwrap().batch.items[0].quality,
            OnchainComparisonQuality::Fresh
        );
    }
}
