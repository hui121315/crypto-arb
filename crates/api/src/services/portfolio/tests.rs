use super::*;
pub(super) use common::config::AppConfig;
pub(super) use shared_types::{
    CloseRunScope, CloseRunStatus, OrderIntent, OrderSource, OrderType, StrategyKind, TimeInForce,
    VenueOperationEvidence, UNRECORDED_EVIDENCE_MARKER,
};

mod cases_a;
mod cases_b;
mod cases_c;
mod cases_quality;

pub(super) fn operation_health(status: VenueOperationStatus) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "system".into(),
        operation: "storage:portfolio_nav".into(),
        status,
        source: "portfolio_nav_store".into(),
        message: "storage".into(),
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(1),
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: Some(VenueOperationEvidence {
            method: "sqlite".into(),
            path: "portfolio_nav_samples".into(),
            checked_at: UNRECORDED_EVIDENCE_MARKER.into(),
            doc_version: UNRECORDED_EVIDENCE_MARKER.into(),
            schema_hash: UNRECORDED_EVIDENCE_MARKER.into(),
            fixture_id: UNRECORDED_EVIDENCE_MARKER.into(),
            parser_test: UNRECORDED_EVIDENCE_MARKER.into(),
            request_builder_test: UNRECORDED_EVIDENCE_MARKER.into(),
            auth_kind: UNRECORDED_EVIDENCE_MARKER.into(),
            request_id: None,
            request_context: Vec::new(),
            doc_urls: Vec::new(),
            use_cases: Vec::new(),
            data_kinds: Vec::new(),
            rate_scopes: Vec::new(),
            weight: 0,
        }),
        problem: None,
        observed_at_ms: 1,
    }
}

pub(super) async fn portfolio_test_state() -> Result<AppState, String> {
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    AppState::new(config)
        .await
        .map_err(|error| format!("state init failed: {error}"))
}

pub(super) fn position(side: &str) -> PositionInfo {
    PositionInfo {
        symbol: "BTC".into(),
        exchange: "OKX".into(),
        side: side.into(),
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 100.0,
        unrealized_pnl: 1.0,
        leverage: 2.0,
        liquidation_price: Some(80.0),
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 50.0,
        maintenance_margin_ratio: 0.01,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

pub(super) fn funding() -> FundingRateData {
    FundingRateData {
        symbol: "BTC".into(),
        exchange: "OKX".into(),
        rate: 0.0002,
        rate_8h: 0.0002,
        predicted_rate: None,
        next_funding_time: 0,
        funding_interval: 8,
        volume_24h: 1_000_000.0,
        timestamp: 1,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

pub(super) fn risk_annotation() -> RiskAnnotation {
    RiskAnnotation {
        now_ms: 0,
        warn_pct: 15.0,
        danger_pct: 8.0,
    }
}

pub(super) fn dry_order(
    id: &str,
    exchange: &str,
    symbol: &str,
    side: OrderSide,
    reduce_only: bool,
) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: id.into(),
            source: dry_order_source(reduce_only),
            strategy: Some(StrategyKind::PerpCross),
            mode: ExecutionMode::DryRun,
            exchange: exchange.into(),
            symbol: symbol.into(),
            side,
            order_type: OrderType::Limit,
            quantity: 2.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 5.0,
            client_order_id: format!("client-{id}"),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state: LiveOrderState::Filled,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: Some(format!("mock-{id}")),
        message: None,
        filled_quantity: Some(2.0),
        filled_price: Some(100.0),
        filled_fee: None,
        updated_at_ms: 2,
    }
}

pub(super) fn dry_order_source(reduce_only: bool) -> OrderSource {
    if reduce_only {
        OrderSource::Manual
    } else {
        OrderSource::ArbitragePreview
    }
}

pub(super) fn execution_run() -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state: ExecutionRunState::Hedged,
        long_leg: execution_leg(HedgeLegRole::Long),
        short_leg: execution_leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "hedged".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

pub(super) fn execution_leg(role: HedgeLegRole) -> ExecutionRunLeg {
    let side = match role {
        HedgeLegRole::Long => "long",
        HedgeLegRole::Short => "short",
    };
    ExecutionRunLeg {
        role,
        exchange: "OKX".into(),
        symbol: "BTC".into(),
        order_ids: vec![format!("{side}-order")],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: Some(2),
        state: LiveOrderState::Filled,
        target_quantity: 1.0,
        filled_quantity: Some(1.0),
        target_notional_usd: 100.0,
        filled_notional_usd: Some(100.0),
        filled_fee: None,
    }
}

pub(super) fn close_run_fixture(id: &str, updated_at_ms: i64) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::UnwindRequired,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 2,
        reason: Some("positions.close_pair".to_owned()),
        legs: Vec::new(),
        submitted_order_count: 1,
        failed_leg_count: 1,
        naked_exposure_usd: 10.0,
        message: "unwind required".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: updated_at_ms,
        updated_at_ms,
    }
}
