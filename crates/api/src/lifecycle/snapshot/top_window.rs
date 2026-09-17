use std::collections::{BTreeSet, HashMap};

use super::*;

#[derive(Default)]
pub(super) struct TopWindowState {
    pub(super) ids: Vec<String>,
    pub(super) rows: HashMap<String, shared_types::OpportunityListRow>,
}

pub(super) fn snapshot_notice_if_subscribed(
    hub: &realtime::WsHub,
    report: &shared_types::OpportunityScanReport,
    cached_at: chrono::DateTime<chrono::Utc>,
    snapshot_id: &str,
    top_window: &mut TopWindowState,
) -> Option<shared_types::OpportunityStreamEvent> {
    (hub.subscriber_count(realtime::channels::ARBITRAGE) > 0)
        .then(|| snapshot_notice(report, cached_at, snapshot_id, top_window))
}

fn snapshot_notice(
    report: &shared_types::OpportunityScanReport,
    cached_at: chrono::DateTime<chrono::Utc>,
    snapshot_id: &str,
    top_window: &mut TopWindowState,
) -> shared_types::OpportunityStreamEvent {
    let mut event = opportunity::stream_event(opportunity::OpportunityStreamEventInput {
        source_rows: &report.opportunities,
        meta: report.meta.clone(),
        cached_at,
        snapshot_id: Some(snapshot_id),
        source: "snapshot",
        status: shared_types::OpportunityEnvelopeStatus::Fresh,
        scope: shared_types::OpportunityEnvelopeScope::MainP0,
        query_key: main_p0_snapshot_query_key(),
        retry_after_ms: None,
        error: None,
        full_window_rows: false,
    });
    let window_ids = opportunity::stream_window_ids(&event);
    let top_rows =
        opportunity::stream_rows_for_ids(&report.opportunities, &window_ids, event.observed_at_ms);
    let rows = top_rows
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();
    apply_top_window_delta(&mut event, rows, top_window);
    event
}

pub(super) fn apply_top_window_delta(
    event: &mut shared_types::OpportunityStreamEvent,
    current_rows: HashMap<String, shared_types::OpportunityListRow>,
    top_window: &mut TopWindowState,
) {
    let previous = top_window.ids.iter().collect::<BTreeSet<_>>();
    let current_ids = opportunity::stream_window_ids(event);
    let current = current_ids.iter().collect::<BTreeSet<_>>();
    event.changed_ids = current_ids
        .iter()
        .filter(|id| top_row_changed(id, &previous, &top_window.rows, &current_rows))
        .cloned()
        .collect();
    event.removed_ids = top_window
        .ids
        .iter()
        .filter(|id| !current.contains(id))
        .cloned()
        .collect();
    event.changed_rows = event
        .changed_ids
        .iter()
        .filter_map(|id| current_rows.get(id).cloned())
        .collect();
    top_window.ids = current_ids;
    top_window.rows = current_rows;
}

fn top_row_changed(
    id: &String,
    previous_ids: &BTreeSet<&String>,
    previous_rows: &HashMap<String, shared_types::OpportunityListRow>,
    current_rows: &HashMap<String, shared_types::OpportunityListRow>,
) -> bool {
    if !previous_ids.contains(id) {
        return true;
    }
    match (previous_rows.get(id), current_rows.get(id)) {
        (Some(previous), Some(current)) => {
            product_row_state(previous) != product_row_state(current)
        }
        _ => true,
    }
}

#[derive(PartialEq)]
struct ProductRowState<'a> {
    id: &'a str,
    symbol: &'a str,
    strategy_kind: Option<shared_types::StrategyKind>,
    strategy_category: Option<shared_types::StrategyCategory>,
    type_label: &'a str,
    spot_leg_mode: Option<shared_types::SpotLegMode>,
    long_leg: &'a shared_types::OpportunityListLeg,
    short_leg: &'a shared_types::OpportunityListLeg,
    metrics: &'a shared_types::OpportunityListMetrics,
    cost: &'a shared_types::OpportunityListCost,
    execution: &'a shared_types::OpportunityListExecution,
    data_source: &'a str,
}

fn product_row_state(row: &shared_types::OpportunityListRow) -> ProductRowState<'_> {
    ProductRowState {
        id: &row.id,
        symbol: &row.symbol,
        strategy_kind: row.strategy_kind,
        strategy_category: row.strategy_category,
        type_label: &row.type_label,
        spot_leg_mode: row.spot_leg_mode,
        long_leg: &row.long_leg,
        short_leg: &row.short_leg,
        metrics: &row.metrics,
        cost: &row.cost,
        execution: &row.execution,
        data_source: &row.data_source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opportunity_delta_is_not_materialized_without_appws_subscribers() {
        let hub = realtime::WsHub::new(8);
        let report = shared_types::OpportunityScanReport::default();
        let cached_at = chrono::Utc::now();
        let mut top_window = TopWindowState {
            ids: vec!["previous".into()],
            rows: HashMap::new(),
        };

        assert!(snapshot_notice_if_subscribed(
            &hub,
            &report,
            cached_at,
            "published-snapshot",
            &mut top_window,
        )
        .is_none());
        assert_eq!(top_window.ids, ["previous"]);

        let _receiver = hub.subscribe(realtime::channels::ARBITRAGE);
        assert!(snapshot_notice_if_subscribed(
            &hub,
            &report,
            cached_at,
            "published-snapshot",
            &mut top_window,
        )
        .is_some());
        assert!(top_window.ids.is_empty());
    }

    #[test]
    fn opportunity_delta_keeps_the_published_snapshot_identity() -> Result<(), String> {
        let hub = realtime::WsHub::new(8);
        let _receiver = hub.subscribe(realtime::channels::ARBITRAGE);
        let report = shared_types::OpportunityScanReport::default();
        let cached_at = chrono::DateTime::from_timestamp_millis(1_785_000_000_123)
            .ok_or_else(|| "valid cached timestamp".to_owned())?;

        let event = snapshot_notice_if_subscribed(
            &hub,
            &report,
            cached_at,
            "authoritative-snapshot",
            &mut TopWindowState::default(),
        )
        .ok_or_else(|| "subscribed stream event".to_owned())?;

        assert_eq!(event.cached_at, cached_at);
        assert_eq!(event.snapshot_id, "authoritative-snapshot");
        Ok(())
    }

    #[test]
    fn product_row_state_ignores_projection_timestamp_only_changes() {
        let first = row_at(chrono::Utc::now());
        let mut later = first.clone();
        later.updated_at += chrono::Duration::seconds(1);

        assert!(product_row_state(&first) == product_row_state(&later));
    }

    #[test]
    fn product_row_state_detects_evidence_and_blocker_changes() {
        let first = row_at(chrono::Utc::now());
        let mut changed = first.clone();
        changed.execution.blockers.push("行情证据过期".into());

        assert!(product_row_state(&first) != product_row_state(&changed));

        let mut changed = first.clone();
        changed.metrics.settlement_countdown_seconds = Some(0);

        assert!(product_row_state(&first) != product_row_state(&changed));
    }

    fn row_at(updated_at: chrono::DateTime<chrono::Utc>) -> shared_types::OpportunityListRow {
        shared_types::OpportunityListRow {
            id: "perp_cross_binance_okx_BTC".into(),
            symbol: "BTC".into(),
            strategy_kind: Some(shared_types::StrategyKind::PerpCross),
            strategy_category: Some(shared_types::StrategyCategory::Futures),
            type_label: "永续跨所".into(),
            spot_leg_mode: None,
            long_leg: leg("binance", 100.0),
            short_leg: leg("okx", 101.0),
            metrics: shared_types::OpportunityListMetrics {
                score: 80.0,
                risk_level: shared_types::RiskLevel::Low,
                net_single_yield: 0.001,
                annualized_funding_bps: Some(100.0),
                one_cycle_net_bps: Some(5.0),
                time_to_settlement_ms: 1_000,
                settlement_countdown_seconds: Some(1),
                liquidity_score: 90.0,
            },
            cost: shared_types::OpportunityListCost {
                verified: true,
                total_cost_bps: 5.0,
                one_cycle_net_bps: Some(5.0),
                ..Default::default()
            },
            execution: shared_types::OpportunityListExecution {
                eligible: true,
                blockers: Vec::new(),
                optimal_position: 1_000.0,
                max_position: 2_000.0,
            },
            data_source: "ws_push".into(),
            updated_at,
        }
    }

    fn leg(venue: &str, price: f64) -> shared_types::OpportunityListLeg {
        shared_types::OpportunityListLeg {
            venue: venue.into(),
            action: format!("{venue} perp"),
            price: Some(price),
            market_evidence: None,
            funding: None,
        }
    }
}
