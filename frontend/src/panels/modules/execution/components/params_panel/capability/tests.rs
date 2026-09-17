use super::super::super::super::data::{
    PreviewDepth, PreviewLiquidation, PreviewProfitEvidence, PreviewReadiness, PreviewRisk,
};
use super::*;
use shared_types::{ApiProblem, HedgeLegRole, OrderPayloadPricePolicy, VenueOrderKind};

#[test]
fn disabled_order_types_use_double_leg_capability_intersection() {
    let left = plan(
        vec![OrderType::Limit, OrderType::Market, OrderType::PostOnly],
        vec![TimeInForce::Ioc, TimeInForce::Fok, TimeInForce::Gtc],
    );
    let right = plan(
        vec![OrderType::Limit],
        vec![TimeInForce::Ioc, TimeInForce::Gtc],
    );

    let disabled = disabled_by_allowed(
        ORDER_TYPE_OPTIONS,
        &common_order_type_labels(&[left, right]).unwrap_or_default(),
    );
    assert_eq!(disabled, vec!["Market", "Post-only"]);
}

#[test]
fn disabled_time_in_force_uses_double_leg_intersection() {
    let left = plan(
        vec![OrderType::Limit],
        vec![TimeInForce::Ioc, TimeInForce::Fok, TimeInForce::Gtc],
    );
    let right = plan(
        vec![OrderType::Limit],
        vec![TimeInForce::Ioc, TimeInForce::Gtc],
    );

    let disabled = disabled_by_allowed(
        TIME_IN_FORCE_OPTIONS,
        &common_time_in_force_labels(&[left, right]).unwrap_or_default(),
    );
    assert_eq!(disabled, vec!["FOK", "GTX"]);
}

#[test]
fn disabled_margin_modes_only_use_write_path_constrained_legs() {
    let mut constrained = plan(vec![OrderType::Limit], vec![TimeInForce::Ioc]);
    constrained.available_margin_modes = vec![shared_types::MarginMode::Cross];
    let unconstrained = plan(vec![OrderType::Limit], vec![TimeInForce::Ioc]);

    let preview = preview_with_plans(vec![constrained, unconstrained]);

    assert_eq!(disabled_margin_modes(&preview), vec!["Isolated"]);
}

#[test]
fn disabled_margin_modes_empty_when_no_leg_constrains() {
    let preview = preview_with_plans(vec![
        plan(vec![OrderType::Limit], vec![TimeInForce::Ioc]),
        plan(vec![OrderType::Limit], vec![TimeInForce::Ioc]),
    ]);

    assert!(disabled_margin_modes(&preview).is_empty());
}

#[test]
fn capability_hint_labels_margin_modes_per_leg() {
    let mut constrained = plan(vec![OrderType::Limit], vec![TimeInForce::Ioc]);
    constrained.available_margin_modes = vec![
        shared_types::MarginMode::Cross,
        shared_types::MarginMode::Isolated,
    ];
    let unconstrained = plan(vec![OrderType::Limit], vec![TimeInForce::Ioc]);

    let constrained_hint = capability_hint(&preview_with_plans(vec![constrained]));
    let unconstrained_hint = capability_hint(&preview_with_plans(vec![unconstrained]));

    assert!(constrained_hint.contains("Cross/Isolated"));
    assert!(unconstrained_hint.contains("保证金模式不进下单载荷"));
}

#[test]
fn first_enabled_option_uses_first_available_label() {
    let disabled = vec!["IOC".to_owned(), "FOK".to_owned()];

    assert_eq!(
        first_enabled_option(TIME_IN_FORCE_OPTIONS, &disabled),
        "GTC"
    );
}

#[test]
fn capability_controls_ignore_stale_preview_plans() {
    let stale = LoadState::Stale {
        value: preview_with_plans(vec![plan(vec![OrderType::Limit], vec![TimeInForce::Ioc])]),
        problem: ApiProblem::new("RATE_LIMITED", "preview slow"),
    };

    assert!(disabled_order_types_for_state(&stale).is_empty());
    assert!(disabled_time_in_force_for_state(&stale).is_empty());
}

#[test]
fn execution_preview_problem_context_capability_error_and_stale() {
    let problem = ApiProblem::new("RATE_LIMITED", "preview slow")
        .with_source("preview-rest")
        .with_status(429)
        .with_request_id(Some("preview-req-1".into()))
        .with_retry_after_ms(Some(2_000));
    let stale = LoadState::Stale {
        value: preview_with_plans(vec![plan(vec![OrderType::Limit], vec![TimeInForce::Ioc])]),
        problem: problem.clone(),
    };
    let error = LoadState::<ExecutionPreview>::Error(problem);

    let stale_hint = capability_hint_for_state(&stale);
    let error_hint = capability_hint_for_state(&error);

    assert!(stale_hint.contains("预览已失效：preview slow"));
    assert!(stale_hint.contains("code RATE_LIMITED"));
    assert!(stale_hint.contains("source preview-rest"));
    assert!(stale_hint.contains("HTTP 429"));
    assert!(stale_hint.contains("request_id preview-req-1"));
    assert!(stale_hint.contains("retry 2000ms"));
    assert!(error_hint.contains("预览失败：preview slow"));
    assert!(error_hint.contains("code RATE_LIMITED"));
    assert!(error_hint.contains("source preview-rest"));
    assert!(error_hint.contains("HTTP 429"));
    assert!(error_hint.contains("request_id preview-req-1"));
    assert!(error_hint.contains("retry 2000ms"));
}

#[test]
fn capability_controls_use_ready_preview_plans() {
    let ready = LoadState::Ready(preview_with_plans(vec![plan(
        vec![OrderType::Limit],
        vec![TimeInForce::Ioc],
    )]));

    assert_eq!(
        disabled_order_types_for_state(&ready),
        vec!["Market", "Post-only"]
    );
    assert_eq!(
        disabled_time_in_force_for_state(&ready),
        vec!["FOK", "GTC", "GTX"]
    );
}

fn preview_with_plans(order_plans: Vec<OrderCompilePlan>) -> ExecutionPreview {
    ExecutionPreview {
        opportunity_id: "opp-1".into(),
        opportunity_snapshot_id: "snapshot-1".into(),
        idempotency_key: Some("idem-1".into()),
        ticket_id: Some("ticket-1".into()),
        expires_at_ms: Some(i64::MAX),
        readiness: PreviewReadiness::Ready,
        source: "后端预检",
        estimated_funding_usd: 1.0,
        open_cost_usd: 0.0,
        close_cost_usd: 0.0,
        slippage_cost_usd: 0.0,
        one_cycle_cost: None,
        max_loss_usd: 1.0,
        used_capital_usd: 100.0,
        liquidation: PreviewLiquidation {
            current_account_pct: None,
            after_hedge_pct: None,
            positions_evidence: None,
        },
        execution_mode_label: "模拟",
        long_allowed: true,
        short_allowed: true,
        long_notional_usd: 100.0,
        short_notional_usd: 100.0,
        long_reference_price: Some(100.0),
        short_reference_price: Some(101.0),
        long_market_evidence: None,
        short_market_evidence: None,
        depth: PreviewDepth {
            long_5bps: None,
            long_10bps: None,
            long_20bps: None,
            short_5bps: None,
            short_10bps: None,
            short_20bps: None,
            executable_status: shared_types::HedgeDepthStatus::Available,
            executable_amount_usd: Some(100.0),
            executable_reason: None,
            long_reason: None,
            short_reason: None,
            long_depth_health: None,
            short_depth_health: None,
        },
        fee_evidence: Vec::new(),
        profit_evidence: PreviewProfitEvidence {
            one_cycle_net_bps: 0.0,
            fee_evidence_ids: Vec::new(),
            fee_evidence_complete: false,
        },
        order_plans,
        identity_evidence_required: false,
        risk: PreviewRisk {
            note: "通过".into(),
            guards: Vec::new(),
            blockers: Vec::new(),
        },
    }
}

fn plan(
    available_order_types: Vec<OrderType>,
    available_time_in_force: Vec<TimeInForce>,
) -> OrderCompilePlan {
    plan_for(
        "gate",
        "BTC_USDT",
        available_order_types,
        available_time_in_force,
    )
}

fn plan_for(
    exchange: &str,
    symbol: &str,
    available_order_types: Vec<OrderType>,
    available_time_in_force: Vec<TimeInForce>,
) -> OrderCompilePlan {
    OrderCompilePlan {
        role: HedgeLegRole::Long,
        exchange: exchange.to_owned(),
        symbol: symbol.to_owned(),
        client_order_id_policy: shared_types::ClientOrderIdPolicy::default(),
        product: shared_types::FeeProduct::Perp,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: OrderType::Limit,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Ioc,
        effective_time_in_force: TimeInForce::Ioc,
        available_order_types,
        available_time_in_force,
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
        blockers: Vec::new(),
    }
}
