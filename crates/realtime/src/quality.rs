use arc_swap::ArcSwap;
use dashmap::DashMap;
use parking_lot::Mutex;
use shared_types::{
    VenueQuality, VenueQualitySampleStatus, VenueQualitySampleWindow,
    VENUE_QUALITY_READY_SAMPLE_MIN, VENUE_QUALITY_WINDOW_MAX_SAMPLES,
};
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

const WINDOW: usize = VENUE_QUALITY_WINDOW_MAX_SAMPLES as usize;
const READY_SAMPLE_MIN: usize = VENUE_QUALITY_READY_SAMPLE_MIN as usize;

#[derive(Debug, Default)]
struct VenueBucket {
    rest_latency_ms: VecDeque<u32>,
    ws_jitter_ms: VecDeque<u32>,
    api_success: VecDeque<bool>,
    fill_success: VecDeque<bool>,
    slippage_bps: VecDeque<f64>,
}

pub struct VenueQualityTracker {
    buckets: DashMap<String, Mutex<VenueBucket>>,
    snapshot: ArcSwap<Vec<VenueQuality>>,
}

impl fmt::Debug for VenueQualityTracker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VenueQualityTracker")
            .field("bucket_count", &self.buckets.len())
            .field("snapshot_count", &self.snapshot.load().len())
            .finish()
    }
}

impl Default for VenueQualityTracker {
    fn default() -> Self {
        Self {
            buckets: DashMap::new(),
            snapshot: ArcSwap::from_pointee(Vec::new()),
        }
    }
}

impl VenueQualityTracker {
    pub fn record_api_probe(&self, venue: &str, success: bool, latency_ms: u32) {
        self.with_bucket(venue, |bucket| {
            push_capped(&mut bucket.rest_latency_ms, latency_ms);
            push_capped(&mut bucket.api_success, success);
        });
    }

    pub fn record_rest_latency(&self, venue: &str, ms: u32) {
        self.with_bucket(venue, |bucket| push_capped(&mut bucket.rest_latency_ms, ms));
    }

    pub fn record_ws_jitter(&self, venue: &str, ms: u32) {
        self.with_bucket(venue, |bucket| push_capped(&mut bucket.ws_jitter_ms, ms));
    }

    pub fn record_fill(&self, venue: &str, success: bool, slippage_bps: f64) {
        self.with_bucket(venue, |bucket| {
            push_capped(&mut bucket.fill_success, success);
            if success {
                push_capped(&mut bucket.slippage_bps, slippage_bps.abs());
            }
        });
    }

    pub fn rebuild_snapshot(&self) {
        let mut out: Vec<VenueQuality> = self
            .buckets
            .iter()
            .map(|entry| {
                let bucket = entry.value().lock();
                VenueQuality {
                    venue: entry.key().clone(),
                    source: "tracker".to_owned(),
                    sample_status: sample_status(&bucket),
                    avg_rest_latency_ms: avg(&bucket.rest_latency_ms),
                    rest_latency_samples: bucket.rest_latency_ms.len() as u32,
                    ws_jitter_p99_ms: p99(&bucket.ws_jitter_ms),
                    ws_jitter_samples: bucket.ws_jitter_ms.len() as u32,
                    fill_rate_pct: success_pct(&bucket.fill_success),
                    fill_window_samples: bucket.fill_success.len() as u32,
                    avg_slippage_bps: avg_slippage(&bucket),
                    slippage_samples: bucket.slippage_bps.len() as u32,
                    uptime_window_pct: api_uptime(&bucket.api_success),
                    uptime_window_samples: bucket.api_success.len() as u32,
                    sample_window: VenueQualitySampleWindow::default(),
                    operation_health: Vec::new(),
                    retry_after_ms: None,
                    last_problem: None,
                }
            })
            .collect();
        out.sort_by(|a, b| a.venue.cmp(&b.venue));
        self.snapshot.store(Arc::new(out));
    }

    pub fn snapshot(&self) -> Arc<Vec<VenueQuality>> {
        self.snapshot.load_full()
    }

    fn with_bucket(&self, venue: &str, f: impl FnOnce(&mut VenueBucket)) {
        let entry = self
            .buckets
            .entry(venue.to_owned())
            .or_insert_with(|| Mutex::new(VenueBucket::default()));
        f(&mut entry.lock());
    }
}

fn push_capped<T>(values: &mut VecDeque<T>, value: T) {
    if values.len() == WINDOW {
        values.pop_front();
    }
    values.push_back(value);
}

fn avg(values: &VecDeque<u32>) -> u32 {
    if values.is_empty() {
        0
    } else {
        (values.iter().copied().map(u64::from).sum::<u64>() / values.len() as u64) as u32
    }
}

fn p99(values: &VecDeque<u32>) -> u32 {
    if values.len() < 100 {
        return 0;
    }
    let mut sorted: Vec<u32> = values.iter().copied().collect();
    sorted.sort_unstable();
    percentile_linear(&sorted, 0.99).round() as u32
}

fn percentile_linear(sorted: &[u32], quantile: f64) -> f64 {
    let pos = (sorted.len() - 1) as f64 * quantile.clamp(0.0, 1.0);
    let lower = pos.floor() as usize;
    let upper = pos.ceil() as usize;
    if lower == upper {
        return sorted[lower] as f64;
    }
    let weight = pos - lower as f64;
    sorted[lower] as f64 * (1.0 - weight) + sorted[upper] as f64 * weight
}

fn pct(n: f64, d: f64) -> f64 {
    if d <= f64::EPSILON {
        0.0
    } else {
        n / d * 100.0
    }
}

fn avg_slippage(bucket: &VenueBucket) -> f64 {
    if bucket.slippage_bps.is_empty() {
        0.0
    } else {
        bucket.slippage_bps.iter().sum::<f64>() / bucket.slippage_bps.len() as f64
    }
}

fn api_uptime(values: &VecDeque<bool>) -> f64 {
    success_pct(values)
}

fn success_pct(values: &VecDeque<bool>) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        let ok = values.iter().filter(|success| **success).count() as f64;
        pct(ok, values.len() as f64)
    }
}

fn sample_status(bucket: &VenueBucket) -> VenueQualitySampleStatus {
    let samples = bucket.rest_latency_ms.len()
        + bucket.ws_jitter_ms.len()
        + bucket.api_success.len()
        + bucket.fill_success.len()
        + bucket.slippage_bps.len();
    if samples == 0 {
        VenueQualitySampleStatus::NoSample
    } else if samples < READY_SAMPLE_MIN {
        VenueQualitySampleStatus::WarmingUp
    } else {
        VenueQualitySampleStatus::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sorted_quality_snapshot() {
        let tracker = VenueQualityTracker::default();
        tracker.record_rest_latency("okx", 20);
        tracker.record_rest_latency("binance", 40);
        tracker.record_ws_jitter("okx", 8);
        tracker.record_api_probe("okx", true, 18);
        tracker.record_api_probe("okx", false, 30_000);
        tracker.record_fill("okx", true, 1.5);
        tracker.record_fill("okx", false, 3.0);
        tracker.rebuild_snapshot();

        let snapshot = tracker.snapshot();

        assert_eq!(snapshot[0].venue, "binance");
        assert_eq!(snapshot[1].venue, "okx");
        assert_eq!(snapshot[1].fill_rate_pct, 50.0);
        assert_eq!(snapshot[1].fill_window_samples, 2);
        assert_eq!(snapshot[1].avg_slippage_bps, 1.5);
        assert_eq!(snapshot[1].slippage_samples, 1);
        assert_eq!(snapshot[1].uptime_window_pct, 50.0);
        assert_eq!(snapshot[1].uptime_window_samples, 2);
        assert_eq!(
            snapshot[1].sample_status,
            VenueQualitySampleStatus::WarmingUp
        );
    }

    #[test]
    fn p99_waits_for_stable_sample_size() {
        let values: VecDeque<u32> = (1..=99).collect();
        assert_eq!(p99(&values), 0);
    }

    #[test]
    fn p99_uses_linear_interpolation_for_large_samples() {
        let values: VecDeque<u32> = (1..=101).collect();
        assert_eq!(p99(&values), 100);
    }

    #[test]
    fn empty_denominators_are_unknown_not_perfect() {
        let values = VecDeque::new();

        assert_eq!(pct(0.0, 0.0), 0.0);
        assert_eq!(api_uptime(&values), 0.0);
    }

    #[test]
    fn sample_status_requires_ready_sample_floor() {
        let mut bucket = VenueBucket::default();
        assert_eq!(sample_status(&bucket), VenueQualitySampleStatus::NoSample);

        push_capped(&mut bucket.rest_latency_ms, 20);
        assert_eq!(sample_status(&bucket), VenueQualitySampleStatus::WarmingUp);

        for ms in 0..READY_SAMPLE_MIN {
            push_capped(&mut bucket.ws_jitter_ms, ms as u32);
        }
        assert_eq!(sample_status(&bucket), VenueQualitySampleStatus::Ready);
    }

    #[test]
    fn fill_and_slippage_windows_are_bounded() {
        let tracker = VenueQualityTracker::default();
        for index in 0..=WINDOW {
            tracker.record_fill("okx", index % 2 == 0, index as f64);
        }
        tracker.rebuild_snapshot();

        let row = tracker
            .snapshot()
            .iter()
            .find(|row| row.venue == "okx")
            .cloned()
            .expect("quality row");

        assert_eq!(row.fill_window_samples, WINDOW as u32);
        assert!(row.slippage_samples <= WINDOW as u32);
    }
}
