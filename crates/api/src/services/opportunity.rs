use chrono::{DateTime, Utc};
use shared_types::arbitrage::OpportunityListRequestMeta;
use shared_types::contracts::p0::{
    OpportunityCountBreakdown, OpportunityEnvelopeScope, OpportunityEnvelopeStatus,
    OpportunityListCost, OpportunityListEnvelope, OpportunityListExecution, OpportunityListLeg,
    OpportunityListLegFunding, OpportunityListMetrics, OpportunityListPage, OpportunityListRow,
    OpportunityListSortKey, OpportunityQueryScopeMeta, OpportunityScanMeta, OpportunityStreamEvent,
    OpportunityStreamEventKind, OPPORTUNITY_PRODUCT_PAGE_SIZE,
};
use shared_types::{
    is_hedge_preview_ready, is_p0_executable_strategy, market_monitor_net_bps_at,
    opportunity_build_blockers_allow_preflight, p0_hedge_leg_product, problem::codes, ApiProblem,
    ArbitrageOpportunityDto, ExecutionCostProfile, FeeProduct, HedgeLegRole, MarketDataQuality,
    MarketDataSnapshotStatusRow, MarketDataSourceKind, OpportunityEnvelope,
    OpportunityLegMarketEvidence, OpportunityScanReport, StrategyKind, FUNDING_WS_EVIDENCE_BLOCKER,
    HEDGE_PREVIEW_MARKET_MAX_AGE_MS, P0_EXECUTABLE_STRATEGY_KINDS,
};
use std::collections::HashMap;
use tracing::debug;

pub(crate) const WARMING_RETRY_AFTER_MS: u64 =
    crate::services::snapshots::ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS;
pub(crate) const DEFAULT_LIST_PAGE_SIZE: usize = 120;
pub(crate) const MAX_LIST_PAGE_SIZE: usize = 120;
pub(crate) const DEFAULT_WIDE_LIMIT: usize = 500;
pub(crate) const MAX_WIDE_LIMIT: usize = 500;

const WIDE_LIST_SOURCE: &str = "arbitrage-opportunities";
const DUPLICATE_ID_BLOCKER: &str = "机会身份重复，无法证明唯一交易标的，仅观察不执行";
const MARKET_DATA_WARMING_CODE: &str = "MARKET_DATA_WARMING";

mod builders;
mod paging;
mod row;
mod snapshot_health;
#[cfg(test)]
mod tests;

pub(crate) use builders::{
    envelope, legacy_wide_endpoint_problem, list_envelope, p0_strategy_kind, stream_event,
    stream_rows_for_ids, stream_window_ids, warming_error,
};
pub(crate) use paging::{
    list_cursor_scope, page_refs, snapshot_id, wide_limit, OpportunityListWindow,
    OpportunityWideLimit,
};
#[cfg(test)]
pub(crate) use row::list_row_from_dto;
pub(crate) use row::sort_refs;
#[cfg(test)]
pub(crate) use tests::counts;

pub(crate) fn normalize_scan_report(
    report: &mut OpportunityScanReport,
    scan_started_at: DateTime<Utc>,
) -> usize {
    report.meta.scan_started_at = Some(scan_started_at);
    deduplicate_opportunity_ids(&mut report.opportunities)
}

fn deduplicate_opportunity_ids(rows: &mut Vec<ArbitrageOpportunityDto>) -> usize {
    let mut indexes = HashMap::with_capacity(rows.len());
    let mut unique = Vec::with_capacity(rows.len());
    let mut duplicate_count = 0usize;
    for row in rows.drain(..) {
        if let Some(index) = indexes.get(&row.id).copied() {
            duplicate_count = duplicate_count.saturating_add(1);
            let existing: &mut ArbitrageOpportunityDto = &mut unique[index];
            debug!(
                opportunity_id = %row.id,
                existing_long_symbol = ?existing.long_leg_market_evidence.as_ref().map(|value| value.symbol.as_str()),
                existing_short_symbol = ?existing.short_leg_market_evidence.as_ref().map(|value| value.symbol.as_str()),
                duplicate_long_symbol = ?row.long_leg_market_evidence.as_ref().map(|value| value.symbol.as_str()),
                duplicate_short_symbol = ?row.short_leg_market_evidence.as_ref().map(|value| value.symbol.as_str()),
                existing_long_price = ?existing.long_price,
                existing_short_price = ?existing.short_price,
                duplicate_long_price = ?row.long_price,
                duplicate_short_price = ?row.short_price,
                "duplicate opportunity identity evidence"
            );
            existing.execution_eligible = false;
            if !existing
                .execution_blockers
                .iter()
                .any(|blocker| blocker == DUPLICATE_ID_BLOCKER)
            {
                existing
                    .execution_blockers
                    .push(DUPLICATE_ID_BLOCKER.to_owned());
            }
            continue;
        }
        indexes.insert(row.id.clone(), unique.len());
        unique.push(row);
    }
    *rows = unique;
    duplicate_count
}

pub(crate) fn scan_required_market_status(row: &MarketDataSnapshotStatusRow) -> bool {
    // HIP-3 sub-DEXs are perp venues; Hyperliquid publishes spot mids only on the root venue.
    let builder_spot_gap = row.operation == shared_types::MarketDataSnapshotOperation::SpotTicks
        && shared_types::is_hyperliquid_builder_venue(&row.venue);
    // The global list is built from aggregate cache projections. Venue-level REST discovery
    // outcomes remain available in diagnostics, while exact candidate readiness is enforced by
    // each row's bilateral WS evidence. Promoting one discovery shard into a list-wide failure
    // makes a healthy realtime window look degraded even when the aggregate input is available.
    let aggregate_scan_input =
        row.venue == crate::services::market_data::cache::MARKET_AGGREGATE_VENUE;
    aggregate_scan_input
        && !builder_spot_gap
        && matches!(
            row.operation,
            shared_types::MarketDataSnapshotOperation::FundingRates
                | shared_types::MarketDataSnapshotOperation::PerpTickers
                | shared_types::MarketDataSnapshotOperation::SpotTicks
        )
}

pub(crate) fn scan_market_status_is_problem(row: &MarketDataSnapshotStatusRow) -> bool {
    scan_required_market_status(row)
        && row.health.quality != MarketDataQuality::Fresh
        && row
            .health
            .problem
            .as_ref()
            .is_none_or(|problem| problem.code != MARKET_DATA_WARMING_CODE)
}

pub(crate) fn has_fresh_ws_market_pair(row: &ArbitrageOpportunityDto, now_ms: i64) -> bool {
    fresh_ws_leg(row.long_leg_market_evidence.as_ref(), now_ms)
        && fresh_ws_leg(row.short_leg_market_evidence.as_ref(), now_ms)
        && !row
            .execution_blockers
            .iter()
            .any(|blocker| blocker == FUNDING_WS_EVIDENCE_BLOCKER)
        && row.quote_conversions.iter().all(|conversion| {
            conversion.rate.is_finite()
                && conversion.rate > 0.0
                && fresh_ws_leg(conversion.market_evidence.as_ref(), now_ms)
        })
}

pub(crate) fn is_product_visible_row(row: &ArbitrageOpportunityDto, now_ms: i64) -> bool {
    market_monitor_net_bps_at(row, now_ms).is_some()
}

pub(crate) fn is_ticket_build_ready(row: &ArbitrageOpportunityDto) -> bool {
    is_ticket_build_ready_at(row, common::time::now_ms())
}

fn is_ticket_build_ready_at(row: &ArbitrageOpportunityDto, now_ms: i64) -> bool {
    if is_hedge_preview_ready(row) {
        return true;
    }
    let deferred_spot_perp = matches!(
        row.strategy_kind,
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
    ) && !row.execution_blockers.is_empty()
        && opportunity_build_blockers_allow_preflight(row.strategy_kind, &row.execution_blockers);
    deferred_spot_perp
        && has_fresh_ws_market_pair(row, now_ms)
        && row.execution_cost.as_ref().is_some_and(|cost| {
            cost.one_cycle.covers_round_trip_cost
                && cost.one_cycle.net_bps.is_finite()
                && cost.one_cycle.net_bps > f64::EPSILON
                && has_verified_round_trip_cost(cost, now_ms)
                && cost
                    .round_trip
                    .as_ref()
                    .is_some_and(|round_trip| round_trip.profitability_evidence.is_cost_verified())
        })
}

fn fresh_ws_leg(evidence: Option<&OpportunityLegMarketEvidence>, now_ms: i64) -> bool {
    evidence.is_some_and(|evidence| {
        evidence.health.quality == MarketDataQuality::Fresh
            && evidence.health.source == MarketDataSourceKind::WsPush
            && evidence.health.observed_at_ms > 0
            && evidence.health.observed_at_ms <= now_ms
            && now_ms.saturating_sub(evidence.health.observed_at_ms)
                <= HEDGE_PREVIEW_MARKET_MAX_AGE_MS
    })
}

fn has_verified_round_trip_cost(cost: &ExecutionCostProfile, now_ms: i64) -> bool {
    cost.round_trip.as_ref().is_some_and(|round_trip| {
        [
            round_trip.long_leg.fee_snapshot.as_ref(),
            round_trip.short_leg.fee_snapshot.as_ref(),
        ]
        .into_iter()
        .all(|snapshot| snapshot.is_some_and(|snapshot| snapshot.is_fresh_verified(now_ms)))
    })
}

pub(crate) struct OpportunityListEnvelopeInput<'a> {
    pub rows: Vec<&'a ArbitrageOpportunityDto>,
    pub source_rows: &'a [ArbitrageOpportunityDto],
    pub request_meta: OpportunityListRequestMeta,
    pub strategy_scope_count: usize,
    pub filtered_count: usize,
    pub symbol_scope_count: Option<usize>,
    pub meta: OpportunityScanMeta,
    pub cached_at: DateTime<Utc>,
    pub snapshot_id: Option<&'a str>,
    pub source: &'a str,
    pub status: OpportunityEnvelopeStatus,
    pub scope: OpportunityEnvelopeScope,
    pub query_key: String,
    pub retry_after_ms: Option<u64>,
    pub error: Option<ApiProblem>,
    pub window: OpportunityListWindow,
}

pub(crate) struct OpportunityEnvelopeInput<'a> {
    pub opportunities: Vec<ArbitrageOpportunityDto>,
    pub source_rows: &'a [ArbitrageOpportunityDto],
    pub filtered_rows: &'a [ArbitrageOpportunityDto],
    pub meta: OpportunityScanMeta,
    pub cached_at: DateTime<Utc>,
    pub source: &'a str,
    pub status: OpportunityEnvelopeStatus,
    pub scope: OpportunityEnvelopeScope,
    pub query_key: String,
    pub retry_after_ms: Option<u64>,
    pub error: Option<ApiProblem>,
    pub query_problems: Vec<ApiProblem>,
}

pub(crate) struct OpportunityStreamEventInput<'a> {
    pub source_rows: &'a [ArbitrageOpportunityDto],
    pub meta: OpportunityScanMeta,
    pub cached_at: DateTime<Utc>,
    pub snapshot_id: Option<&'a str>,
    pub source: &'a str,
    pub status: OpportunityEnvelopeStatus,
    pub scope: OpportunityEnvelopeScope,
    pub query_key: String,
    pub retry_after_ms: Option<u64>,
    pub error: Option<ApiProblem>,
    pub full_window_rows: bool,
}
