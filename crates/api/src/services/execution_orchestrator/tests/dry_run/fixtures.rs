use super::*;
use shared_types::hedge::HedgeTicketOrderPlans;
use shared_types::{
    ExecutionGuard, FeeProduct, HedgeDepthStatus, HedgeExecutableNotional, HedgeSizing,
    HedgeTicket, MarginMode, OrderBookInfo, OrderCompilePlan, OrderPayloadPricePolicy, OrderSource,
    SpotLegMode, StrategyKind, TimeInForce, VenueOrderKind, VenueSymbolCapability,
};

mod evidence;
use evidence::{cost_profile, fee_snapshot, fresh_health};

pub(super) async fn test_state() -> anyhow::Result<AppState> {
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    let state = AppState::new(config).await?;
    state.trading_service().select_mock_adapter();
    Ok(state)
}

pub(super) fn preview(now_ms: i64) -> anyhow::Result<HedgePreviewResponse> {
    let long_leg = intent(
        "dry-run-long",
        "BTC-LONG",
        OrderSide::Buy,
        100.0,
        0.1,
        now_ms,
    );
    let short_leg = intent(
        "dry-run-short",
        "BTC-SHORT",
        OrderSide::Sell,
        101.0,
        0.1,
        now_ms,
    );
    let ticket = ticket(now_ms);
    let ticket_order_plans = HedgeTicketOrderPlans::from_compile_plans(
        ticket.ticket_id.clone(),
        compile_plan(HedgeLegRole::Long, &long_leg),
        compile_plan(HedgeLegRole::Short, &short_leg),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(HedgePreviewResponse {
        opportunity_id: ticket.opportunity_id.clone(),
        opportunity_snapshot_id: "snapshot-dry-run".to_owned(),
        ticket,
        workflow_view: Default::default(),
        ticket_order_plans: Some(ticket_order_plans),
        long_leg,
        long_risk: shared_types::RiskDecision::allow(10.0),
        short_leg,
        short_risk: shared_types::RiskDecision::allow(10.0),
        long_order_plan: None,
        short_order_plan: None,
        estimated_funding_next_settlement_usd: Some(0.0),
        estimated_funding_per_8h_usd: 0.0,
        estimated_gross_edge_usd: 0.1,
        estimated_open_cost_usd: 0.002,
        estimated_close_cost_usd: 0.002,
        estimated_slippage_usd: 0.0,
        current_account_liq_distance_pct: None,
        after_hedge_liq_distance_pct: None,
        positions_evidence: None,
        used_capital_usd: 0.0,
        max_loss_usd: 0.0,
        idempotency_key: "dry-run-pair".to_owned(),
    })
}

pub(super) fn seed_spot_book(state: &AppState, symbol: &str, price: f64, now_ms: i64) {
    seed_spot_book_with_quantity(state, symbol, price, 100.0, now_ms);
}

pub(super) fn seed_spot_book_with_quantity(
    state: &AppState,
    symbol: &str,
    price: f64,
    quantity: f64,
    now_ms: i64,
) {
    state.market_data().store_spot_orderbook(
        "mock",
        OrderBookInfo {
            symbol: symbol.to_owned(),
            exchange: "mock".to_owned(),
            bids: vec![[price, quantity]],
            asks: vec![[price, quantity]],
            timestamp: now_ms,
        },
        crate::services::market_data::MarketSource::WsPush,
    );
}

fn ticket(now_ms: i64) -> HedgeTicket {
    HedgeTicket {
        ticket_id: "ticket-dry-run".to_owned(),
        opportunity_id: "opportunity-dry-run".to_owned(),
        strategy: Some(StrategyKind::SpotCross),
        spot_leg_mode: Some(SpotLegMode::SellInventory),
        symbol: "BTC".to_owned(),
        created_at_ms: now_ms,
        market_checked_at_ms: now_ms,
        expires_at_ms: now_ms.saturating_add(60_000),
        long_leg: quote(
            HedgeLegRole::Long,
            "BTC-LONG",
            OrderSide::Buy,
            100.0,
            now_ms,
        ),
        short_leg: quote(
            HedgeLegRole::Short,
            "BTC-SHORT",
            OrderSide::Sell,
            101.0,
            now_ms,
        ),
        cost: Some(cost_profile()),
        fee_snapshots: vec![
            fee_snapshot("BTC-LONG", now_ms),
            fee_snapshot("BTC-SHORT", now_ms),
        ],
        sizing: HedgeSizing {
            requested_capital_usd: 10.0,
            leverage: 1.0,
            target_notional_usd: 10.0,
            long_notional_cap_usd: 10.0,
            short_notional_cap_usd: 10.0,
            target_base_quantity: Some(0.1),
            max_executable_notional: HedgeExecutableNotional {
                status: HedgeDepthStatus::Available,
                amount_usd: Some(10_000.0),
                reason: None,
                long_leg_depth_usd: Some(10_000.0),
                short_leg_depth_usd: Some(10_100.0),
            },
        },
        guards: vec![ExecutionGuard {
            key: "profit_lock".to_owned(),
            label: "收益下限".to_owned(),
            passed: true,
            detail: "通过".to_owned(),
            preflight_outcome: None,
        }],
        blockers: Vec::new(),
    }
}

fn intent(
    id: &str,
    symbol: &str,
    side: OrderSide,
    price: f64,
    quantity: f64,
    now_ms: i64,
) -> OrderIntent {
    let client_order_id = format!("{id}-client");
    OrderIntent {
        id: id.to_owned(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::SpotCross),
        mode: ExecutionMode::DryRun,
        exchange: "mock".to_owned(),
        symbol: symbol.to_owned(),
        side,
        order_type: OrderType::Limit,
        quantity,
        price: Some(price),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id_policy: Some(exchange::client_order_id_policy("mock", &client_order_id)),
        client_order_id,
        created_at_ms: now_ms,
    }
}

fn compile_plan(role: HedgeLegRole, intent: &OrderIntent) -> OrderCompilePlan {
    OrderCompilePlan {
        role,
        exchange: intent.exchange.clone(),
        symbol: intent.symbol.clone(),
        client_order_id_policy: intent.client_order_id_policy.clone().unwrap_or_default(),
        product: FeeProduct::Spot,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: OrderType::Limit,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Ioc,
        effective_time_in_force: TimeInForce::Ioc,
        available_order_types: vec![OrderType::Limit],
        available_time_in_force: vec![TimeInForce::Ioc],
        available_margin_modes: Vec::new(),
        venue_capability: VenueSymbolCapability::default(),
        market_order_style: None,
        venue_order_kind: VenueOrderKind::Limit,
        payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
        reference_price: intent.price,
        protection_price: intent.price,
        payload_price: intent.price,
        slippage_tolerance_bps: None,
        summary: "mock limit".to_owned(),
        blockers: Vec::new(),
    }
}

fn quote(
    role: HedgeLegRole,
    symbol: &str,
    side: OrderSide,
    price: f64,
    now_ms: i64,
) -> HedgeLegQuote {
    HedgeLegQuote {
        role,
        exchange: "mock".to_owned(),
        symbol: symbol.to_owned(),
        side,
        reference_price: Some(price),
        bid: Some(price),
        ask: Some(price),
        mid: Some(price),
        open_vwap_price: Some(price),
        open_slippage_bps: Some(0.0),
        close_vwap_price: Some(price),
        close_slippage_bps: Some(0.0),
        depth_usd_5bps: Some(price * 100.0),
        depth_usd_10bps: Some(price * 100.0),
        depth_usd_20bps: Some(price * 100.0),
        max_notional_usd: Some(price * 100.0),
        market_evidence: None,
        depth_health: Some(fresh_health(now_ms)),
        depth_reason: None,
        funding_bps: Some(0.0),
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: Some(now_ms),
        blockers: Vec::new(),
    }
}
