use exchange::HttpQualityWindowSnapshot;
use shared_types::{
    normalized_venue_name, problem::codes, ApiProblem, VenueOperationHealth, VenueOperationStatus,
    VenueQuality, VenueQualitySampleStatus, VenueQualitySampleWindow,
    VENUE_QUALITY_READY_SAMPLE_MIN,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn quality_rows(
    venue_names: Vec<String>,
    tracker_rows: Vec<VenueQuality>,
    http_windows: Vec<HttpQualityWindowSnapshot>,
    operation_rows: Vec<VenueOperationHealth>,
) -> Vec<VenueQuality> {
    let mut rows = venue_names
        .into_iter()
        .map(|venue| (normalized_venue_name(&venue), neutral_quality(venue)))
        .collect::<BTreeMap<_, _>>();
    let mut tracker_keys = BTreeSet::new();
    for row in tracker_rows {
        let key = normalized_venue_name(&row.venue);
        if quality_sample_count(&row) > 0 {
            tracker_keys.insert(key.clone());
        }
        rows.insert(key, row);
    }

    let mut http_keys = BTreeSet::new();
    for window in http_windows {
        let key = normalized_venue_name(&window.exchange);
        let Some(row) = rows.get_mut(&key) else {
            continue;
        };
        apply_http_window(row, &window);
        http_keys.insert(key);
    }

    let known_venues = rows.keys().cloned().collect();
    let mut operations = group_operations(operation_rows, &known_venues);
    for (key, row) in &mut rows {
        row.operation_health = operations.remove(key).unwrap_or_default();
        apply_operation_context(row);
        row.sample_status = sample_status(quality_sample_count(row));
        row.source = source_label(tracker_keys.contains(key), http_keys.contains(key)).to_owned();
    }

    rows.into_values().collect()
}

fn neutral_quality(venue: String) -> VenueQuality {
    VenueQuality {
        venue,
        source: "no_sample".to_owned(),
        sample_status: VenueQualitySampleStatus::NoSample,
        avg_rest_latency_ms: 0,
        rest_latency_samples: 0,
        ws_jitter_p99_ms: 0,
        ws_jitter_samples: 0,
        fill_rate_pct: 0.0,
        fill_window_samples: 0,
        avg_slippage_bps: 0.0,
        slippage_samples: 0,
        uptime_window_pct: 0.0,
        uptime_window_samples: 0,
        sample_window: VenueQualitySampleWindow::default(),
        operation_health: Vec::new(),
        retry_after_ms: None,
        last_problem: None,
    }
}

fn apply_http_window(row: &mut VenueQuality, window: &HttpQualityWindowSnapshot) {
    row.rest_latency_samples = window.request_total;
    row.uptime_window_samples = window.request_total;
    row.sample_window.oldest_operation_observed_at_ms = window.oldest_observed_at_ms;
    row.sample_window.latest_operation_observed_at_ms = window.latest_observed_at_ms;
    if window.request_total == 0 {
        return;
    }
    row.avg_rest_latency_ms = saturating_u32(
        window
            .latency_ms_total
            .checked_div(u64::from(window.request_total))
            .unwrap_or(0),
    );
    row.uptime_window_pct =
        f64::from(window.success_total) / f64::from(window.request_total) * 100.0;
}

fn group_operations(
    operations: Vec<VenueOperationHealth>,
    known_venues: &BTreeSet<String>,
) -> BTreeMap<String, Vec<VenueOperationHealth>> {
    let mut grouped = BTreeMap::<String, Vec<VenueOperationHealth>>::new();
    for operation in operations {
        let key = normalized_venue_name(&operation.venue);
        if known_venues.contains(&key) {
            grouped.entry(key).or_default().push(operation);
        }
    }
    for rows in grouped.values_mut() {
        rows.sort_by(|left, right| {
            left.operation
                .cmp(&right.operation)
                .then_with(|| left.source.cmp(&right.source))
        });
    }
    grouped
}

fn apply_operation_context(row: &mut VenueQuality) {
    row.retry_after_ms = row
        .operation_health
        .iter()
        .flat_map(operation_retry_after_ms)
        .max();
    let operation_oldest = row
        .operation_health
        .iter()
        .map(|operation| operation.observed_at_ms)
        .filter(|observed_at_ms| *observed_at_ms > 0)
        .min();
    let operation_latest = row
        .operation_health
        .iter()
        .map(|operation| operation.observed_at_ms)
        .filter(|observed_at_ms| *observed_at_ms > 0)
        .max();
    row.sample_window.oldest_operation_observed_at_ms = min_observed(
        row.sample_window.oldest_operation_observed_at_ms,
        operation_oldest,
    );
    row.sample_window.latest_operation_observed_at_ms = max_observed(
        row.sample_window.latest_operation_observed_at_ms,
        operation_latest,
    );
    row.last_problem = row
        .operation_health
        .iter()
        .filter(|operation| operation.status != VenueOperationStatus::Ok)
        .max_by_key(|operation| operation.observed_at_ms)
        .map(operation_problem);
}

fn operation_retry_after_ms(operation: &VenueOperationHealth) -> impl Iterator<Item = u64> + '_ {
    operation.retry_after_ms.into_iter().chain(
        operation
            .problem
            .as_ref()
            .and_then(|problem| problem.retry_after_ms),
    )
}

fn operation_problem(operation: &VenueOperationHealth) -> ApiProblem {
    if let Some(problem) = operation.problem.clone() {
        return problem;
    }
    let mut problem = ApiProblem::new(
        codes::VENUE_QUALITY_OPERATION_DEGRADED,
        operation
            .error
            .clone()
            .unwrap_or_else(|| operation.message.clone()),
    )
    .with_source(operation.source.clone())
    .with_retry_after_ms(operation.retry_after_ms)
    .with_request_id(
        operation
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.request_id.clone()),
    );
    problem.details = Some(serde_json::json!({
        "venue": operation.venue,
        "operation": operation.operation,
        "status": operation.status,
        "latencyMs": operation.latency_ms,
        "latencyP95Ms": operation.latency_p95_ms,
        "observedAtMs": operation.observed_at_ms,
    }));
    problem
}

fn quality_sample_count(row: &VenueQuality) -> u64 {
    u64::from(row.rest_latency_samples.max(row.uptime_window_samples))
        .saturating_add(u64::from(row.ws_jitter_samples))
        .saturating_add(u64::from(row.fill_window_samples.max(row.slippage_samples)))
}

fn sample_status(sample_count: u64) -> VenueQualitySampleStatus {
    if sample_count == 0 {
        VenueQualitySampleStatus::NoSample
    } else if sample_count < u64::from(VENUE_QUALITY_READY_SAMPLE_MIN) {
        VenueQualitySampleStatus::WarmingUp
    } else {
        VenueQualitySampleStatus::Ready
    }
}

fn source_label(has_tracker: bool, has_http: bool) -> &'static str {
    match (has_tracker, has_http) {
        (true, true) => "tracker+exchange_http_metrics",
        (true, false) => "tracker",
        (false, true) => "exchange_http_metrics",
        (false, false) => "no_sample",
    }
}

fn saturating_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn min_observed(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    left.into_iter().chain(right).min()
}

fn max_observed(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    left.into_iter().chain(right).max()
}
