use super::health::{capability_health, fee_snapshot_health, market_health};
use super::refresh_margin_evidence;
use shared_types::hedge::HedgeTicketOrderPlans;
use shared_types::{
    ExecutionGuard, FeeProduct, HedgeLegQuote, HedgeLegRole, HedgePreflightOperation,
    HedgePreflightScope, HedgePreflightStatus, HedgeTicketLegView, HedgeTicketView, MarginMode,
    MarginPreflightOutcome, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
    OpportunityLegMarketEvidence, OrderCompilePlan, OrderPayloadPricePolicy, OrderType,
    ResourceStatus, TimeInForce, TradeFeeEvidence, TradeFeeSnapshot, TradeFeeSource,
    VenueOrderKind, WorkflowEvidenceHealth,
};

#[test]
fn market_projection_keeps_source_time_and_problem() {
    let mut quote = quote();
    quote.market_evidence = Some(OpportunityLegMarketEvidence {
        venue: "okx".into(),
        symbol: "BTC-USDT-SWAP".into(),
        price: Some(100.0),
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 42,
            coverage: None,
            problem: None,
        },
    });

    let health = market_health(&quote);

    assert_eq!(health.status, ResourceStatus::Ready);
    assert_eq!(health.source.as_deref(), Some("ws_push"));
    assert_eq!(health.observed_at_ms, Some(42));
    assert!(health.problem.is_none());
}

#[test]
fn fee_projection_requires_fresh_verified_snapshot() {
    let snapshot = TradeFeeSnapshot {
        venue: "okx".into(),
        symbol: "BTC-USDT-SWAP".into(),
        product: FeeProduct::Perp,
        account_id: None,
        maker_fee_bps: 2.0,
        taker_fee_bps: 5.0,
        open_fee_bps: 5.0,
        close_fee_bps: 5.0,
        source: TradeFeeSource::OfficialSchedule,
        fetched_at_ms: 10,
        valid_until_ms: 100,
        freshness_ms: Some(90),
        evidence: Some(TradeFeeEvidence {
            evidence_id: "fee-okx-v1".into(),
            source_name: "okx_official".into(),
            source_url: "https://www.okx.com/fees".into(),
            checked_at_ms: 10,
            effective_at_ms: None,
            schedule_version: Some("v1".into()),
            tier: None,
            scope: None,
            problem: None,
        }),
        verification_problem: None,
        note: None,
    };

    assert_eq!(
        fee_snapshot_health(&snapshot, 50).status,
        ResourceStatus::Ready
    );
    assert_eq!(
        fee_snapshot_health(&snapshot, 101).status,
        ResourceStatus::Error
    );
}

#[test]
fn compiled_capability_is_ready_when_paper_preview_skips_live_guard() -> Result<(), &'static str> {
    let mut compile = compile_plan();
    compile.venue_capability.source = "ticket_bound_capability_registry".to_owned();
    let plans = HedgeTicketOrderPlans::from_compile_plans("ticket-1", compile.clone(), {
        compile.role = HedgeLegRole::Short;
        compile
    })
    .map_err(|_| "invalid order plans")?;

    let health = capability_health(None, &plans.long);

    assert_eq!(health.status, ResourceStatus::Ready);
    assert_eq!(
        health.source.as_deref(),
        Some("ticket_bound_capability_registry")
    );
    assert!(health.problem.is_none());
    Ok(())
}

#[test]
fn missing_compiled_capability_stays_blocked_without_live_guard() -> Result<(), &'static str> {
    let mut compile = compile_plan();
    compile.blockers.push("instrument unavailable".to_owned());
    let plans = HedgeTicketOrderPlans::from_compile_plans("ticket-1", compile.clone(), {
        compile.role = HedgeLegRole::Short;
        compile
    })
    .map_err(|_| "invalid order plans")?;

    let health = capability_health(None, &plans.long);

    assert_eq!(health.status, ResourceStatus::Error);
    assert!(health
        .problem
        .as_ref()
        .is_some_and(|problem| problem.message == "instrument unavailable"));
    Ok(())
}

#[test]
fn final_margin_refresh_updates_both_legs_without_erasing_other_health() -> Result<(), &'static str>
{
    let preserved_market = WorkflowEvidenceHealth {
        status: ResourceStatus::Ready,
        source: Some("ws_push".into()),
        evidence_id: Some("market-okx-1".into()),
        ..WorkflowEvidenceHealth::default()
    };
    let mut view = HedgeTicketView {
        long_leg: Some(HedgeTicketLegView {
            role: HedgeLegRole::Long,
            venue: "okx".into(),
            market: preserved_market.clone(),
            ..HedgeTicketLegView::default()
        }),
        short_leg: Some(HedgeTicketLegView {
            role: HedgeLegRole::Short,
            venue: "binance".into(),
            ..HedgeTicketLegView::default()
        }),
        ..HedgeTicketView::default()
    };
    let guard = ExecutionGuard {
        key: "margin_balance".into(),
        label: "保证金余额".into(),
        passed: true,
        detail: "确认终检通过".into(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: HedgePreflightStatus::Passed,
            checked_at_ms: 42,
            scope: HedgePreflightScope {
                venues: vec!["okx".into(), "binance".into()],
                operations: vec![HedgePreflightOperation::MarginBalance],
                ..HedgePreflightScope::default()
            },
            source: Some("account_state.margin_facts".into()),
            request_id: Some("req-final-margin".into()),
            ..MarginPreflightOutcome::default()
        }),
    };

    refresh_margin_evidence(&mut view, &guard, "okx", "binance")?;

    let long = view.long_leg.as_ref().ok_or("missing long leg")?;
    let short = view.short_leg.as_ref().ok_or("missing short leg")?;
    assert_eq!(long.market, preserved_market);
    assert_eq!(long.balance.status, ResourceStatus::Ready);
    assert_eq!(long.balance.observed_at_ms, Some(42));
    assert_eq!(short.balance.observed_at_ms, Some(42));
    assert_eq!(
        short.balance.request_id.as_deref(),
        Some("req-final-margin")
    );
    Ok(())
}

fn compile_plan() -> OrderCompilePlan {
    OrderCompilePlan {
        role: HedgeLegRole::Long,
        exchange: "okx".to_owned(),
        symbol: "BTC-USDT-SWAP".to_owned(),
        client_order_id_policy: Default::default(),
        product: FeeProduct::Perp,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: OrderType::Limit,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Ioc,
        effective_time_in_force: TimeInForce::Ioc,
        available_order_types: vec![OrderType::Limit],
        available_time_in_force: vec![TimeInForce::Ioc],
        available_margin_modes: vec![MarginMode::Cross],
        venue_capability: Default::default(),
        market_order_style: None,
        venue_order_kind: VenueOrderKind::Limit,
        payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
        reference_price: Some(100.0),
        protection_price: None,
        payload_price: Some(100.0),
        slippage_tolerance_bps: None,
        summary: "test plan".to_owned(),
        blockers: Vec::new(),
    }
}

fn quote() -> HedgeLegQuote {
    HedgeLegQuote {
        role: HedgeLegRole::Long,
        exchange: "okx".into(),
        symbol: "BTC-USDT-SWAP".into(),
        side: shared_types::OrderSide::Buy,
        reference_price: Some(100.0),
        bid: None,
        ask: None,
        mid: None,
        open_vwap_price: None,
        open_slippage_bps: None,
        close_vwap_price: None,
        close_slippage_bps: None,
        depth_usd_5bps: None,
        depth_usd_10bps: None,
        depth_usd_20bps: None,
        max_notional_usd: None,
        market_evidence: None,
        depth_health: None,
        depth_reason: None,
        funding_bps: None,
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: None,
        blockers: Vec::new(),
    }
}
