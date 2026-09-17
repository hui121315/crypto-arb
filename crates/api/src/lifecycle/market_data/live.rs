use super::*;
use crate::services::market_subscriptions::MarketSubscriptionFeed;

mod plan;

use plan::{
    live_request_plan, opportunity_request_plan, LiveRequestPlan, VersionedOpportunityPlan,
    WS_LIVE_OTHER_ROWS, WS_LIVE_PERP_CROSS_ROWS, WS_LIVE_SYMBOLS_PER_VENUE,
};

const WS_LIVE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const WS_LIVE_FUNDING_EVERY_TICKS: u64 = 10;
const WS_LIVE_PLAN_EVERY_TICKS: u64 = 50;
const WS_SPOT_FULL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const WS_SPOT_FULL_PLAN_EVERY_TICKS: u64 = 20;

pub(super) fn spawn_live_ingest(state: &AppState, tasks: &mut BackgroundTasks) {
    let live_state = state.clone();
    let live_shutdown = tasks.shutdown_token();
    tasks.supervise(
        "market_ws_live",
        WS_LIVE_POLL_INTERVAL.as_millis() as i64,
        move || {
            let state = live_state.clone();
            let shutdown = live_shutdown.clone();
            async move { run_live_ingest(state, shutdown).await }
        },
    );

    let spot_state = state.clone();
    let spot_shutdown = tasks.shutdown_token();
    tasks.supervise(
        "market_ws_spot_full",
        WS_SPOT_FULL_POLL_INTERVAL.as_millis() as i64,
        move || {
            let state = spot_state.clone();
            let shutdown = spot_shutdown.clone();
            async move { run_full_spot_ingest(state, shutdown).await }
        },
    );

    info!(
        poll_ms = WS_LIVE_POLL_INTERVAL.as_millis() as u64,
        funding_every_ticks = WS_LIVE_FUNDING_EVERY_TICKS,
        plan_every_ticks = WS_LIVE_PLAN_EVERY_TICKS,
        perp_cross_rows = WS_LIVE_PERP_CROSS_ROWS,
        other_rows = WS_LIVE_OTHER_ROWS,
        symbols_per_venue = WS_LIVE_SYMBOLS_PER_VENUE,
        "public market WS live projection started"
    );
    info!(
        poll_ms = WS_SPOT_FULL_POLL_INTERVAL.as_millis() as u64,
        plan_every_ticks = WS_SPOT_FULL_PLAN_EVERY_TICKS,
        "full-market spot WS cache projection started"
    );
}

async fn run_live_ingest(state: AppState, shutdown: ShutdownToken) {
    let runtime = MarketDataRuntime {
        aggregator: Arc::clone(state.aggregator_handle()),
        market_data: Arc::clone(state.market_data()),
        market_subscriptions: Arc::clone(state.market_subscriptions()),
        watchlist: Arc::clone(state.watchlist()),
        watchlist_alert_store: Arc::clone(state.watchlist_alert_store()),
        hub: state.ws_hub().clone(),
    };
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(WS_LIVE_POLL_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut tick_count = 0u64;
    let mut opportunity_plan = VersionedOpportunityPlan::default();
    let mut applied_positions_version = String::new();
    let mut requests = LiveRequestPlan::default();

    while tick_or_shutdown(&mut tick, &shutdown).await {
        tick_count = tick_count.saturating_add(1);
        refresh_live_request_plan(
            &state,
            tick_count,
            &mut opportunity_plan,
            &mut applied_positions_version,
            &mut requests,
        );
        if requests.is_empty() {
            continue;
        }
        let include_funding = tick_count % WS_LIVE_FUNDING_EVERY_TICKS == 0;
        let started_at_ms = common::time::now_ms();
        let stats = ws_touch::ingest_ws_market_updates(
            &runtime,
            &requests.perp,
            &requests.spot,
            &requests.funding,
            &requests.marks,
            include_funding,
        )
        .await;
        registry.record_result_timed("market_ws_live", started_at_ms, Ok(()));
        if stats.changed_rows == 0 {
            continue;
        }
        if stats.changed_rows > stats.mark_changed_rows {
            state.request_arbitrage_refresh();
        }
        if stats.mark_changed_rows > 0 {
            state.request_portfolio_refresh();
        }
        debug!(
            requested = stats.requested,
            rows = stats.rows,
            changed_rows = stats.changed_rows,
            mark_changed_rows = stats.mark_changed_rows,
            include_funding,
            "public market WS changes queued for opportunity refresh"
        );
    }
}

async fn run_full_spot_ingest(state: AppState, shutdown: ShutdownToken) {
    let runtime = MarketDataRuntime {
        aggregator: Arc::clone(state.aggregator_handle()),
        market_data: Arc::clone(state.market_data()),
        market_subscriptions: Arc::clone(state.market_subscriptions()),
        watchlist: Arc::clone(state.watchlist()),
        watchlist_alert_store: Arc::clone(state.watchlist_alert_store()),
        hub: state.ws_hub().clone(),
    };
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(WS_SPOT_FULL_POLL_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut tick_count = 0u64;
    let mut requests = BTreeMap::new();

    while tick_or_shutdown(&mut tick, &shutdown).await {
        tick_count = tick_count.saturating_add(1);
        if tick_count == 1 || tick_count % WS_SPOT_FULL_PLAN_EVERY_TICKS == 0 {
            requests = state.instrument_registry().spot_ws_requests_by_venue();
            requests.retain(|venue, _| {
                state
                    .market_subscriptions()
                    .enabled(venue, MarketSubscriptionFeed::Spot)
            });
        }
        if requests.is_empty() {
            continue;
        }
        let started_at_ms = common::time::now_ms();
        let stats = ws_touch::ingest_ws_spot_market_updates(&runtime, &requests).await;
        registry.record_result_timed("market_ws_spot_full", started_at_ms, Ok(()));
        if stats.changed_rows > 0 {
            state.request_arbitrage_refresh();
        }
        debug!(
            venues = requests.len(),
            requested = stats.requested,
            rows = stats.rows,
            changed_rows = stats.changed_rows,
            "full-market spot WS cache projection completed"
        );
    }
}

fn refresh_live_request_plan(
    state: &AppState,
    tick_count: u64,
    opportunity_plan: &mut VersionedOpportunityPlan,
    applied_positions_version: &mut String,
    requests: &mut LiveRequestPlan,
) {
    let opportunity_version = state.opportunity_index().version();
    let positions_version = state
        .portfolio_snapshot()
        .get_arc_now()
        .map(|entry| entry.value.snapshot_version.clone())
        .unwrap_or_default();
    let opportunity_due =
        opportunity_plan_due(tick_count, opportunity_version, opportunity_plan.version);
    let opportunity_changed = if opportunity_due {
        opportunity_plan.replace(opportunity_request_plan(state))
    } else {
        false
    };
    if !live_request_plan_due(
        tick_count == 1 || opportunity_changed,
        positions_version != *applied_positions_version,
    ) {
        return;
    }
    *requests = live_request_plan(state, &opportunity_plan.requests);
    retain_enabled_requests(state, requests);
    *applied_positions_version = positions_version;
}

fn retain_enabled_requests(state: &AppState, requests: &mut LiveRequestPlan) {
    let subscriptions = state.market_subscriptions();
    requests
        .spot
        .retain(|venue, _| subscriptions.enabled(venue, MarketSubscriptionFeed::Spot));
    requests
        .perp
        .retain(|venue, _| subscriptions.enabled(venue, MarketSubscriptionFeed::Perp));
    requests
        .marks
        .retain(|venue, _| subscriptions.enabled(venue, MarketSubscriptionFeed::Perp));
    requests
        .funding
        .retain(|venue, _| subscriptions.enabled(venue, MarketSubscriptionFeed::Funding));
}

fn opportunity_plan_due(tick_count: u64, observed_version: u64, applied_version: u64) -> bool {
    tick_count == 1
        || observed_version != applied_version
        || tick_count % WS_LIVE_PLAN_EVERY_TICKS == 0
}

fn live_request_plan_due(opportunity_due: bool, positions_changed: bool) -> bool {
    opportunity_due || positions_changed
}
