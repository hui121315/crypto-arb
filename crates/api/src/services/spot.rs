mod query;

use crate::state::AppState;
use shared_types::{MarketDataEnvelope, SpotTick, SpotTicksPage, SpotTicksQuery};

pub(crate) use query::split_spot_pair;

pub(crate) async fn ticks(state: &AppState) -> Vec<SpotTick> {
    state
        .market_data()
        .spot_ticks_snapshot(state.aggregator())
        .await
}

pub(crate) async fn filtered_ticks(
    state: &AppState,
    query: &SpotTicksQuery,
) -> MarketDataEnvelope<SpotTicksPage> {
    let ticks = ticks(state).await;
    let row_evidence = state.market_data().spot_tick_row_evidence(&ticks);
    filtered_ticks_from_rows(state, &ticks, row_evidence, query)
}

pub(crate) fn filtered_ticks_cached(
    state: &AppState,
    query: &SpotTicksQuery,
) -> MarketDataEnvelope<SpotTicksPage> {
    let snapshot = state.market_data().market_snapshot_cached();
    filtered_ticks_from_rows(
        state,
        &snapshot.spot_ticks,
        snapshot.spot_tick_row_evidence,
        query,
    )
}

fn filtered_ticks_from_rows(
    state: &AppState,
    ticks: &[SpotTick],
    row_evidence: Vec<shared_types::MarketDataRowEvidence>,
    query: &SpotTicksQuery,
) -> MarketDataEnvelope<SpotTicksPage> {
    let now_ms = common::time::now_ms();
    let mut filtered = query::apply_query(ticks, row_evidence, query);
    filtered.data.base_listing_coverage = query::base_listing_coverage(
        state.instrument_registry(),
        &filtered.data.ticks,
        query,
        now_ms,
    );

    let row_count = filtered.data.ticks.len();
    crate::services::market_data::envelope::spot_ticks_envelope(
        filtered.data,
        row_count,
        &state.market_data().runtime_health_snapshot(),
        filtered.row_evidence,
        Some(filtered.row_cap),
        Some(filtered.coverage),
    )
}
