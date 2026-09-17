//! Lightweight outbound HTTP counters for exchange REST calls.

use crate::venue_spec::{endpoint_evidence, EndpointEvidenceSnapshot, HttpMethod};
use arc_swap::ArcSwapOption;
use dashmap::DashMap;
use shared_types::{normalized_venue_name, VENUE_QUALITY_WINDOW_MAX_SAMPLES};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

const HTTP_LATENCY_BUCKETS_MS: [u64; 10] = [10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestMetricSnapshot {
    pub exchange: String,
    pub method: String,
    pub path: String,
    pub request_total: u64,
    pub weight_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpOutcomeMetricSnapshot {
    pub exchange: String,
    pub method: String,
    pub path: String,
    pub endpoint_evidence: Option<EndpointEvidenceSnapshot>,
    pub outcome: String,
    pub status_code: Option<u16>,
    pub request_total: u64,
    pub retry_total: u64,
    pub latency_ms_total: u64,
    pub retry_after_ms_total: u64,
    pub latency_buckets: Vec<HttpLatencyBucketSnapshot>,
    pub latency_p95_ms: Option<u64>,
    pub last_latency_ms: u64,
    pub last_retry_after_ms: Option<u64>,
    pub last_request_id: Option<String>,
    pub last_request_context: Vec<String>,
    pub last_observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpLatencyBucketSnapshot {
    pub le_ms: u64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpQualityWindowSnapshot {
    pub exchange: String,
    pub request_total: u32,
    pub success_total: u32,
    pub latency_ms_total: u64,
    pub oldest_observed_at_ms: Option<i64>,
    pub latest_observed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HttpOutcomeSample<'a> {
    pub(crate) exchange: &'a str,
    pub(crate) method: &'a str,
    pub(crate) path: &'a str,
    pub(crate) outcome: &'a str,
    pub(crate) status_code: Option<u16>,
    pub(crate) latency_ms: u64,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) request_id: Option<&'a str>,
    pub(crate) request_context: &'a [String],
    pub(crate) retry: bool,
    pub(crate) observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HttpRequestMetricKey {
    exchange: String,
    method: String,
    path: String,
}

impl HttpRequestMetricKey {
    fn new(exchange: &str, method: &str, path: &str) -> Self {
        Self {
            exchange: exchange.to_owned(),
            method: method.to_owned(),
            path: path.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HttpOutcomeMetricKey {
    exchange: String,
    method: String,
    path: String,
    outcome: String,
    status_code: Option<u16>,
}

impl HttpOutcomeMetricKey {
    fn new(sample: &HttpOutcomeSample<'_>) -> Self {
        Self {
            exchange: sample.exchange.to_owned(),
            method: sample.method.to_owned(),
            path: sample.path.to_owned(),
            outcome: sample.outcome.to_owned(),
            status_code: sample.status_code,
        }
    }
}

#[derive(Debug, Default)]
struct HttpRequestMetric {
    request_total: AtomicU64,
    weight_total: AtomicU64,
}

struct HttpOutcomeMetric {
    request_total: AtomicU64,
    retry_total: AtomicU64,
    latency_ms_total: AtomicU64,
    retry_after_ms_total: AtomicU64,
    latency_buckets: [AtomicU64; HTTP_LATENCY_BUCKETS_MS.len()],
    last_latency_ms: AtomicU64,
    last_retry_after_ms: AtomicU64,
    last_request_id: ArcSwapOption<String>,
    last_request_context: ArcSwapOption<Vec<String>>,
    last_observed_at_ms: AtomicI64,
}

#[derive(Debug, Clone, Copy)]
struct HttpQualitySample {
    success: bool,
    latency_ms: u64,
    observed_at_ms: i64,
}

#[derive(Debug, Default)]
struct HttpQualityWindow {
    samples: VecDeque<HttpQualitySample>,
}

impl Default for HttpOutcomeMetric {
    fn default() -> Self {
        Self {
            request_total: AtomicU64::new(0),
            retry_total: AtomicU64::new(0),
            latency_ms_total: AtomicU64::new(0),
            retry_after_ms_total: AtomicU64::new(0),
            latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            last_latency_ms: AtomicU64::new(0),
            last_retry_after_ms: AtomicU64::new(0),
            last_request_id: ArcSwapOption::empty(),
            last_request_context: ArcSwapOption::empty(),
            last_observed_at_ms: AtomicI64::new(0),
        }
    }
}

static HTTP_REQUEST_METRICS: OnceLock<DashMap<HttpRequestMetricKey, HttpRequestMetric>> =
    OnceLock::new();
static HTTP_OUTCOME_METRICS: OnceLock<DashMap<HttpOutcomeMetricKey, HttpOutcomeMetric>> =
    OnceLock::new();
static HTTP_QUALITY_WINDOWS: OnceLock<DashMap<String, HttpQualityWindow>> = OnceLock::new();

pub(crate) fn record_http_request(exchange: &str, method: &str, path: &str, weight: u32) {
    let entry = metrics().entry(HttpRequestMetricKey::new(exchange, method, path));
    let row = entry.or_default();
    row.request_total.fetch_add(1, Ordering::Relaxed);
    row.weight_total
        .fetch_add(u64::from(weight), Ordering::Relaxed);
}

pub(crate) fn record_http_outcome(sample: HttpOutcomeSample<'_>) {
    let entry = outcome_metrics().entry(HttpOutcomeMetricKey::new(&sample));
    let row = entry.or_default();
    row.request_total.fetch_add(1, Ordering::Relaxed);
    if sample.retry {
        row.retry_total.fetch_add(1, Ordering::Relaxed);
    }
    row.latency_ms_total
        .fetch_add(sample.latency_ms, Ordering::Relaxed);
    row.retry_after_ms_total
        .fetch_add(sample.retry_after_ms.unwrap_or(0), Ordering::Relaxed);
    record_latency_buckets(&row, sample.latency_ms);
    row.last_latency_ms
        .store(sample.latency_ms, Ordering::Relaxed);
    row.last_retry_after_ms
        .store(sample.retry_after_ms.unwrap_or(0), Ordering::Relaxed);
    row.last_request_id.store(
        sample
            .request_id
            .map(|request_id| Arc::new(request_id.to_owned())),
    );
    row.last_request_context.store(
        (!sample.request_context.is_empty()).then(|| Arc::new(sample.request_context.to_vec())),
    );
    row.last_observed_at_ms
        .store(sample.observed_at_ms, Ordering::Relaxed);
    record_quality_sample(&sample);
}

pub fn http_request_metrics_snapshot() -> Vec<HttpRequestMetricSnapshot> {
    let mut rows = metrics()
        .iter()
        .map(|entry| {
            let key = entry.key();
            let value = entry.value();
            HttpRequestMetricSnapshot {
                exchange: key.exchange.clone(),
                method: key.method.clone(),
                path: key.path.clone(),
                request_total: value.request_total.load(Ordering::Relaxed),
                weight_total: value.weight_total.load(Ordering::Relaxed),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.exchange
            .cmp(&right.exchange)
            .then_with(|| left.method.cmp(&right.method))
            .then_with(|| left.path.cmp(&right.path))
    });
    rows
}

pub fn http_outcome_metrics_snapshot() -> Vec<HttpOutcomeMetricSnapshot> {
    let mut rows = outcome_metrics()
        .iter()
        .map(|entry| {
            let key = entry.key();
            let value = entry.value();
            HttpOutcomeMetricSnapshot {
                exchange: key.exchange.clone(),
                method: key.method.clone(),
                path: key.path.clone(),
                endpoint_evidence: http_endpoint_evidence(key),
                outcome: key.outcome.clone(),
                status_code: key.status_code,
                request_total: value.request_total.load(Ordering::Relaxed),
                retry_total: value.retry_total.load(Ordering::Relaxed),
                latency_ms_total: value.latency_ms_total.load(Ordering::Relaxed),
                retry_after_ms_total: value.retry_after_ms_total.load(Ordering::Relaxed),
                latency_buckets: latency_bucket_snapshot(value),
                latency_p95_ms: latency_p95_ms(value),
                last_latency_ms: value.last_latency_ms.load(Ordering::Relaxed),
                last_retry_after_ms: non_zero(value.last_retry_after_ms.load(Ordering::Relaxed)),
                last_request_id: value
                    .last_request_id
                    .load_full()
                    .map(|request_id| request_id.as_ref().clone()),
                last_request_context: value
                    .last_request_context
                    .load_full()
                    .map(|context| context.as_ref().clone())
                    .unwrap_or_default(),
                last_observed_at_ms: value.last_observed_at_ms.load(Ordering::Relaxed),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.exchange
            .cmp(&right.exchange)
            .then_with(|| left.method.cmp(&right.method))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.outcome.cmp(&right.outcome))
            .then_with(|| left.status_code.cmp(&right.status_code))
    });
    rows
}

pub fn http_quality_window_snapshot() -> Vec<HttpQualityWindowSnapshot> {
    let mut rows = quality_windows()
        .iter()
        .map(|entry| quality_window_snapshot(entry.key().clone(), &entry.value().samples))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.exchange.cmp(&right.exchange));
    rows
}

fn http_endpoint_evidence(key: &HttpOutcomeMetricKey) -> Option<EndpointEvidenceSnapshot> {
    let method = HttpMethod::parse(&key.method)?;
    endpoint_evidence(&key.exchange, method, &key.path)
}

fn metrics() -> &'static DashMap<HttpRequestMetricKey, HttpRequestMetric> {
    HTTP_REQUEST_METRICS.get_or_init(DashMap::new)
}

fn outcome_metrics() -> &'static DashMap<HttpOutcomeMetricKey, HttpOutcomeMetric> {
    HTTP_OUTCOME_METRICS.get_or_init(DashMap::new)
}

fn quality_windows() -> &'static DashMap<String, HttpQualityWindow> {
    HTTP_QUALITY_WINDOWS.get_or_init(DashMap::new)
}

fn record_quality_sample(sample: &HttpOutcomeSample<'_>) {
    let mut window = quality_windows()
        .entry(normalized_venue_name(sample.exchange))
        .or_default();
    if window.samples.len() >= VENUE_QUALITY_WINDOW_MAX_SAMPLES as usize {
        window.samples.pop_front();
    }
    window.samples.push_back(HttpQualitySample {
        success: sample.outcome == "success",
        latency_ms: sample.latency_ms,
        observed_at_ms: sample.observed_at_ms,
    });
}

fn quality_window_snapshot(
    exchange: String,
    samples: &VecDeque<HttpQualitySample>,
) -> HttpQualityWindowSnapshot {
    HttpQualityWindowSnapshot {
        exchange,
        request_total: samples.len() as u32,
        success_total: samples.iter().filter(|sample| sample.success).count() as u32,
        latency_ms_total: samples
            .iter()
            .map(|sample| sample.latency_ms)
            .fold(0_u64, u64::saturating_add),
        oldest_observed_at_ms: observed_at(
            samples.iter().map(|sample| sample.observed_at_ms).min(),
        ),
        latest_observed_at_ms: observed_at(
            samples.iter().map(|sample| sample.observed_at_ms).max(),
        ),
    }
}

fn observed_at(value: Option<i64>) -> Option<i64> {
    value.filter(|value| *value > 0)
}

fn non_zero(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

fn record_latency_buckets(row: &HttpOutcomeMetric, latency_ms: u64) {
    for (bucket, counter) in HTTP_LATENCY_BUCKETS_MS
        .iter()
        .zip(row.latency_buckets.iter())
    {
        if latency_ms <= *bucket {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn latency_bucket_snapshot(row: &HttpOutcomeMetric) -> Vec<HttpLatencyBucketSnapshot> {
    HTTP_LATENCY_BUCKETS_MS
        .iter()
        .zip(row.latency_buckets.iter())
        .map(|(le_ms, counter)| HttpLatencyBucketSnapshot {
            le_ms: *le_ms,
            count: counter.load(Ordering::Relaxed),
        })
        .collect()
}

fn latency_p95_ms(row: &HttpOutcomeMetric) -> Option<u64> {
    let total = row.request_total.load(Ordering::Relaxed);
    if total == 0 {
        return None;
    }
    let target = total.saturating_mul(95).div_ceil(100);
    for (bucket, counter) in HTTP_LATENCY_BUCKETS_MS
        .iter()
        .zip(row.latency_buckets.iter())
    {
        if counter.load(Ordering::Relaxed) >= target {
            return Some(*bucket);
        }
    }
    Some(*HTTP_LATENCY_BUCKETS_MS.last().unwrap_or(&0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_request_and_weight_totals() {
        let exchange = "unit_http_metrics";
        let path = "/unit/path";

        record_http_request(exchange, "GET", path, 3);
        record_http_request(exchange, "GET", path, 7);

        let rows = http_request_metrics_snapshot();
        let Some(row) = rows
            .iter()
            .find(|row| row.exchange == exchange && row.path == path)
        else {
            panic!("missing unit http metric row");
        };

        assert_eq!(row.request_total, 2);
        assert_eq!(row.weight_total, 10);
    }

    #[test]
    fn records_outcome_latency_and_retry_after_totals() {
        let exchange = "unit_http_outcome";
        let path = "/unit/outcome";

        record_http_outcome(HttpOutcomeSample {
            exchange,
            method: "GET",
            path,
            outcome: "rate_limited",
            status_code: Some(429),
            latency_ms: 12,
            retry_after_ms: Some(2_000),
            request_id: Some("req-old"),
            request_context: &["symbol=OLDUSDT".to_owned()],
            retry: true,
            observed_at_ms: 10,
        });
        record_http_outcome(HttpOutcomeSample {
            exchange,
            method: "GET",
            path,
            outcome: "rate_limited",
            status_code: Some(429),
            latency_ms: 8,
            retry_after_ms: Some(1_000),
            request_id: Some("req-new"),
            request_context: &["symbol=NEWUSDT".to_owned()],
            retry: true,
            observed_at_ms: 20,
        });

        let rows = http_outcome_metrics_snapshot();
        let Some(row) = rows
            .iter()
            .find(|row| row.exchange == exchange && row.path == path)
        else {
            panic!("missing unit http outcome metric row");
        };

        assert_eq!(row.request_total, 2);
        assert_eq!(row.retry_total, 2);
        assert_eq!(row.latency_ms_total, 20);
        assert_eq!(row.retry_after_ms_total, 3_000);
        assert_eq!(row.status_code, Some(429));
        assert!(row.endpoint_evidence.is_none());
        assert_eq!(row.latency_p95_ms, Some(25));
        assert!(row
            .latency_buckets
            .iter()
            .any(|bucket| bucket.le_ms == 25 && bucket.count == 2));
        assert_eq!(row.last_latency_ms, 8);
        assert_eq!(row.last_retry_after_ms, Some(1_000));
        assert_eq!(row.last_request_id.as_deref(), Some("req-new"));
        assert_eq!(row.last_request_context, vec!["symbol=NEWUSDT"]);
        assert_eq!(row.last_observed_at_ms, 20);
    }

    #[test]
    fn outcome_snapshot_attaches_endpoint_spec_evidence() {
        let exchange = "unit_http_binance";
        let path = "/fapi/v1/depth";

        record_http_outcome(HttpOutcomeSample {
            exchange,
            method: "GET",
            path,
            outcome: "success",
            status_code: Some(200),
            latency_ms: 5,
            retry_after_ms: None,
            request_id: None,
            request_context: &[],
            retry: false,
            observed_at_ms: 30,
        });

        let rows = http_outcome_metrics_snapshot();
        let Some(row) = rows
            .iter()
            .find(|row| row.exchange == exchange && row.path == path)
        else {
            panic!("missing binance http outcome metric row");
        };

        assert!(row.endpoint_evidence.is_none());

        let exchange = "binance";
        record_http_outcome(HttpOutcomeSample {
            exchange,
            method: "GET",
            path,
            outcome: "success",
            status_code: Some(200),
            latency_ms: 5,
            retry_after_ms: None,
            request_id: Some("req-evidence"),
            request_context: &["symbol=BTCUSDT".to_owned()],
            retry: false,
            observed_at_ms: 40,
        });

        let rows = http_outcome_metrics_snapshot();
        let Some(row) = rows
            .iter()
            .find(|row| row.exchange == exchange && row.path == path)
        else {
            panic!("missing official binance http outcome metric row");
        };
        let evidence = row.endpoint_evidence.as_ref().expect("endpoint evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.weight, 2);
        assert_eq!(row.last_request_id.as_deref(), Some("req-evidence"));
        assert_eq!(row.last_request_context, vec!["symbol=BTCUSDT"]);
        assert!(evidence.doc_urls[0].starts_with("https://"));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
    }

    #[test]
    fn quality_window_is_venue_scoped_and_bounded() {
        let exchange = "unit_http_quality_window";
        let other = "unit_http_quality_window_other";
        for index in 0..=VENUE_QUALITY_WINDOW_MAX_SAMPLES {
            record_http_outcome(HttpOutcomeSample {
                exchange,
                method: "GET",
                path: "/unit/quality",
                outcome: if index % 2 == 0 { "success" } else { "timeout" },
                status_code: None,
                latency_ms: u64::from(index),
                retry_after_ms: None,
                request_id: None,
                request_context: &[],
                retry: false,
                observed_at_ms: i64::from(index) + 1,
            });
        }
        record_http_outcome(HttpOutcomeSample {
            exchange: other,
            method: "GET",
            path: "/unit/quality",
            outcome: "success",
            status_code: Some(200),
            latency_ms: 7,
            retry_after_ms: None,
            request_id: None,
            request_context: &[],
            retry: false,
            observed_at_ms: 9,
        });

        let rows = http_quality_window_snapshot();
        let row = rows
            .iter()
            .find(|row| row.exchange == exchange)
            .expect("quality window row");
        let other_row = rows
            .iter()
            .find(|row| row.exchange == other)
            .expect("other quality window row");

        assert_eq!(row.request_total, VENUE_QUALITY_WINDOW_MAX_SAMPLES);
        assert_eq!(row.oldest_observed_at_ms, Some(2));
        assert_eq!(
            row.latest_observed_at_ms,
            Some(i64::from(VENUE_QUALITY_WINDOW_MAX_SAMPLES) + 1)
        );
        assert_eq!(other_row.request_total, 1);
        assert_eq!(other_row.latency_ms_total, 7);
    }
}
