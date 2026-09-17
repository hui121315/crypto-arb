use std::collections::{BTreeMap, HashMap};

use crate::services::market_data::cache::{
    MARKET_OP_WS_TICKER_SNAPSHOT, MARKET_OP_WS_TICKER_SUBSCRIBE,
};
use crate::services::market_data::{MarketQuality, MarketRuntimeHealth};
use shared_types::{
    normalized_venue_name, venue_names_equal, ApiProblem, WatchlistItem, WatchlistItemRuntime,
    WatchlistPrewarmStatus,
};

use super::MarketDataRuntime;

pub(super) async fn refresh_watchlist_runtime(
    runtime: &MarketDataRuntime,
    planned_rows: &[WatchlistItem],
    ticker_requests: &BTreeMap<String, Vec<String>>,
    now_ms: i64,
) -> Option<shared_types::WatchlistEnvelope> {
    if planned_rows.is_empty() {
        return None;
    }
    let health = runtime.market_data.runtime_health_snapshot();
    let references = leg_reference_counts(planned_rows);
    let planned_by_id = planned_rows
        .iter()
        .map(|item| (item.id, item))
        .collect::<HashMap<_, _>>();
    let mut rows = runtime.watchlist.write().await;
    for item in rows.iter_mut() {
        let Some(planned) = planned_by_id.get(&item.id).copied() else {
            continue;
        };
        if !same_watchlist_generation(item, planned) {
            continue;
        }
        item.runtime = item_runtime(planned, ticker_requests, &health, &references, now_ms);
    }
    Some(realtime::alerts::watchlist_envelope_with_storage(
        rows.clone(),
        runtime.watchlist_alert_store.health(),
    ))
}

fn item_runtime(
    item: &WatchlistItem,
    ticker_requests: &BTreeMap<String, Vec<String>>,
    health: &[MarketRuntimeHealth],
    references: &HashMap<(String, String), usize>,
    now_ms: i64,
) -> WatchlistItemRuntime {
    if !item.enabled {
        return WatchlistItemRuntime {
            status: WatchlistPrewarmStatus::Disabled,
            last_prewarm_at_ms: Some(now_ms),
            ..WatchlistItemRuntime::default()
        };
    }
    let legs = item_legs(item);
    let requested_public_legs = legs.len();
    if requested_public_legs == 0 {
        return WatchlistItemRuntime {
            status: WatchlistPrewarmStatus::Idle,
            last_prewarm_at_ms: Some(now_ms),
            ..WatchlistItemRuntime::default()
        };
    }
    let planned_ticker_legs = legs
        .iter()
        .filter(|leg| ticker_planned(ticker_requests, leg))
        .count();
    let deduplicated_legs = legs
        .iter()
        .filter(|leg| references.get(*leg).is_some_and(|count| *count > 1))
        .count();
    let capped_legs = legs
        .iter()
        .filter(|leg| !ticker_planned(ticker_requests, leg))
        .count();
    let (health_samples, health_problem) = item_health(&legs, health, ticker_requests);
    let (status, problem) = if capped_legs > 0 {
        (
            WatchlistPrewarmStatus::Capped,
            Some(
                ApiProblem::new(
                    "WATCHLIST_PREWARM_CAPPED",
                    format!(
                        "{capped_legs} public prewarm leg(s) were excluded by the bounded plan"
                    ),
                )
                .with_source("market_prewarm"),
            ),
        )
    } else if let Some(problem) = health_problem {
        (WatchlistPrewarmStatus::Degraded, Some(problem))
    } else if health_samples > 0 {
        (WatchlistPrewarmStatus::Fresh, None)
    } else {
        (WatchlistPrewarmStatus::Planned, None)
    };
    WatchlistItemRuntime {
        status,
        requested_public_legs,
        planned_ticker_legs,
        deduplicated_legs,
        capped_legs,
        last_prewarm_at_ms: Some(now_ms),
        problem,
    }
}

fn item_health(
    legs: &[(String, String)],
    health: &[MarketRuntimeHealth],
    ticker_requests: &BTreeMap<String, Vec<String>>,
) -> (usize, Option<ApiProblem>) {
    let mut samples = 0usize;
    for leg in legs {
        if ticker_planned(ticker_requests, leg) {
            let row = ticker_health_row(health, &leg.0);
            if let Some(problem) = accumulate_health(row, &mut samples) {
                return (samples, Some(problem));
            }
        }
    }
    (samples, None)
}

fn ticker_health_row<'a>(
    health: &'a [MarketRuntimeHealth],
    venue: &str,
) -> Option<&'a MarketRuntimeHealth> {
    let find = |operation| {
        health
            .iter()
            .find(|row| venue_names_equal(&row.venue, venue) && row.operation == operation)
    };
    find(MARKET_OP_WS_TICKER_SNAPSHOT)
        .filter(|row| row.quality == MarketQuality::Fresh)
        .or_else(|| find(MARKET_OP_WS_TICKER_SNAPSHOT))
        .or_else(|| find(MARKET_OP_WS_TICKER_SUBSCRIBE))
}

fn accumulate_health(row: Option<&MarketRuntimeHealth>, samples: &mut usize) -> Option<ApiProblem> {
    if let Some(row) = row {
        *samples = samples.saturating_add(1);
        if row.quality != MarketQuality::Fresh {
            return Some(runtime_health_problem(row));
        }
    }
    None
}

fn runtime_health_problem(row: &MarketRuntimeHealth) -> ApiProblem {
    let mut problem = ApiProblem::new(
        "WATCHLIST_PREWARM_DEGRADED",
        row.last_error.clone().unwrap_or_else(|| {
            format!(
                "{} {} runtime quality is {:?}",
                row.venue, row.operation, row.quality
            )
        }),
    )
    .with_retry_after_ms(row.retry_after_ms)
    .with_source("market_prewarm");
    problem.details = Some(serde_json::json!({
        "venue": row.venue,
        "operation": row.operation,
        "requested": row.requested,
        "rows": row.rows,
    }));
    problem
}

fn leg_reference_counts(rows: &[WatchlistItem]) -> HashMap<(String, String), usize> {
    let mut counts = HashMap::new();
    for item in rows.iter().filter(|item| item.enabled) {
        for leg in item_legs(item) {
            *counts.entry(leg).or_insert(0) += 1;
        }
    }
    counts
}

fn item_legs(item: &WatchlistItem) -> Vec<(String, String)> {
    let mut legs = Vec::with_capacity(2);
    for venue in [item.venue_long.as_deref(), item.venue_short.as_deref()]
        .into_iter()
        .flatten()
    {
        let leg = (
            normalized_venue_name(venue.trim()),
            item.symbol.trim().to_ascii_uppercase(),
        );
        if !legs.iter().any(|existing| existing == &leg) {
            legs.push(leg);
        }
    }
    legs
}

fn ticker_planned(requests: &BTreeMap<String, Vec<String>>, leg: &(String, String)) -> bool {
    requests.iter().any(|(venue, symbols)| {
        venue_names_equal(venue, &leg.0)
            && symbols
                .iter()
                .any(|symbol| symbol.eq_ignore_ascii_case(&leg.1))
    })
}

fn same_watchlist_generation(current: &WatchlistItem, planned: &WatchlistItem) -> bool {
    current.id == planned.id
        && current.created_at_ms == planned.created_at_ms
        && current.symbol == planned.symbol
        && current.venue_long == planned.venue_long
        && current.venue_short == planned.venue_short
        && current.min_net_yield == planned.min_net_yield
        && current.min_volume_24h == planned.min_volume_24h
        && current.enabled == planned.enabled
}
