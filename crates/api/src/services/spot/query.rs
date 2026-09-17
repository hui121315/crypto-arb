use crate::services::instrument_registry::InstrumentRegistry;
use shared_types::problem::codes;
use shared_types::{
    ApiProblem, InstrumentCoverageEntry, ListPage, MarketDataCoverage, MarketDataQuality,
    MarketDataRowEvidence, RowCapEvidence, SpotTick, SpotTicksPage, SpotTicksQuery,
};
use std::collections::{BTreeSet, HashMap};

pub(super) const SPOT_TICKS_MAX_LIMIT: usize = 128;
const SPOT_TICKS_DEFAULT_LIMIT: usize = 64;
const SPOT_TICKS_SOURCE: &str = "spot-v1-diagnostics";

pub(super) struct FilteredSpotTicks {
    pub(super) data: SpotTicksPage,
    pub(super) row_evidence: Vec<MarketDataRowEvidence>,
    pub(super) row_cap: RowCapEvidence,
    pub(super) coverage: MarketDataCoverage,
}

struct SpotTickRow {
    tick: SpotTick,
    evidence: Option<MarketDataRowEvidence>,
}

#[derive(Clone)]
struct SpotSymbolParts {
    compact: String,
    base: String,
    quote: Option<String>,
}

struct SpotTickFilter {
    symbol: Option<SpotSymbolParts>,
    base: Option<String>,
    quote: Option<String>,
    venue: Option<String>,
    fresh_only: bool,
}

impl From<&SpotTicksQuery> for SpotTickFilter {
    fn from(query: &SpotTicksQuery) -> Self {
        Self {
            symbol: query.symbol.as_deref().and_then(spot_symbol_parts),
            base: query
                .base
                .as_deref()
                .and_then(spot_symbol_parts)
                .map(|parts| parts.base),
            quote: query
                .quote
                .as_deref()
                .map(symbol_identity)
                .filter(|value| !value.is_empty()),
            venue: query
                .venue
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase),
            fresh_only: query.fresh_only,
        }
    }
}

struct SpotTickWindow {
    limit: usize,
    offset: usize,
    problems: Vec<ApiProblem>,
}

impl SpotTickWindow {
    fn from_query(query: &SpotTicksQuery) -> Self {
        let mut problems = Vec::new();
        let limit = match query.limit {
            None => SPOT_TICKS_DEFAULT_LIMIT,
            Some(0) => {
                problems.push(query_problem(
                    codes::LIST_LIMIT_CLAMPED,
                    "spot tick limit was raised to minimum",
                    serde_json::json!({ "field": "limit", "requested": 0, "applied": 1 }),
                ));
                1
            }
            Some(limit) if limit > SPOT_TICKS_MAX_LIMIT => {
                problems.push(query_problem(
                    codes::LIST_LIMIT_CLAMPED,
                    "spot tick limit was clamped to maximum",
                    serde_json::json!({
                        "field": "limit",
                        "requested": limit,
                        "applied": SPOT_TICKS_MAX_LIMIT,
                        "maxLimit": SPOT_TICKS_MAX_LIMIT,
                    }),
                ));
                SPOT_TICKS_MAX_LIMIT
            }
            Some(limit) => limit,
        };
        let offset = match query
            .cursor
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            None => 0,
            Some(cursor) => {
                if let Ok(offset) = cursor.parse::<usize>() {
                    offset
                } else {
                    problems.push(query_problem(
                        codes::LIST_CURSOR_INVALID,
                        "spot tick cursor was invalid",
                        serde_json::json!({ "cursor": cursor, "applied": 0 }),
                    ));
                    0
                }
            }
        };
        Self {
            limit,
            offset,
            problems,
        }
    }
}

pub(super) fn apply_query(
    ticks: &[SpotTick],
    row_evidence: Vec<MarketDataRowEvidence>,
    query: &SpotTicksQuery,
) -> FilteredSpotTicks {
    let mut evidence_by_key = row_evidence
        .into_iter()
        .map(|evidence| ((evidence.venue.clone(), evidence.symbol.clone()), evidence))
        .collect::<HashMap<_, _>>();
    let filter = SpotTickFilter::from(query);
    let mut rows = ticks
        .iter()
        .cloned()
        .map(|tick| {
            let evidence = evidence_by_key.remove(&(tick.venue.clone(), tick.symbol.clone()));
            SpotTickRow { tick, evidence }
        })
        .filter(|row| matches_filter(row, &filter))
        .collect::<Vec<_>>();

    let matching_count = rows.len();
    let fresh_count = rows.iter().filter(|row| is_fresh(row)).count();
    if filter.fresh_only {
        rows.retain(is_fresh);
    }
    rows.sort_by(|left, right| {
        left.tick
            .venue
            .cmp(&right.tick.venue)
            .then_with(|| left.tick.symbol.cmp(&right.tick.symbol))
            .then_with(|| right.tick.received_at_ms.cmp(&left.tick.received_at_ms))
    });

    let total_rows = rows.len();
    let window = SpotTickWindow::from_query(query);
    let returned = rows
        .into_iter()
        .skip(window.offset)
        .take(window.limit)
        .collect::<Vec<_>>();
    let row_evidence = returned
        .iter()
        .filter_map(|row| row.evidence.clone())
        .collect::<Vec<_>>();
    let ticks = returned.into_iter().map(|row| row.tick).collect::<Vec<_>>();
    let returned_count = ticks.len();
    let next_offset = window.offset.saturating_add(returned_count);
    let has_more = next_offset < total_rows;
    let page = ListPage {
        limit: window.limit,
        max_limit: SPOT_TICKS_MAX_LIMIT,
        start_offset: window.offset,
        returned_count,
        total_rows,
        has_more,
        next_cursor: has_more.then(|| next_offset.to_string()),
        ..ListPage::default()
    };
    let received = if filter.fresh_only {
        fresh_count
    } else {
        matching_count
    };

    FilteredSpotTicks {
        row_cap: RowCapEvidence::exact(window.limit, returned_count, total_rows, SPOT_TICKS_SOURCE),
        coverage: MarketDataCoverage::new(matching_count as u64, received as u64),
        data: SpotTicksPage {
            ticks,
            page,
            base_listing_coverage: Vec::new(),
            query_problems: window.problems,
            request_id: common::request_id::current(),
        },
        row_evidence,
    }
}

pub(super) fn base_listing_coverage(
    registry: &InstrumentRegistry,
    ticks: &[SpotTick],
    query: &SpotTicksQuery,
    now_ms: i64,
) -> Vec<InstrumentCoverageEntry> {
    let mut bases = ticks
        .iter()
        .filter_map(|tick| spot_symbol_parts(&tick.symbol).map(|parts| parts.base))
        .collect::<BTreeSet<_>>();
    if let Some(base) = query.base.as_deref().and_then(spot_symbol_parts) {
        bases.insert(base.base);
    }
    if let Some(symbol) = query.symbol.as_deref().and_then(spot_symbol_parts) {
        bases.insert(symbol.base);
    }
    bases
        .into_iter()
        .map(|base| registry.coverage(&base, now_ms))
        .collect()
}

fn matches_filter(row: &SpotTickRow, filter: &SpotTickFilter) -> bool {
    let parts = spot_symbol_parts(&row.tick.symbol);
    filter.symbol.as_ref().is_none_or(|symbol| {
        parts.as_ref().is_some_and(|candidate| {
            candidate.compact == symbol.compact
                || (symbol.quote.is_none() && candidate.base == symbol.base)
        })
    }) && filter.base.as_ref().is_none_or(|base| {
        parts
            .as_ref()
            .is_some_and(|candidate| candidate.base == *base)
    }) && filter.quote.as_ref().is_none_or(|quote| {
        parts
            .as_ref()
            .and_then(|candidate| candidate.quote.as_ref())
            .is_some_and(|candidate| candidate == quote)
    }) && filter
        .venue
        .as_ref()
        .is_none_or(|venue| row.tick.venue.eq_ignore_ascii_case(venue))
        && (!filter.fresh_only || is_fresh(row))
}

fn is_fresh(row: &SpotTickRow) -> bool {
    row.evidence
        .as_ref()
        .is_some_and(|evidence| evidence.health.quality == MarketDataQuality::Fresh)
}

fn spot_symbol_parts(value: &str) -> Option<SpotSymbolParts> {
    let compact = symbol_identity(value);
    if compact.is_empty() {
        return None;
    }
    if let Some((raw_base, raw_quote)) = delimited_spot_pair(value) {
        let base = symbol_identity(raw_base);
        let quote = symbol_identity(raw_quote);
        if !base.is_empty() && !quote.is_empty() {
            return Some(SpotSymbolParts {
                compact,
                base,
                quote: Some(quote),
            });
        }
    }
    let base = symbol_identity(&exchange::strip_common_suffixes(value.trim()));
    let base = if base.is_empty() {
        compact.clone()
    } else {
        base
    };
    let quote = compact
        .strip_prefix(&base)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some(SpotSymbolParts {
        compact,
        base,
        quote,
    })
}

fn delimited_spot_pair(value: &str) -> Option<(&str, &str)> {
    let pair = value
        .split_once('/')
        .or_else(|| value.split_once(':'))
        .or_else(|| value.split_once('-'))
        .or_else(|| value.split_once('_'))?;
    (!pair.0.is_empty()
        && !pair.1.is_empty()
        && !pair
            .1
            .bytes()
            .any(|byte| matches!(byte, b'/' | b':' | b'-' | b'_')))
    .then_some(pair)
}

pub(crate) fn split_spot_pair(value: &str) -> Option<(String, String)> {
    let parts = spot_symbol_parts(value)?;
    Some((parts.base, parts.quote?))
}

fn symbol_identity(value: &str) -> String {
    value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| char::from(byte.to_ascii_uppercase()))
        .collect()
}

fn query_problem(code: &str, message: &str, details: serde_json::Value) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message)
        .with_request_id(common::request_id::current())
        .with_source(SPOT_TICKS_SOURCE);
    problem.details = Some(details);
    problem
}

#[cfg(test)]
mod tests;
