use super::*;

pub const VENUE_QUALITY_WINDOW_MAX_SAMPLES: u32 = 600;
pub const VENUE_QUALITY_READY_SAMPLE_MIN: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueQualitySampleWindow {
    pub max_samples_per_metric: u32,
    pub ready_sample_min: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_operation_observed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_operation_observed_at_ms: Option<i64>,
}

impl Default for VenueQualitySampleWindow {
    fn default() -> Self {
        Self {
            max_samples_per_metric: VENUE_QUALITY_WINDOW_MAX_SAMPLES,
            ready_sample_min: VENUE_QUALITY_READY_SAMPLE_MIN,
            oldest_operation_observed_at_ms: None,
            latest_operation_observed_at_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueQuality {
    pub venue: String,
    pub source: String,
    pub sample_status: VenueQualitySampleStatus,
    pub avg_rest_latency_ms: u32,
    pub rest_latency_samples: u32,
    pub ws_jitter_p99_ms: u32,
    pub ws_jitter_samples: u32,
    pub fill_rate_pct: f64,
    pub fill_window_samples: u32,
    pub avg_slippage_bps: f64,
    pub slippage_samples: u32,
    pub uptime_window_pct: f64,
    pub uptime_window_samples: u32,
    #[serde(default)]
    pub sample_window: VenueQualitySampleWindow,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_problem: Option<ApiProblem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueQualitySource {
    RuntimeSamples,
    NeutralNoSample,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueQualityEnvelope {
    pub rows: Vec<VenueQuality>,
    pub generated_at_ms: i64,
    pub source: VenueQualitySource,
    pub row_count: usize,
    pub sampled_count: usize,
    #[serde(default)]
    pub operation_count: usize,
    #[serde(default)]
    pub attention_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl VenueQualityEnvelope {
    pub fn new(rows: Vec<VenueQuality>, generated_at_ms: i64, source: VenueQualitySource) -> Self {
        let row_count = rows.len();
        let sampled_count = rows
            .iter()
            .filter(|row| row.sample_status != VenueQualitySampleStatus::NoSample)
            .count();
        let operation_count = rows.iter().map(|row| row.operation_health.len()).sum();
        let attention_count = rows
            .iter()
            .flat_map(|row| &row.operation_health)
            .filter(|operation| operation.status != VenueOperationStatus::Ok)
            .count();
        let retry_after_ms = rows.iter().filter_map(|row| row.retry_after_ms).max();
        Self {
            rows,
            generated_at_ms,
            source,
            row_count,
            sampled_count,
            operation_count,
            attention_count,
            retry_after_ms,
            request_id: None,
        }
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueQualitySampleStatus {
    NoSample,
    WarmingUp,
    Ready,
}
