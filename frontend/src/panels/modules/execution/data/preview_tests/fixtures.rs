use super::super::*;
use shared_types::{MarketDataQuality, MarketDataSourceKind};

#[path = "fixtures/response.rs"]
mod response;

pub(super) fn preview_response() -> shared_types::HedgePreviewResponse {
    response::preview_response()
}

pub(super) fn preview_query_fixture() -> PreviewQuery {
    PreviewQuery {
        seed: preview_seed_fixture(),
        input: preview_input(),
    }
}

pub(super) fn preview_seed_fixture() -> PreviewSeed {
    let mut selection = ExecutionSelection::empty();
    selection.opportunity_id = "opp-1".into();
    selection.opportunity_snapshot_id = "snapshot-1".into();
    selection.pair = "MU".into();
    selection.long_leg_label = "hype 做多 MU".into();
    selection.short_leg_label = "gate 做空 MU".into();
    PreviewSeed::from_selection(&selection)
}

pub(super) fn preview_input() -> PreviewInput {
    PreviewInput {
        capital_usd: 100.0,
        leverage: 2.0,
        order_type: OrderType::Limit,
        limit_offset_bps: 0.0,
        long_price: Some(100.0),
        short_price: Some(101.0),
        long_notional_usd: 200.0,
        short_notional_usd: 200.0,
        execution_params: HedgeExecutionParams {
            capital_usd: 100.0,
            leverage: 2.0,
            order_type: OrderType::Limit,
            market_order_style: None,
            margin_mode: MarginMode::Cross,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            limit_offset_bps: 0.0,
        },
    }
}

pub(super) fn hyperliquid_ticket_order_plans() -> shared_types::hedge::HedgeTicketOrderPlans {
    let long = hyperliquid_order_plan(shared_types::HedgeLegRole::Long);
    let short = hyperliquid_order_plan(shared_types::HedgeLegRole::Short);
    let long_identity = long.identity_plan();
    let short_identity = short.identity_plan();
    shared_types::hedge::HedgeTicketOrderPlans {
        ticket_id: "ticket-1".into(),
        long: shared_types::hedge::HedgeTicketOrderPlanEvidence {
            compile_plan: long,
            identity_plan: long_identity,
        },
        short: shared_types::hedge::HedgeTicketOrderPlanEvidence {
            compile_plan: short,
            identity_plan: short_identity,
        },
    }
}

fn hyperliquid_order_plan(role: shared_types::HedgeLegRole) -> shared_types::OrderCompilePlan {
    let venue_client_order_id = match role {
        shared_types::HedgeLegRole::Long => "0x00000000000000000000000000000001",
        shared_types::HedgeLegRole::Short => "0x00000000000000000000000000000002",
    };
    let instrument = hyperliquid_instrument();
    let sizing = shared_types::plan_leg_sizing(200.0, &instrument, 100.0).ok();
    shared_types::OrderCompilePlan {
        role,
        exchange: "hyperliquid:xyz".into(),
        symbol: "XYZ".into(),
        client_order_id_policy: shared_types::ClientOrderIdPolicy {
            venue: "hyperliquid:xyz".into(),
            venue_family: "hyperliquid".into(),
            venue_field: "c/cloid".into(),
            public_client_order_id: format!("public-{venue_client_order_id}"),
            venue_client_order_id: Some(venue_client_order_id.into()),
            derivation: shared_types::ClientOrderIdDerivation::StableHash,
            policy_version: "hyperliquid-cloid-v1".into(),
            official_format: "0x + 32 lowercase hex".into(),
            max_length: Some(34),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            constraints: Vec::new(),
            blockers: Vec::new(),
            official_doc_urls: Vec::new(),
        },
        product: shared_types::FeeProduct::Perp,
        instrument_spec: sizing.map(|_| instrument),
        sizing_plan: sizing,
        requested_order_type: OrderType::Market,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Ioc,
        effective_time_in_force: TimeInForce::Ioc,
        available_order_types: vec![OrderType::Limit, OrderType::Market],
        available_time_in_force: vec![TimeInForce::Ioc],
        available_margin_modes: vec![MarginMode::Cross],
        venue_capability: shared_types::VenueSymbolCapability::default(),
        market_order_style: None,
        venue_order_kind: shared_types::VenueOrderKind::ProtectedIoc,
        payload_price_policy: shared_types::OrderPayloadPricePolicy::ProtectionPrice,
        reference_price: Some(100.0),
        protection_price: Some(100.05),
        payload_price: Some(100.05),
        slippage_tolerance_bps: Some(5.0),
        summary: "Hyperliquid protected IOC".into(),
        blockers: Vec::new(),
    }
}

fn hyperliquid_instrument() -> shared_types::InstrumentSpec {
    shared_types::InstrumentSpec {
        venue: "hyperliquid:xyz".into(),
        native_symbol: "@123".into(),
        canonical_symbol: "XYZ".into(),
        display_symbol: "XYZ Perp".into(),
        asset_class: shared_types::InstrumentAssetClass::Crypto,
        product_type: Some("perp".into()),
        quote_asset: Some("USDC".into()),
        settle_asset: Some("USDC".into()),
        margin_asset: Some("USDC".into()),
        contract_size: Some(1.0),
        execution_supported: true,
        price_tick: Some(0.001),
        qty_step: Some(0.01),
        min_qty: Some(0.01),
        min_notional: None,
        listing_status: shared_types::InstrumentListingStatus::Trading,
        funding_interval_ms: Some(28_800_000),
        builder_dex: Some("xyz".into()),
        source: shared_types::InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some("https://api.hyperliquid.xyz/info".into()),
        checked_at_ms: 1_700_000_000_000,
        schema_version: Some("hyperliquid-meta-v1".into()),
    }
}

fn ticket() -> shared_types::HedgeTicket {
    shared_types::HedgeTicket {
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        strategy: Some(shared_types::StrategyKind::PerpCross),
        spot_leg_mode: None,
        symbol: "MU".into(),
        created_at_ms: 1,
        market_checked_at_ms: 1,
        expires_at_ms: 60_001,
        long_leg: leg_quote(
            shared_types::HedgeLegRole::Long,
            shared_types::OrderSide::Buy,
        ),
        short_leg: leg_quote(
            shared_types::HedgeLegRole::Short,
            shared_types::OrderSide::Sell,
        ),
        cost: None,
        fee_snapshots: Vec::new(),
        sizing: shared_types::HedgeSizing {
            requested_capital_usd: 100.0,
            leverage: 2.0,
            target_notional_usd: 200.0,
            long_notional_cap_usd: 200.0,
            short_notional_cap_usd: 200.0,
            target_base_quantity: None,
            max_executable_notional: shared_types::HedgeExecutableNotional {
                status: shared_types::HedgeDepthStatus::Available,
                amount_usd: Some(200.0),
                ..shared_types::HedgeExecutableNotional::default()
            },
        },
        guards: Vec::new(),
        blockers: Vec::new(),
    }
}

pub(super) fn execution_cost() -> shared_types::ExecutionCostProfile {
    shared_types::ExecutionCostProfile {
        gross_edge_bps: 4.0,
        fee_bps: 7.0,
        wear_bps: 8.0,
        total_cost_bps: 15.0,
        one_cycle: shared_types::OneCycleCostProfile {
            gross_edge_bps: 4.0,
            open_fee_bps: 3.5,
            close_fee_bps: 3.5,
            open_slippage_bps: 4.0,
            close_slippage_bps: 4.0,
            funding_window_mismatch_buffer_bps: 0.0,
            yield_basis: Some(shared_types::YieldBasis::NativeSettlement),
            long_next_settlement_ms: Some(1_000),
            short_next_settlement_ms: Some(2_000),
            target_buffer_bps: 0.0,
            net_bps: -11.0,
            covers_round_trip_cost: false,
        },
        breakeven_periods: 4,
        breakeven_hours: 32.0,
        recommended_hold_periods: 5,
        recommended_hold_hours: 40.0,
        net_bps_at_recommended_hold: 5.0,
        round_trip: None,
    }
}

fn leg_quote(
    role: shared_types::HedgeLegRole,
    side: shared_types::OrderSide,
) -> shared_types::HedgeLegQuote {
    shared_types::HedgeLegQuote {
        role,
        exchange: "venue".into(),
        symbol: "MU".into(),
        side,
        reference_price: Some(100.0),
        bid: Some(99.9),
        ask: Some(100.1),
        mid: Some(100.0),
        open_vwap_price: Some(100.1),
        open_slippage_bps: Some(0.0),
        close_vwap_price: Some(99.9),
        close_slippage_bps: Some(0.0),
        depth_usd_5bps: Some(1_000.0),
        depth_usd_10bps: Some(1_500.0),
        depth_usd_20bps: Some(2_000.0),
        max_notional_usd: Some(1_000.0),
        market_evidence: None,
        depth_health: None,
        depth_reason: None,
        funding_bps: Some(0.0),
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: Some(1),
        blockers: Vec::new(),
    }
}

fn order_intent(id: &str, side: shared_types::OrderSide) -> shared_types::OrderIntent {
    shared_types::OrderIntent {
        id: id.into(),
        source: shared_types::OrderSource::ArbitragePreview,
        strategy: Some(shared_types::StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: "venue".into(),
        symbol: "MU".into(),
        side,
        order_type: OrderType::Limit,
        quantity: 2.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 2.0,
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

pub(super) fn leg_evidence(
    venue: &str,
    symbol: &str,
    source: MarketDataSourceKind,
) -> shared_types::OpportunityLegMarketEvidence {
    shared_types::OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: symbol.into(),
        price: Some(100.0),
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: None,
            problem: None,
        },
    }
}
