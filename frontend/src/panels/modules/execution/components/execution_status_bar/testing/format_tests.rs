use super::super::format::{
    actual_cost_text, delta_cost_text, funding_actual_text, leg_evidence_text,
    open_actual_cost_text, timeline_event_class, timeline_event_meta, timeline_event_title,
};
use shared_types::{
    ExecutionFillConfidence, ExecutionLedgerEventType, ExecutionRunEventKind, ExecutionRunLeg,
    ExecutionRunLegEvidence, ExecutionRunState, ExecutionRunTimelineEvent, HedgeLegRole,
    LiveOrderState, OrderUpdateSource,
};

#[test]
fn leg_evidence_text_surfaces_finality_source_and_fee() {
    let mut leg = leg(HedgeLegRole::Long);
    let mut evidence = ExecutionRunLegEvidence::new(HedgeLegRole::Long);
    evidence.finality_confidence = ExecutionFillConfidence::VenueFill;
    leg.finality_source = Some(OrderUpdateSource::PrivateWs);
    leg.confirmed_filled_at_ms = Some(1_800_000);
    leg.filled_fee = Some(1.25);

    let text = leg_evidence_text(&leg, &evidence);

    assert!(text.contains("终态 私有WS"));
    assert!(text.contains("置信 交易所成交"));
    assert!(text.contains("确认"));
    assert!(text.contains("fee $1"));
}

#[test]
fn leg_evidence_text_prefers_native_instrument_and_precision() -> Result<(), serde_json::Error> {
    let leg = leg(HedgeLegRole::Long);
    let mut evidence = ExecutionRunLegEvidence::new(HedgeLegRole::Long);
    evidence.compile_plan = serde_json::from_value(serde_json::json!({
        "role": "long",
        "exchange": "kucoin",
        "symbol": "BTC-USDC",
        "requestedOrderType": "limit",
        "effectiveOrderType": "limit",
        "requestedTimeInForce": "ioc",
        "effectiveTimeInForce": "ioc",
        "venueOrderKind": "limit",
        "payloadPricePolicy": "limit_price",
        "summary": "ticket-bound plan",
        "instrumentSpec": {
            "venue": "kucoin",
            "nativeSymbol": "XBTUSDCM",
            "canonicalSymbol": "BTC-USDC",
            "displaySymbol": "BTC-USDC Perp",
            "assetClass": "crypto",
            "contractSize": 0.001,
            "priceTick": 0.1,
            "qtyStep": 1.0,
            "listingStatus": "trading",
            "source": "official_endpoint",
            "checkedAtMs": 1
        },
        "venueCapability": {
            "venue": "kucoin",
            "symbol": "XBTUSDCM",
            "source": "ticket_bound_capability_registry"
        }
    }))?;

    let text = leg_evidence_text(&leg, &evidence);

    assert!(text.contains("native XBTUSDCM"));
    assert!(text.contains("精度 tick 0.1 / step 1 / contract 0.001"));
    assert!(text.contains("能力 ticket_bound_capability_registry"));
    assert!(!text.contains("native BTC-USDC"));
    Ok(())
}

#[test]
fn timeline_event_keeps_source_confidence_and_request_context() {
    let event = ExecutionRunTimelineEvent {
        event_id: "fill-1".into(),
        kind: ExecutionRunEventKind::Fill,
        state: ExecutionRunState::Hedged,
        source: OrderUpdateSource::PrivateWs,
        message: "双腿成交".into(),
        occurred_at_ms: 1_800_000,
        request_id: Some("req-fill-1".into()),
        leg_role: Some(HedgeLegRole::Short),
        order_identity: None,
        ledger_event_type: Some(ExecutionLedgerEventType::FillEvent),
        finality_confidence: ExecutionFillConfidence::VenueFill,
        problem: None,
    };

    assert_eq!(timeline_event_title(&event), "空腿 成交");
    let meta = timeline_event_meta(&event);
    assert!(meta.contains("私有WS"));
    assert!(meta.contains("成交事件"));
    assert!(meta.contains("交易所成交"));
    assert!(meta.contains("request_id req-fill-1"));
    assert_eq!(timeline_event_class(&event), "execution-timeline-row done");
}

#[test]
fn cost_text_keeps_open_actual_separate_from_total_actual() {
    assert_eq!(open_actual_cost_text(Some(2.0)), "开仓真实 $2");
    assert_eq!(funding_actual_text(None, 0, false), "资金费待账本");
    assert_eq!(
        funding_actual_text(Some(-0.12), 1, false),
        "资金费 -$0.12 · 1 条事件"
    );
    assert_eq!(
        actual_cost_text(None, false),
        "真实总成本待平仓事实源".to_owned()
    );
    assert_eq!(delta_cost_text(None, false), "差异待平仓".to_owned());
}

#[test]
fn closed_cost_text_points_to_realized_review_ledger() {
    assert_eq!(funding_actual_text(None, 0, true), "资金费见复盘");
    assert_eq!(actual_cost_text(None, true), "真实总成本见复盘");
    assert_eq!(delta_cost_text(None, true), "成本差异见复盘");
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
        state: LiveOrderState::Created,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
