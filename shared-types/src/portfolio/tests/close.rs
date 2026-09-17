use super::*;

#[test]
fn close_run_serializes_status_and_problem() {
    let run = CloseRun {
        id: "close-1".to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::PartiallySubmitted,
        action_run_id: Some("act-1".to_owned()),
        request_id: Some("req-1".to_owned()),
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 2,
        reason: Some("positions.close_pair".to_owned()),
        legs: vec![CloseLeg {
            venue: "binance".to_owned(),
            symbol: "MUUSDT".to_owned(),
            side: PositionSide::Long,
            status: CloseLegStatus::Failed,
            quantity: 1.0,
            mark_price: 10.0,
            notional_usd: 10.0,
            order: None,
            finality_source: None,
            confirmed_filled_at_ms: None,
            problem: Some(ApiProblem::new("CLOSE_RUN_FAILED", "failed")),
            pair_evidence: None,
            cost_events: Vec::new(),
        }],
        submitted_order_count: 1,
        failed_leg_count: 1,
        naked_exposure_usd: 10.0,
        message: "partial".to_owned(),
        problem: Some(ApiProblem::new("CLOSE_RUN_PARTIAL", "partial")),
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: Some(CloseRunCostReconciliation {
            close_fee_usd: Some(0.2),
            close_slippage_usd: Some(1.0),
            funding_usd: Some(-0.1),
            manual_handling_usd: Some(4.0),
            total_actual_cost_usd: Some(1.2),
            evidence_order_ids: vec!["order-1".to_owned()],
            evidence_event_ids: vec![
                "fee-1".to_owned(),
                "slippage-1".to_owned(),
                "funding-1".to_owned(),
                "manual-1".to_owned(),
            ],
            close_fee_event_ids: vec!["fee-1".to_owned()],
            close_slippage_event_ids: vec!["slippage-1".to_owned()],
            funding_event_ids: vec!["funding-1".to_owned()],
            manual_handling_event_ids: vec!["manual-1".to_owned()],
            ..CloseRunCostReconciliation::default()
        }),
        started_at_ms: 1,
        updated_at_ms: 2,
    };

    let text = serde_json::to_string(&run).expect("serialize close run");

    assert!(text.contains("\"partially_submitted\""));
    assert!(text.contains("\"actionRunId\":\"act-1\""));
    assert!(text.contains("\"requestId\":\"req-1\""));
    assert!(text.contains("\"nakedExposureUsd\":10.0"));
    assert!(text.contains("\"costReconciliation\""));
    assert!(text.contains("\"totalActualCostUsd\":1.2"));
    assert!(text.contains("\"closeSlippageEventIds\""));
}

#[test]
fn close_run_serializes_submitted_without_claiming_final_success() {
    let run = CloseRun {
        id: "close-1".to_owned(),
        scope: CloseRunScope::Single,
        status: CloseRunStatus::Submitted,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 1,
        reason: None,
        legs: Vec::new(),
        submitted_order_count: 1,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "submitted".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    };

    let text = serde_json::to_string(&run).expect("serialize close run");

    assert!(text.contains("\"status\":\"submitted\""));
    assert!(!text.contains("\"status\":\"succeeded\""));
}

#[test]
fn close_run_serializes_unwind_required() {
    let run = CloseRun {
        id: "close-1".to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::UnwindRequired,
        action_run_id: Some("act-1".to_owned()),
        request_id: Some("req-1".to_owned()),
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 2,
        reason: Some("positions.close_pair".to_owned()),
        legs: Vec::new(),
        submitted_order_count: 2,
        failed_leg_count: 1,
        naked_exposure_usd: 10.0,
        message: "unwind required".to_owned(),
        problem: Some(ApiProblem::new(
            "CLOSE_RUN_UNWIND_REQUIRED",
            "unwind required",
        )),
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: Some(CloseRunUnwindPlan {
            status: CloseRunUnwindPlanStatus::BlockedPendingManualRecheck,
            filled_legs: Vec::new(),
            failed_legs: Vec::new(),
            compensation_candidates: Vec::new(),
            remaining_positions: Vec::new(),
            compensation_attempts: Vec::new(),
            manual_terminal_evidence: None,
            next_actions: vec![CloseRunNextAction {
                kind: CloseRunNextActionKind::SubmitCompensationOrder,
                label: "提交补偿单".to_owned(),
                candidate_index: Some(0),
                requires_confirmation: true,
                required_evidence: vec!["fresh_position_snapshot".to_owned()],
                reason: Some("manual incident review required".to_owned()),
            }],
            required_evidence: vec!["fresh_position_snapshot".to_owned()],
        }),
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    };

    let text = serde_json::to_string(&run).expect("serialize close run");

    assert!(text.contains("\"status\":\"unwind_required\""));
    assert!(text.contains("\"code\":\"CLOSE_RUN_UNWIND_REQUIRED\""));
    assert!(text.contains("\"unwindPlan\""));
    assert!(text.contains("\"blocked_pending_manual_recheck\""));
    assert!(text.contains("\"nextActions\""));
    assert!(text.contains("\"submit_compensation_order\""));
}

#[test]
fn close_run_serializes_manual_terminal_evidence() {
    let run = CloseRun {
        id: "close-manual".to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::ManuallyResolved,
        action_run_id: Some("act-1".to_owned()),
        request_id: Some("req-1".to_owned()),
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 2,
        reason: Some("operator reviewed remote account".to_owned()),
        legs: Vec::new(),
        submitted_order_count: 2,
        failed_leg_count: 1,
        naked_exposure_usd: 100.0,
        message: "manual terminal recorded".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: Some(CloseRunUnwindPlan {
            status: CloseRunUnwindPlanStatus::ManualTerminalRecorded,
            filled_legs: Vec::new(),
            failed_legs: Vec::new(),
            compensation_candidates: Vec::new(),
            remaining_positions: vec![unwind_evidence()],
            compensation_attempts: Vec::new(),
            manual_terminal_evidence: Some(CloseRunManualTerminalEvidence {
                action_run_id: Some("act-1".to_owned()),
                actor: "api-token:abc".to_owned(),
                reason: "remote account flat".to_owned(),
                snapshot_version: "pos-1".to_owned(),
                recorded_at_ms: 10,
                remaining_positions: vec![unwind_evidence()],
                required_evidence: vec!["fresh_position_snapshot".to_owned()],
                evidence: vec!["operator-ticket-1".to_owned()],
                manual_handling_cost_usd: Some(12.5),
                manual_handling_event_id: Some("manual-cost-1".to_owned()),
            }),
            next_actions: Vec::new(),
            required_evidence: vec!["fresh_position_snapshot".to_owned()],
        }),
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 10,
    };

    let text = serde_json::to_string(&run).expect("serialize manual terminal close run");

    assert!(text.contains("\"manually_resolved\""));
    assert!(text.contains("\"manual_terminal_recorded\""));
    assert!(text.contains("\"manualTerminalEvidence\""));
    assert!(text.contains("\"operator-ticket-1\""));
}

#[test]
fn close_run_unwind_leg_evidence_serializes_notional_quality() {
    let evidence = unwind_evidence();

    let text = serde_json::to_string(&evidence).expect("serialize unwind evidence");

    assert!(text.contains("\"confirmedPrice\":100.0"));
    assert!(text.contains("\"notionalQuality\":\"actual\""));
    assert!(text.contains("\"notionalSource\":\"filled_quantity_x_filled_price\""));
    assert!(!text.contains("notionalMissingFields"));

    let decoded: CloseRunUnwindLegEvidence = serde_json::from_value(serde_json::json!({
        "venue": "binance",
        "symbol": "MUUSDT",
        "side": "long",
        "status": "filled",
        "targetQuantity": 1.0,
        "confirmedQuantity": 0.4,
        "markPrice": 99.0,
        "notionalUsd": 40.0
    }))
    .expect("legacy unwind evidence decodes");

    assert_eq!(decoded.confirmed_price, None);
    assert_eq!(decoded.notional_quality, ExecutionLedgerQuality::Estimated);
    assert_eq!(decoded.notional_source, "legacy_unclassified");
}

fn unwind_evidence() -> CloseRunUnwindLegEvidence {
    CloseRunUnwindLegEvidence {
        venue: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: PositionSide::Long,
        status: CloseLegStatus::Filled,
        target_quantity: 1.0,
        confirmed_quantity: Some(0.4),
        confirmed_price: Some(100.0),
        mark_price: 99.0,
        notional_usd: 40.0,
        notional_quality: ExecutionLedgerQuality::Actual,
        notional_source: "filled_quantity_x_filled_price".to_owned(),
        notional_missing_fields: Vec::new(),
        compensation_order_side: Some(OrderSide::Buy),
        order_id: Some("order-1".to_owned()),
        client_order_id: Some("client-1".to_owned()),
        exchange_order_id: Some("exchange-1".to_owned()),
        finality_source: Some(OrderUpdateSource::PrivateWs),
        confirmed_filled_at_ms: Some(42),
        problem: None,
    }
}

#[test]
fn close_run_event_uses_camel_case_shape() {
    let encoded = serde_json::to_value(CloseRunEvent {
        event: "close_run_updated".to_owned(),
        close_run: None,
        timestamp_ms: 42,
    })
    .expect("close run event encodes");

    assert_eq!(encoded["event"], "close_run_updated");
    assert!(encoded.get("closeRun").is_none());
    assert_eq!(encoded["timestampMs"], 42);

    let decoded: CloseRunEvent = serde_json::from_value(serde_json::json!({
        "event": "close_run_updated",
        "closeRun": null,
        "timestampMs": 43
    }))
    .expect("close run event decodes");

    assert_eq!(decoded.event, "close_run_updated");
    assert!(decoded.close_run.is_none());
    assert_eq!(decoded.timestamp_ms, 43);
}
