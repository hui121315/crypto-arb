use super::super::super::data::{
    PreviewDepth, PreviewLiquidation, PreviewProfitEvidence, PreviewReadiness, PreviewRisk,
};
use super::*;
use shared_types::{
    ApiProblem, ClientOrderIdDerivation, ClientOrderIdPolicy, ExecutionRunLeg, ExecutionRunState,
    HedgeDepthStatus, HedgeLegRole, LiveOrderState, OrderCompilePlan, OrderPayloadPricePolicy,
    OrderType, TimeInForce, VenueOrderKind,
};

#[path = "tests/load_state.rs"]
mod load_state;
#[path = "tests/recent_labels.rs"]
mod recent_labels;

#[test]
fn run_label_uses_action_state_then_real_run() {
    let pending = ActionState::pending("提交中");
    assert_eq!(run_label(&pending, Some(&run())), "提交中");

    let idle = ActionState::Idle;
    assert_eq!(run_label(&idle, Some(&run())), "第二腿已提交，等待成交确认");
    assert_eq!(run_label(&idle, None), "草案待提交");

    let succeeded = ActionState::succeeded("提交成功");
    assert_eq!(
        run_label(&succeeded, Some(&run())),
        "第二腿已提交，等待成交确认"
    );
    assert_eq!(run_label(&succeeded, None), "提交成功");

    let accepted = ActionState::accepted("已提交，等待成交确认");
    assert_eq!(
        run_label(&accepted, Some(&run())),
        "第二腿已提交，等待成交确认"
    );
    assert_eq!(run_label(&accepted, None), "已提交，等待成交确认");
}

#[test]
fn hedged_label_requires_filled_legs() {
    let idle = ActionState::Idle;
    let mut run = run_with_state(ExecutionRunState::Hedged);

    assert_eq!(run_label(&idle, Some(&run)), "等待成交确认");

    run.long_leg.state = LiveOrderState::Filled;
    run.short_leg.state = LiveOrderState::Filled;

    assert_eq!(run_label(&idle, Some(&run)), "双腿完成");
}

#[test]
fn active_run_blocks_duplicate_submission_until_closed() {
    let preview = ready_preview();
    let mut run = run_with_state(ExecutionRunState::Hedged);
    run.long_leg.state = LiveOrderState::Filled;
    run.short_leg.state = LiveOrderState::Filled;

    assert!(run_blocks_new_submission(Some(&run), &preview));
    assert_eq!(submit_button_label(Some(&run), &preview), "已有执行");

    run.state = ExecutionRunState::Closed;
    assert!(!run_blocks_new_submission(Some(&run), &preview));
    assert_eq!(submit_button_label(Some(&run), &preview), "提交 模拟");
    assert_eq!(run_label(&ActionState::Idle, Some(&run)), "执行已收口");
}

#[test]
fn draft_action_status_only_uses_run_for_same_ticket() {
    let preview = ready_preview();
    let mut run = run();
    assert!(run_matches_preview(&run, &preview));

    run.ticket_id = "ticket-old".into();
    assert!(!run_matches_preview(&run, &preview));
}

fn run() -> ExecutionRun {
    run_with_state(ExecutionRunState::SecondLegSubmitted)
}

fn ready_preview() -> ExecutionPreview {
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
            executable_status: HedgeDepthStatus::Available,
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
        order_plans: vec![
            identity_plan(HedgeLegRole::Long),
            identity_plan(HedgeLegRole::Short),
        ],
        identity_evidence_required: true,
        risk: PreviewRisk {
            note: "通过".into(),
            guards: Vec::new(),
            blockers: Vec::new(),
        },
    }
}

#[test]
fn can_submit_fails_closed_when_identity_evidence_is_missing() {
    let mut preview = ready_preview();
    preview.order_plans[0]
        .client_order_id_policy
        .constraints
        .clear();

    assert!(!preview.can_submit());
}

#[test]
fn legacy_non_binance_plans_still_use_existing_compile_blockers() {
    let mut preview = ready_preview();
    preview.order_plans = vec![
        legacy_plan(HedgeLegRole::Long, "kucoin"),
        legacy_plan(HedgeLegRole::Short, "gate"),
    ];
    preview.identity_evidence_required = false;

    assert!(preview.can_submit());

    preview.order_plans[1]
        .blockers
        .push("venue order type unsupported".into());
    assert!(!preview.can_submit());
}

fn identity_plan(role: HedgeLegRole) -> OrderCompilePlan {
    OrderCompilePlan {
        role,
        exchange: "binance".into(),
        symbol: "BTCUSDC".into(),
        client_order_id_policy: identity_policy(),
        product: shared_types::FeeProduct::Perp,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: OrderType::Limit,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Ioc,
        effective_time_in_force: TimeInForce::Ioc,
        available_order_types: vec![OrderType::Limit],
        available_time_in_force: vec![TimeInForce::Ioc],
        available_margin_modes: Vec::new(),
        venue_capability: shared_types::VenueSymbolCapability::default(),
        market_order_style: None,
        venue_order_kind: VenueOrderKind::Limit,
        payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
        reference_price: Some(100.0),
        protection_price: Some(100.0),
        payload_price: Some(100.0),
        slippage_tolerance_bps: None,
        summary: "Binance USD-M limit".into(),
        blockers: Vec::new(),
    }
}

fn legacy_plan(role: HedgeLegRole, exchange: &str) -> OrderCompilePlan {
    let mut plan = identity_plan(role);
    plan.exchange = exchange.into();
    plan.symbol = "BTCUSDTM".into();
    plan.client_order_id_policy.venue = exchange.into();
    plan.client_order_id_policy.venue_family = exchange.into();
    plan.client_order_id_policy.constraints.clear();
    plan
}

fn identity_policy() -> ClientOrderIdPolicy {
    let mut constraints = vec![
        "identity.canonical_symbol=BTCUSDC".into(),
        "identity.native_symbol=BTCUSDC".into(),
        "identity.settle_asset=USDC".into(),
        "identity.quote_asset=USDC".into(),
        "identity.product=perp".into(),
        "identity.exchange_order_id_finality_source=private_user_stream_with_rest_fallback".into(),
    ];
    for kind in ["metadata", "user_stream", "order_finality", "fee"] {
        constraints.push(format!("identity.evidence.{kind}.status=verified"));
        constraints.push(format!("identity.evidence.{kind}.evidence_id={kind}-1"));
        constraints.push(format!("identity.evidence.{kind}.source=binance-capture"));
    }
    ClientOrderIdPolicy {
        venue: "binance".into(),
        venue_family: "binance".into(),
        venue_field: "newClientOrderId".into(),
        public_client_order_id: "public-order-1".into(),
        venue_client_order_id: Some("venue-order-1".into()),
        derivation: ClientOrderIdDerivation::Identity,
        policy_version: "binance-usdm-v1".into(),
        official_format: "1..=36 ASCII".into(),
        max_length: Some(36),
        supports_query_by_client_id: true,
        supports_cancel_by_client_id: true,
        constraints,
        blockers: Vec::new(),
        official_doc_urls: Vec::new(),
    }
}

fn run_with_state(state: ExecutionRunState) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
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
        updated_at_ms: 2,
    }
}

fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".into(),
        symbol: "BTC-USDT".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Accepted,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
