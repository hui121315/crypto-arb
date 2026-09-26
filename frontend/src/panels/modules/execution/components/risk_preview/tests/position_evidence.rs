use super::*;
use shared_types::ApiProblem;

#[test]
fn positions_evidence_summary_surfaces_degraded_fields() {
    let evidence = degraded_positions_evidence();

    let summary = positions_evidence_line(&evidence);
    let detail = operation_health_detail(&evidence.operation_health);

    assert!(summary.contains("Degraded"));
    assert!(summary.contains("1 字段数据待确认"));
    assert!(summary.contains("1 问题"));
    assert!(detail.contains("binance positions WARN"));
    assert!(detail.contains("retry 1000ms"));
}

#[test]
fn paper_preview_marks_private_positions_and_liquidation_as_not_required() {
    let mut preview = ready_preview(None);
    preview.liquidation.positions_evidence = Some(degraded_positions_evidence());

    assert_eq!(positions_evidence_summary(&preview), "模拟无需实盘数据依据");
    assert!(positions_evidence_detail(&preview).contains("模拟模式不读取"));
    assert_eq!(current_liq_value(&preview), "模拟无需");
    assert_eq!(current_liq_state(&preview), CheckItemState::Ok);
    assert!(!positions_evidence_needs_attention(&preview));
}

#[test]
fn live_preview_keeps_degraded_private_positions_blocking() {
    let mut preview = ready_preview(None);
    preview.execution_mode_label = "实盘";
    preview.liquidation.positions_evidence = Some(degraded_positions_evidence());

    assert!(positions_evidence_summary(&preview).contains("Degraded"));
    assert_eq!(current_liq_value(&preview), "数据待确认");
    assert_eq!(current_liq_state(&preview), CheckItemState::Block);
    assert!(positions_evidence_needs_attention(&preview));
}

fn degraded_positions_evidence() -> HedgePreviewPositionsEvidence {
    HedgePreviewPositionsEvidence {
        status: ListStatus::Degraded,
        source: "account_position_runtime".into(),
        observed_at_ms: 1,
        row_count: 1,
        current_account_liq_distance_pct: None,
        problems: vec![
            ApiProblem::new("POSITION_FIELD_UNAVAILABLE", "missing liquidation").with_status(200),
        ],
        operation_health: vec![VenueOperationHealth {
            venue: "binance".into(),
            operation: "positions".into(),
            status: VenueOperationStatus::Warn,
            source: "test".into(),
            message: "stale".into(),
            supported: Some(true),
            configured: Some(true),
            requested: None,
            rows: Some(1),
            freshness_ms: Some(5_000),
            retry_after_ms: Some(1_000),
            latency_ms: None,
            latency_p95_ms: None,
            error: None,
            evidence: None,
            problem: None,
            observed_at_ms: 1,
        }],
        field_quality: vec![AccountFieldQuality::new(
            AccountFieldSubject::position("binance", "BTCUSDT", "long"),
            "liquidationDistancePct",
            AccountFieldQualityStatus::Missing,
            "account_position_runtime",
            Some(1),
        )],
        row_health: Vec::new(),
        account_bindings: Vec::new(),
        request_id: Some("request-positions".into()),
        retry_after_ms: Some(1_000),
    }
}
