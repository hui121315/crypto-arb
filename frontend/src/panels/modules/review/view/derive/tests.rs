use super::*;
use shared_types::{VenueOperationHealth, VenueQuality, VenueQualitySampleStatus};

#[test]
fn empty_execution_ledger_is_a_normal_empty_state() {
    let envelope = ReviewEnvelope::new(
        Vec::<u8>::new(),
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::NoLedgerEvents),
        Vec::new(),
    );

    let rows = review_rows(&LoadState::Ready(envelope.clone()));
    let meta = state_meta(&LoadState::Ready(envelope));

    assert_eq!(rows.empty_text("暂无复盘记录"), "暂无复盘记录");
    assert!(meta.contains("暂无交易记录"));
    assert!(!meta.contains("降级"));
    assert!(!meta.contains("失败"));
}

#[test]
fn review_rows_preserve_degraded_envelope_problem() {
    let envelope = ReviewEnvelope::new(
        Vec::<u8>::new(),
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::PartialEvidence),
        Vec::new(),
    )
    .with_page(
        ListPage {
            limit: 0,
            max_limit: 0,
            start_offset: 0,
            returned_count: 0,
            total_rows: 0,
            has_more: false,
            next_cursor: None,
            ..ListPage::default()
        },
        ListStatus::Degraded,
        vec![ApiProblem::new(
            "REVIEW_LEDGER_INCOMPLETE",
            "ledger incomplete",
        )],
    );

    let rows = review_rows(&LoadState::Ready(envelope));

    assert_eq!(
        rows.empty_text("等待策略绩效"),
        "上次快照为空，刷新失败：ledger incomplete"
    );
}

#[test]
fn state_meta_keeps_degraded_problem_context_with_rows() {
    let envelope = ReviewEnvelope::new(
        vec![1_u8],
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::PartialEvidence),
        Vec::new(),
    )
    .with_page(
        ListPage {
            limit: 30,
            max_limit: 100,
            start_offset: 0,
            returned_count: 1,
            total_rows: 4,
            has_more: true,
            next_cursor: Some("cursor-1".into()),
            ..ListPage::default()
        },
        ListStatus::Degraded,
        vec![
            ApiProblem::new("REVIEW_LEDGER_INCOMPLETE", "ledger degraded")
                .with_status(503)
                .with_request_id(Some("req-review-1".into()))
                .with_retry_after_ms(Some(2_000)),
            ApiProblem::new("REVIEW_STORAGE_STALE", "storage stale"),
        ],
    )
    .with_request_id(Some("req-review-body-1".to_owned()));

    let meta = state_meta(&LoadState::Ready(envelope));

    assert!(meta.contains("1/4 行"));
    assert!(meta.contains("还有下一页"));
    assert!(meta.contains("request_id req-review-body-1"));
    assert!(meta.contains(
        "分页降级：ledger degraded · HTTP 503 · request_id req-review-1 · retry 2000ms +1"
    ));
}

#[test]
fn venue_quality_meta_keeps_operation_retry_problem_and_request_context() {
    let problem = ApiProblem::new("HTTP_RATE_LIMITED", "rate limited")
        .with_request_id(Some("req-operation-1".to_owned()))
        .with_retry_after_ms(Some(15_000));
    let operation = VenueOperationHealth {
        venue: "binance".to_owned(),
        operation: "http_rest:GET /fapi/v1/depth".to_owned(),
        status: VenueOperationStatus::Warn,
        source: "exchange_http_metrics".to_owned(),
        message: "rate limited".to_owned(),
        supported: Some(true),
        configured: None,
        requested: Some(1),
        rows: Some(1),
        freshness_ms: Some(250),
        retry_after_ms: Some(15_000),
        latency_ms: Some(30),
        latency_p95_ms: Some(100),
        error: Some("rate limited".to_owned()),
        evidence: None,
        problem: Some(problem.clone()),
        observed_at_ms: 9_500,
    };
    let row = VenueQuality {
        venue: "binance".to_owned(),
        source: "exchange_http_metrics".to_owned(),
        sample_status: VenueQualitySampleStatus::WarmingUp,
        avg_rest_latency_ms: 30,
        rest_latency_samples: 4,
        ws_jitter_p99_ms: 0,
        ws_jitter_samples: 0,
        fill_rate_pct: 0.0,
        fill_window_samples: 0,
        avg_slippage_bps: 0.0,
        slippage_samples: 0,
        uptime_window_pct: 75.0,
        uptime_window_samples: 4,
        sample_window: shared_types::VenueQualitySampleWindow::default(),
        operation_health: vec![operation],
        retry_after_ms: Some(15_000),
        last_problem: Some(problem),
    };
    let envelope = VenueQualityEnvelope::new(vec![row], 10_000, VenueQualitySource::RuntimeSamples)
        .with_request_id(Some("req-quality-body-1".to_owned()));

    let meta = venue_quality_meta(&LoadState::Ready(envelope));

    assert!(meta.contains("1 operation"));
    assert!(meta.contains("1 需关注"));
    assert!(meta.contains("retry 15000ms"));
    assert!(meta.contains("request_id req-quality-body-1"));
    assert!(meta.contains("HTTP_RATE_LIMITED"));
    assert!(meta.contains("request_id req-operation-1"));
}

#[test]
fn state_meta_surfaces_funding_payment_ingest_report() {
    let mut report = FundingPaymentIngestReport {
        observed_at_ms: 1_700,
        mapped: 3,
        ledger_events: 1,
        skipped: 2,
        no_filled_anchor: 1,
        ambiguous_order_group: 1,
        unmatched_or_ambiguous_order: 2,
        ..FundingPaymentIngestReport::default()
    };
    report.refresh_skip_reasons();
    let envelope = ReviewEnvelope::new(
        vec![1_u8],
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::PartialEvidence),
        vec![ReviewPnlField::Funding],
    )
    .with_funding_payment_ingest(report);

    let meta = state_meta(&LoadState::Ready(envelope));

    assert!(meta.contains("资金费入账 1/3"));
    assert!(meta.contains("跳过 2"));
    assert!(meta.contains("缺成交锚点 1"));
    assert!(meta.contains("歧义 1"));
    assert!(meta.contains("首因 缺成交锚点:1"));
}

#[test]
fn venue_quality_chart_meta_surfaces_source_freshness_and_problem() {
    let state = LoadState::Stale {
        value: VenueQualityEnvelope::new(Vec::new(), 10_000, VenueQualitySource::RuntimeSamples),
        problem: ApiProblem::new("TIMEOUT", "timeout").with_request_id(Some("req-7".to_owned())),
    };

    let meta = venue_quality_chart_meta(&state, 12_500);

    assert_eq!(meta.source, "执行质量样本");
    assert_eq!(meta.freshness, "2.5s");
    assert_eq!(meta.problem.as_deref(), Some("timeout · request_id req-7"));
    assert_eq!(
        meta.label(),
        "执行质量样本 · 2.5s · 降级：timeout · request_id req-7"
    );
}
