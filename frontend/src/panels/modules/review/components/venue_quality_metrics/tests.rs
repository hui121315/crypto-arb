use super::*;

#[test]
fn summary_cards_do_not_treat_unsampled_rows_as_metrics() {
    let rows = vec![
        quality("empty", VenueQualitySampleStatus::NoSample, 0),
        quality("slow", VenueQualitySampleStatus::Ready, 120),
    ];

    let cards = summary_cards(&rows);
    let rest = cards.iter().find(|card| card.label == "已采样最慢 REST");

    assert_eq!(cards.len(), 4);
    assert_eq!(rest.map(|card| card.venue.as_str()), Some("slow"));
    assert_eq!(rest.map(|card| card.value.as_str()), Some("120ms"));
}

#[test]
fn summary_cards_omit_metrics_without_samples() {
    let rows = vec![quality("empty", VenueQualitySampleStatus::NoSample, 0)];

    let cards = summary_cards(&rows);

    assert!(cards.is_empty());
}

#[test]
fn risk_sort_keeps_unsampled_rows_last() {
    let rows = sorted_by_risk(vec![
        quality("empty", VenueQualitySampleStatus::NoSample, 0),
        quality("ready", VenueQualitySampleStatus::Ready, 20),
    ]);

    assert_eq!(rows.first().map(|row| row.venue.as_str()), Some("ready"));
    assert_eq!(rows.last().map(|row| row.venue.as_str()), Some("empty"));
}

#[test]
fn operation_summary_keeps_attention_and_retry_context() {
    let mut row = quality("binance", VenueQualitySampleStatus::WarmingUp, 4);
    let problem = shared_types::ApiProblem::new("HTTP_RATE_LIMITED", "rate limited")
        .with_request_id(Some("req-quality-operation".to_owned()))
        .with_retry_after_ms(Some(15_000));
    row.operation_health = vec![shared_types::VenueOperationHealth {
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
        observed_at_ms: 1_000,
    }];
    row.retry_after_ms = Some(15_000);
    row.last_problem = Some(problem);

    assert_eq!(operation_class(&row), "quality-warn");
    assert_eq!(operation_value(&row), "1 项 · 1 需关注 · retry 15000ms");
    assert_eq!(operation_status_label(VenueOperationStatus::Warn), "警告");
}

fn quality(venue: &str, sample_status: VenueQualitySampleStatus, samples: u32) -> VenueQuality {
    VenueQuality {
        venue: venue.to_owned(),
        source: "test".to_owned(),
        sample_status,
        avg_rest_latency_ms: samples,
        rest_latency_samples: samples.min(1),
        ws_jitter_p99_ms: samples,
        ws_jitter_samples: samples.min(1),
        fill_rate_pct: 99.0,
        fill_window_samples: samples.min(1),
        avg_slippage_bps: 1.0,
        slippage_samples: samples.min(1),
        uptime_window_pct: 99.5,
        uptime_window_samples: samples.min(1),
        sample_window: shared_types::VenueQualitySampleWindow::default(),
        operation_health: Vec::new(),
        retry_after_ms: None,
        last_problem: None,
    }
}
