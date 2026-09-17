use super::*;

pub const UNRECORDED_EVIDENCE_MARKER: &str = "not_recorded";

fn default_evidence_marker() -> String {
    UNRECORDED_EVIDENCE_MARKER.to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOperationEvidence {
    pub method: String,
    /// Low-cardinality endpoint path template used for metrics and diagnostics.
    pub path: String,
    #[serde(default = "default_evidence_marker")]
    pub checked_at: String,
    #[serde(default = "default_evidence_marker")]
    pub doc_version: String,
    #[serde(default = "default_evidence_marker")]
    pub schema_hash: String,
    #[serde(default = "default_evidence_marker")]
    pub fixture_id: String,
    #[serde(default = "default_evidence_marker")]
    pub parser_test: String,
    #[serde(default = "default_evidence_marker")]
    pub request_builder_test: String,
    #[serde(default = "default_evidence_marker")]
    pub auth_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_context: Vec<String>,
    pub doc_urls: Vec<String>,
    pub use_cases: Vec<String>,
    pub data_kinds: Vec<String>,
    pub rate_scopes: Vec<String>,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOperationHealth {
    pub venue: String,
    pub operation: String,
    pub status: VenueOperationStatus,
    pub source: String,
    pub message: String,
    #[serde(default)]
    pub supported: Option<bool>,
    #[serde(default)]
    pub configured: Option<bool>,
    #[serde(default)]
    pub requested: Option<u64>,
    #[serde(default)]
    pub rows: Option<u64>,
    #[serde(default)]
    pub freshness_ms: Option<i64>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p95_ms: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<VenueOperationEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    pub observed_at_ms: i64,
}

impl VenueOperationHealth {
    /// 该运行态健康项当前是否“可用”。
    ///
    /// 契约：把“已配置/能力支持”（静态字段 `configured`/`supported`）与
    /// “当前可用”（运行态 `status`）彻底分离——只有运行态 `status == Ok`
    /// 且未被显式标记为不支持（`supported == Some(false)`）或未配置
    /// （`configured == Some(false)`）时才算当前可用；`Warn/Blocked/Unknown/Unsupported`
    /// 一律视为不可用。
    pub fn is_currently_usable(&self) -> bool {
        matches!(self.status, VenueOperationStatus::Ok)
            && self.supported != Some(false)
            && self.configured != Some(false)
    }

    /// 该项静态能力是否被声明支持（`supported` 未显式为 false）。
    /// 仅表示能力面声明，不代表当前运行态可用，需配合 [`Self::is_currently_usable`]。
    pub fn capability_supported(&self) -> bool {
        self.supported != Some(false)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOperationHealthSnapshot {
    pub rows: Vec<VenueOperationHealth>,
    pub generated_at_ms: i64,
    pub row_count: usize,
    pub attention_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl VenueOperationHealthSnapshot {
    pub fn new(rows: Vec<VenueOperationHealth>, generated_at_ms: i64) -> Self {
        let row_count = rows.len();
        let attention_count = rows
            .iter()
            .filter(|row| row.status != VenueOperationStatus::Ok)
            .count();
        let retry_after_ms = snapshot_retry_after_ms(&rows);
        Self {
            rows,
            generated_at_ms,
            row_count,
            attention_count,
            retry_after_ms,
        }
    }
}

fn snapshot_retry_after_ms(rows: &[VenueOperationHealth]) -> Option<u64> {
    rows.iter().filter_map(row_retry_after_ms).max()
}

fn row_retry_after_ms(row: &VenueOperationHealth) -> Option<u64> {
    row.retry_after_ms
        .into_iter()
        .chain(
            row.problem
                .as_ref()
                .and_then(|problem| problem.retry_after_ms),
        )
        .max()
}
