use super::*;
use std::collections::HashSet;

pub(crate) fn stream_event(input: OpportunityStreamEventInput<'_>) -> OpportunityStreamEvent {
    let main_p0_counts = p0_count_breakdown(input.source_rows);
    let registry_counts = registry_count_breakdown(input.source_rows);
    let mut meta = input.meta;
    let partial_failures = partial_failures(&meta);
    let observed_at_ms = Utc::now().timestamp_millis();
    let snapshot = classify_snapshot(input.status, &input.cached_at, observed_at_ms, meta.scan_ms);
    let status = envelope_status(snapshot.status, partial_failures.is_empty());
    let error = snapshot.problem.or(input.error);
    let retry_after_ms = retry_after_ms(input.retry_after_ms, &partial_failures, error.as_ref());
    meta.funding_row_evidence.clear();
    meta.market_data_status = None;
    let snapshot_id = input
        .snapshot_id
        .map(str::to_owned)
        .unwrap_or_else(|| snapshot_id(input.cached_at, &meta));
    let windows = stream_windows(input.source_rows, &meta, &snapshot_id, observed_at_ms);
    let top_ids = windows
        .first()
        .map(|window| window.ids.clone())
        .unwrap_or_default();
    let scope_meta = windows
        .first()
        .map(|window| window.scope_meta.clone())
        .unwrap_or_default();
    let mut event = OpportunityStreamEvent {
        event: OpportunityStreamEventKind::SnapshotInvalidated,
        snapshot_id,
        scope_meta,
        changed_ids: Vec::new(),
        changed_rows: Vec::new(),
        removed_ids: Vec::new(),
        top_ids,
        windows,
        main_p0_counts,
        registry_counts,
        meta,
        status,
        scope: input.scope,
        query_key: input.query_key,
        source: input.source.to_owned(),
        cached_at: input.cached_at,
        observed_at_ms,
        freshness_ms: Some(snapshot.freshness_ms),
        retry_after_ms,
        error,
        partial_failures,
    };
    if input.full_window_rows {
        event.changed_ids = stream_window_ids(&event);
        event.changed_rows =
            stream_rows_for_ids(input.source_rows, &event.changed_ids, event.observed_at_ms);
    }
    event
}

fn stream_windows(
    rows: &[ArbitrageOpportunityDto],
    meta: &OpportunityScanMeta,
    snapshot_id: &str,
    now_ms: i64,
) -> Vec<shared_types::OpportunityStreamWindow> {
    let (main_scope_count, strategy_scope_counts) = stream_scope_counts(rows);
    let mut ranked = rows
        .iter()
        .filter(|row| row.strategy_kind.is_some_and(is_p0_executable_strategy))
        .filter(|row| is_product_visible_row(row, now_ms))
        .collect::<Vec<_>>();
    sort_refs(
        ranked.as_mut_slice(),
        OpportunityListSortKey::NetSingleYield,
        now_ms,
    );
    let mut windows = Vec::with_capacity(P0_EXECUTABLE_STRATEGY_KINDS.len() + 1);
    windows.push(stream_window(
        rows,
        &ranked,
        meta,
        snapshot_id,
        None,
        main_scope_count,
    ));
    windows.extend(
        P0_EXECUTABLE_STRATEGY_KINDS
            .iter()
            .copied()
            .zip(strategy_scope_counts)
            .map(|(strategy, scope_count)| {
                stream_window(
                    rows,
                    &ranked,
                    meta,
                    snapshot_id,
                    Some(strategy),
                    scope_count,
                )
            }),
    );
    windows
}

fn stream_scope_counts(rows: &[ArbitrageOpportunityDto]) -> (usize, Vec<usize>) {
    let mut main = 0usize;
    let mut strategies = vec![0usize; P0_EXECUTABLE_STRATEGY_KINDS.len()];
    for row in rows {
        let Some(strategy) = row.strategy_kind else {
            continue;
        };
        let Some(index) = P0_EXECUTABLE_STRATEGY_KINDS
            .iter()
            .position(|candidate| *candidate == strategy)
        else {
            continue;
        };
        main = main.saturating_add(1);
        strategies[index] = strategies[index].saturating_add(1);
    }
    (main, strategies)
}

fn stream_window(
    rows: &[ArbitrageOpportunityDto],
    ranked_rows: &[&ArbitrageOpportunityDto],
    meta: &OpportunityScanMeta,
    snapshot_id: &str,
    strategy: Option<StrategyKind>,
    strategy_scope_count: usize,
) -> shared_types::OpportunityStreamWindow {
    let mut filtered_count = 0usize;
    let mut ids = Vec::with_capacity(OPPORTUNITY_PRODUCT_PAGE_SIZE);
    for row in ranked_rows
        .iter()
        .copied()
        .filter(|row| strategy.is_none_or(|selected| row.strategy_kind == Some(selected)))
    {
        filtered_count = filtered_count.saturating_add(1);
        if ids.len() < OPPORTUNITY_PRODUCT_PAGE_SIZE {
            ids.push(row.id.clone());
        }
    }
    let filter_key = stream_filter_key(strategy);
    let cursor_scope = list_cursor_scope(&filter_key);
    let window = OpportunityListWindow::from_bound_query(
        Some(OPPORTUNITY_PRODUCT_PAGE_SIZE),
        None,
        Some("net_single_yield"),
        &cursor_scope,
    );
    let page = window.page(filtered_count, ids.len(), snapshot_id.to_owned());
    let scope_meta = scope_meta(
        rows,
        strategy_scope_count,
        filtered_count,
        Some(strategy_scope_count),
        meta,
        &page,
    );
    shared_types::OpportunityStreamWindow {
        strategy_kind: strategy,
        ids,
        page,
        scope_meta,
        query_key: format!(
            "{filter_key};pageSize={};cursor=0;sortKey=NetSingleYield;fast=false;fresh=false",
            OPPORTUNITY_PRODUCT_PAGE_SIZE
        ),
    }
}

fn stream_filter_key(strategy: Option<StrategyKind>) -> String {
    let strategies = strategy.map_or_else(
        || {
            P0_EXECUTABLE_STRATEGY_KINDS
                .iter()
                .map(|kind| kind.as_query_value())
                .collect::<Vec<_>>()
                .join(",")
        },
        |kind| kind.as_query_value().to_owned(),
    );
    format!("scope=main_p0;strategy={strategies};symbol=*;minYield=*")
}

pub(crate) fn stream_window_ids(event: &OpportunityStreamEvent) -> Vec<String> {
    let mut seen = HashSet::new();
    event
        .windows
        .iter()
        .flat_map(|window| window.ids.iter())
        .chain(event.top_ids.iter())
        .filter(|id| seen.insert(id.as_str()))
        .cloned()
        .collect()
}

pub(crate) fn stream_rows_for_ids(
    rows: &[ArbitrageOpportunityDto],
    ids: &[String],
    observed_at_ms: i64,
) -> Vec<OpportunityListRow> {
    let ids = ids.iter().map(String::as_str).collect::<HashSet<_>>();
    rows.iter()
        .filter(|row| ids.contains(row.id.as_str()))
        .map(|row| list_row_from_dto_at(row, observed_at_ms))
        .collect()
}
