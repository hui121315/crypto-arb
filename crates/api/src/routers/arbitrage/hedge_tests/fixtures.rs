use super::*;

#[path = "fixtures/quotes.rs"]
mod quotes;

pub(super) fn blocked_opportunity() -> ArbitrageOpportunityDto {
    let raw = RawOpportunity {
        symbol: "BTC".into(),
        arb_type: ArbitrageType::SpotCross,
        long_exchange: "bitget".into(),
        short_exchange: "gate".into(),
        long_rate: zero_rate("BTC", "bitget"),
        short_rate: zero_rate("BTC", "gate"),
        spread_8h: 0.004,
        single_yield: 0.004,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::SpotCross),
            long_price: Some(200.0),
            short_price: Some(201.0),
            execution_blockers: vec!["当前资金费差为 0，先观察不执行".into()],
            ..Default::default()
        },
    };
    OpportunityBuilder {
        raw: &raw,
        metrics: &RiskMetrics::default(),
        position: &PositionSizing::default(),
        cost: &CostBreakdown::default(),
        min_holding_periods: 1,
        net_single_yield: raw.single_yield,
        data_source: "test",
        confidence: 0.8,
    }
    .build()
}

pub(super) fn perp_opportunity() -> ArbitrageOpportunityDto {
    let mut opportunity = blocked_opportunity();
    opportunity.arb_type = ArbitrageType::CrossExchange;
    opportunity.strategy_kind = Some(StrategyKind::PerpCross);
    opportunity.long_action = "bitget 做多永续".into();
    opportunity.short_action = "gate 做空永续".into();
    opportunity
}

pub(super) fn raw_eligible_without_verified_fee_opportunity() -> ArbitrageOpportunityDto {
    let next_funding_time = chrono::Utc::now().timestamp_millis() + 8 * 3_600_000;
    let mut long_rate = funding_rate("BTC", "binance", -0.0001);
    long_rate.next_funding_time = next_funding_time;
    let mut short_rate = funding_rate("BTC", "okx", 0.0002);
    short_rate.next_funding_time = next_funding_time;
    let raw = RawOpportunity {
        symbol: "BTC".into(),
        arb_type: ArbitrageType::CrossExchange,
        long_exchange: "binance".into(),
        short_exchange: "okx".into(),
        long_rate,
        short_rate,
        spread_8h: 0.003,
        single_yield: 0.003,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::PerpCross),
            long_action: Some("binance 做多永续".into()),
            short_action: Some("okx 做空永续".into()),
            long_price: Some(100.0),
            short_price: Some(100.1),
            long_leg_market_evidence: Some(market_evidence("binance", "BTC")),
            short_leg_market_evidence: Some(market_evidence("okx", "BTC")),
            ..Default::default()
        },
    };
    OpportunityBuilder {
        raw: &raw,
        metrics: &RiskMetrics::default(),
        position: &PositionSizing::default(),
        cost: &CostBreakdown::default(),
        min_holding_periods: 1,
        net_single_yield: raw.single_yield,
        data_source: "test",
        confidence: 0.9,
    }
    .build()
}

pub(super) fn confirm_response_with_run(state: ExecutionRunState) -> HedgeConfirmResponse {
    HedgeConfirmResponse {
        idempotency_key: "hedge-test".into(),
        status: shared_types::HedgeConfirmStatus::Submitted,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(execution_run_with_state(state)),
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: None,
        partial_outcome: None,
        error: None,
    }
}

pub(super) fn execution_run_with_state(state: ExecutionRunState) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-hedge-test".into(),
        ticket_id: "ticket-test".into(),
        opportunity_id: "opp-test".into(),
        state,
        long_leg: run_leg_with_role(HedgeLegRole::Long),
        short_leg: run_leg_with_role(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "test".into(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

pub(super) fn run_leg_with_role(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "okx".into(),
        symbol: "BTCUSDT".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: shared_types::LiveOrderState::Submitted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

pub(super) fn zero_rate(symbol: &str, exchange: &str) -> FundingRateData {
    funding_rate(symbol, exchange, 0.0)
}

pub(super) fn funding_rate(symbol: &str, exchange: &str, rate: f64) -> FundingRateData {
    FundingRateData {
        symbol: symbol.into(),
        exchange: exchange.into(),
        rate,
        rate_8h: rate,
        predicted_rate: None,
        next_funding_time: 0,
        funding_interval: 8,
        volume_24h: 100_000.0,
        timestamp: 1_715_925_600_000,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

pub(super) fn market_evidence(venue: &str, symbol: &str) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: symbol.into(),
        price: Some(100.0),
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::LocalCache,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: None,
            problem: None,
        },
    }
}

pub(super) fn market_params() -> HedgeExecutionParams {
    HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Market,
        market_order_style: None,
        margin_mode: MarginMode::Cross,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        limit_offset_bps: 5.0,
    }
}

pub(super) fn market_leg_build<'a>(
    opp: &'a shared_types::ArbitrageOpportunityDto,
    params: &'a HedgeExecutionParams,
) -> LegBuild<'a> {
    LegBuild {
        opp,
        key: "hedge-market",
        is_long: true,
        quantity: 15.0,
        price: 100.0,
        params,
        mode: ExecutionMode::DryRun,
        strategy: Some(StrategyKind::PerpCross),
    }
}

pub(super) fn order_plan_for_exchange(exchange: &str) -> shared_types::OrderCompilePlan {
    let mut opp = perp_opportunity();
    opp.long_exchange = exchange.to_owned();
    let params = market_params();
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);
    compile_order_plan(&build, &intent)
}

pub(super) fn position_with_liq_distance(liq_distance_pct: Option<f64>) -> PositionInfo {
    PositionInfo {
        symbol: "BTCUSDT".into(),
        exchange: "okx".into(),
        side: "long".into(),
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 100.0,
        leverage: 2.0,
        unrealized_pnl: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: liq_distance_pct,
        next_funding_ms: None,
        paired_with: None,
        margin: 50.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

pub(super) fn ticket_with(blockers: Vec<String>, guard_passed: bool) -> HedgeTicket {
    HedgeTicket {
        ticket_id: "ticket-test".into(),
        opportunity_id: "opp-test".into(),
        strategy: Some(StrategyKind::PerpCross),
        spot_leg_mode: None,
        symbol: "BTCUSDT".into(),
        created_at_ms: 1,
        market_checked_at_ms: 1,
        expires_at_ms: 60_000,
        long_leg: leg_quote(HedgeLegRole::Long, OrderSide::Buy),
        short_leg: leg_quote(HedgeLegRole::Short, OrderSide::Sell),
        cost: None,
        fee_snapshots: Vec::new(),
        sizing: shared_types::HedgeSizing {
            requested_capital_usd: 100.0,
            leverage: 1.0,
            target_notional_usd: 100.0,
            long_notional_cap_usd: 100.0,
            short_notional_cap_usd: 100.0,
            target_base_quantity: None,
            max_executable_notional: shared_types::HedgeExecutableNotional {
                status: shared_types::HedgeDepthStatus::Available,
                amount_usd: Some(100.0),
                ..shared_types::HedgeExecutableNotional::default()
            },
        },
        guards: vec![shared_types::ExecutionGuard {
            key: "depth".into(),
            label: "深度".into(),
            passed: guard_passed,
            detail: if guard_passed {
                "通过"
            } else {
                "深度不足"
            }
            .into(),
            preflight_outcome: None,
        }],
        blockers,
    }
}

pub(super) fn leg_quote(role: HedgeLegRole, side: OrderSide) -> HedgeLegQuote {
    quotes::leg_quote(role, side)
}
