use super::*;
use exchange::HttpQualityWindowSnapshot;
use shared_types::{
    ApiProblem, VenueOperationHealth, VenueOperationStatus, VenueQuality, VenueQualitySampleStatus,
    VenueQualitySampleWindow,
};

#[test]
fn neutral_snapshot_is_unsampled_not_perfect() {
    let rows = quality_rows(vec!["okx".to_owned()], Vec::new(), Vec::new(), Vec::new());

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].sample_status, VenueQualitySampleStatus::NoSample);
    assert_eq!(rows[0].fill_rate_pct, 0.0);
    assert_eq!(rows[0].uptime_window_pct, 0.0);
    assert_eq!(rows[0].rest_latency_samples, 0);
}

#[test]
fn real_http_outcomes_are_venue_scoped_and_keep_operation_context() -> Result<(), &'static str> {
    let rows = quality_rows(
        vec!["binance".to_owned(), "okx".to_owned()],
        vec![tracker_quality("binance")],
        vec![http_window("binance", 4, 3, 120, 100, 110)],
        vec![operation("binance", VenueOperationStatus::Warn, 110)],
    );
    let binance = rows
        .iter()
        .find(|row| row.venue == "binance")
        .ok_or("missing binance quality row")?;
    let okx = rows
        .iter()
        .find(|row| row.venue == "okx")
        .ok_or("missing okx quality row")?;

    assert_eq!(binance.avg_rest_latency_ms, 30);
    assert_eq!(binance.rest_latency_samples, 4);
    assert_eq!(binance.uptime_window_pct, 75.0);
    assert_eq!(binance.operation_health.len(), 1);
    assert_eq!(binance.retry_after_ms, Some(2_000));
    assert_eq!(
        binance
            .last_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("HTTP_RATE_LIMITED")
    );
    assert_eq!(
        binance.sample_window.latest_operation_observed_at_ms,
        Some(110)
    );
    assert_eq!(okx.rest_latency_samples, 0);
    assert_eq!(okx.avg_rest_latency_ms, 0);
    assert_eq!(okx.sample_status, VenueQualitySampleStatus::NoSample);
    Ok(())
}

#[test]
fn envelope_keeps_request_operation_and_retry_totals() {
    let mut row = tracker_quality("binance");
    row.operation_health = vec![operation("binance", VenueOperationStatus::Warn, 110)];
    row.retry_after_ms = Some(2_000);
    let envelope = VenueQualityEnvelope::new(vec![row], 120, VenueQualitySource::RuntimeSamples)
        .with_request_id(Some("req-quality-1".to_owned()));

    assert_eq!(envelope.operation_count, 1);
    assert_eq!(envelope.attention_count, 1);
    assert_eq!(envelope.retry_after_ms, Some(2_000));
    assert_eq!(envelope.request_id.as_deref(), Some("req-quality-1"));
}

fn tracker_quality(venue: &str) -> VenueQuality {
    VenueQuality {
        venue: venue.to_owned(),
        source: "tracker".to_owned(),
        sample_status: VenueQualitySampleStatus::WarmingUp,
        avg_rest_latency_ms: 0,
        rest_latency_samples: 0,
        ws_jitter_p99_ms: 10,
        ws_jitter_samples: 2,
        fill_rate_pct: 50.0,
        fill_window_samples: 2,
        avg_slippage_bps: 1.0,
        slippage_samples: 1,
        uptime_window_pct: 0.0,
        uptime_window_samples: 0,
        sample_window: VenueQualitySampleWindow::default(),
        operation_health: Vec::new(),
        retry_after_ms: None,
        last_problem: None,
    }
}

fn http_window(
    exchange: &str,
    request_total: u32,
    success_total: u32,
    latency_ms_total: u64,
    oldest_observed_at_ms: i64,
    latest_observed_at_ms: i64,
) -> HttpQualityWindowSnapshot {
    HttpQualityWindowSnapshot {
        exchange: exchange.to_owned(),
        request_total,
        success_total,
        latency_ms_total,
        oldest_observed_at_ms: Some(oldest_observed_at_ms),
        latest_observed_at_ms: Some(latest_observed_at_ms),
    }
}

fn operation(
    venue: &str,
    status: VenueOperationStatus,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: "http_rest:GET /quality".to_owned(),
        status,
        source: "exchange_http_metrics".to_owned(),
        message: "rate limited".to_owned(),
        supported: Some(true),
        configured: None,
        requested: None,
        rows: Some(1),
        freshness_ms: Some(0),
        retry_after_ms: Some(2_000),
        latency_ms: Some(30),
        latency_p95_ms: Some(100),
        error: Some("rate limited".to_owned()),
        evidence: None,
        problem: Some(
            ApiProblem::new("HTTP_RATE_LIMITED", "rate limited")
                .with_request_id(Some("req-http-quality".to_owned()))
                .with_retry_after_ms(Some(2_000))
                .with_source("exchange_http_metrics"),
        ),
        observed_at_ms,
    }
}
