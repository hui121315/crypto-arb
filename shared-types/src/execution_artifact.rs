//! Snapshot-bound execution artifact contracts.

use crate::{
    ExecutionEnvironment, HedgeLegRole, MarketDataQuality, MarketDataSourceKind, OrderSide,
    StrategyKind,
};
use serde::{Deserialize, Serialize};

pub const EXECUTION_ARTIFACT_SCHEMA_VERSION: &str = "crossline.execution-artifact.v1";
pub const TRANSFER_ROUTE_EVIDENCE_KEY: &str = "transfer_route";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionArtifactStatus {
    Ready,
    Blocked,
    Expired,
    Missing,
    Tampered,
    #[default]
    Unknown,
}

impl ExecutionArtifactStatus {
    #[must_use]
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionArtifactEvidence {
    pub key: String,
    pub label: String,
    pub passed: bool,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionArtifactLeg {
    pub role: HedgeLegRole,
    pub venue: String,
    pub symbol: String,
    pub side: OrderSide,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_price: Option<f64>,
    pub target_notional_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_usd_5bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_quality: Option<MarketDataQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_source: Option<MarketDataSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_observed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeterministicExecutionArtifact {
    pub schema_version: String,
    pub artifact_id: String,
    pub opportunity_id: String,
    pub opportunity_snapshot_id: String,
    pub ticket_id: String,
    pub idempotency_key: String,
    pub environment: ExecutionEnvironment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<StrategyKind>,
    pub symbol: String,
    pub generated_at_ms: i64,
    pub expires_at_ms: i64,
    pub status: ExecutionArtifactStatus,
    pub expected_gross_edge_usd: f64,
    pub expected_total_cost_usd: f64,
    pub expected_net_edge_usd: f64,
    pub capital_usd: f64,
    pub max_loss_usd: f64,
    pub legs: Vec<ExecutionArtifactLeg>,
    pub evidence: Vec<ExecutionArtifactEvidence>,
    pub invalidation_conditions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    pub checksum: String,
    /// A copyable read-only validation command. It never submits an order.
    pub validation_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionArtifactBuildRequest {
    pub idempotency_key: String,
    pub ticket_id: String,
    pub opportunity_snapshot_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionArtifactValidationRequest {
    pub idempotency_key: String,
    pub ticket_id: String,
    pub opportunity_snapshot_id: String,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionArtifactValidationResponse {
    pub valid: bool,
    pub status: ExecutionArtifactStatus,
    pub checked_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<DeterministicExecutionArtifact>,
}
