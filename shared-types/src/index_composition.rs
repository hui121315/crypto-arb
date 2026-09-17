use crate::market::MarketDataEnvelope;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexComponent {
    pub symbol: String,
    pub name: String,
    pub weight: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexCompositionQuality {
    Verified,
    Unverified,
    Unsupported,
    Stale,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexCompositionStatus {
    Verified,
    HiddenPrice,
    Mismatch,
    Unverified,
    Unsupported,
    Stale,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexCompositionRiskProfile {
    pub status: IndexCompositionStatus,
    pub overlap_score: f64,
    pub long_quality: IndexCompositionQuality,
    pub short_quality: IndexCompositionQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_evidence: Option<IndexCompositionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_evidence: Option<IndexCompositionEvidence>,
}

/// 单腿指数成分证据：quality 判定来自哪个官方接口/缓存、何时取得。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexCompositionEvidence {
    pub source: String,
    pub received_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexCompositionSnapshot {
    pub venue: String,
    pub symbol: String,
    pub index_id: String,
    pub components: Vec<IndexComponent>,
    pub quality: IndexCompositionQuality,
    pub source: String,
    pub received_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
}

pub type IndexCompositionListEnvelope = MarketDataEnvelope<Vec<IndexCompositionSnapshot>>;
