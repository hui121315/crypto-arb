use super::super::*;
use shared_types::{
    HedgeLegRole, OrderPayloadPricePolicy, TimeInForce, VenueOperationEvidence, VenueOrderKind,
};

pub(super) fn plan(exchange: &str, symbol: &str, blockers: Vec<String>) -> OrderCompilePlan {
    OrderCompilePlan {
        role: HedgeLegRole::Long,
        exchange: exchange.to_owned(),
        symbol: symbol.to_owned(),
        client_order_id_policy: shared_types::ClientOrderIdPolicy::default(),
        product: FeeProduct::Perp,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: OrderType::Limit,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Gtc,
        effective_time_in_force: TimeInForce::Gtc,
        available_order_types: vec![OrderType::Limit],
        available_time_in_force: vec![TimeInForce::Gtc],
        available_margin_modes: Vec::new(),
        venue_capability: shared_types::VenueSymbolCapability::default(),
        market_order_style: None,
        venue_order_kind: VenueOrderKind::Limit,
        payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
        reference_price: Some(1.0),
        protection_price: Some(1.0),
        payload_price: Some(1.0),
        slippage_tolerance_bps: None,
        summary: "test".to_owned(),
        blockers,
    }
}

pub(super) fn capabilities() -> ExchangeCapabilities {
    ExchangeCapabilities {
        supports_testnet: true,
        supports_live: true,
        supports_spot: false,
        supports_perp: true,
        supports_limit_orders: true,
        supports_market_orders: true,
        supports_post_only: true,
        supports_reduce_only: true,
    }
}

pub(super) fn account_mode(venue: &str, mode: &str) -> VenueAccountModeInfo {
    VenueAccountModeInfo {
        venue: venue.to_owned(),
        mode: mode.to_owned(),
        source: "test".to_owned(),
        checked_at_ms: 1,
        freshness_ms: Some(8),
        account_scope: None,
    }
}

pub(super) fn operation_row(
    venue: &str,
    operation: &str,
    status: VenueOperationStatus,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: operation.to_owned(),
        status,
        source: "test".to_owned(),
        message: format!("{operation} sample"),
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(u64::from(status == VenueOperationStatus::Ok)),
        freshness_ms: Some(100),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1_000,
    }
}

pub(super) fn full_live_operation_rows(venue: &str) -> Vec<VenueOperationHealth> {
    required_live_operations()
        .into_iter()
        .map(|required| operation_row(venue, required.operation, VenueOperationStatus::Ok))
        .collect()
}

pub(super) fn request_evidence(request_id: &str) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "GET".to_owned(),
        path: "/test".to_owned(),
        checked_at: "test".to_owned(),
        doc_version: "test".to_owned(),
        schema_hash: "test".to_owned(),
        fixture_id: "test".to_owned(),
        parser_test: "test".to_owned(),
        request_builder_test: "test".to_owned(),
        auth_kind: "private".to_owned(),
        request_id: Some(request_id.to_owned()),
        request_context: Vec::new(),
        doc_urls: Vec::new(),
        use_cases: Vec::new(),
        data_kinds: Vec::new(),
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

pub(super) fn missing_guard() -> ExecutionGuard {
    ExecutionGuard {
        key: "missing".to_owned(),
        label: "missing".to_owned(),
        passed: false,
        detail: "missing".to_owned(),
        preflight_outcome: None,
    }
}

pub(super) fn sizing_plan_fixture() -> ExecutionSizingPlan {
    ExecutionSizingPlan {
        target_notional_usd: 1234.0,
        reference_price: 30_000.0,
        price_tick: 0.1,
        contract_size: 1.0,
        qty_step: 0.001,
        min_qty: 0.001,
        min_notional: 5.0,
        raw_contracts: 1234.0 / 30_000.0,
        raw_base_qty: 1234.0 / 30_000.0,
        rounded_contracts: 0.041,
        rounded_base_qty: 0.041,
        actual_notional_usd: 1230.0,
        rounding_delta_usd: 4.0,
        rounding_loss_bps: 4.0 / 1234.0 * 10_000.0,
    }
}
