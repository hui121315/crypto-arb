use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSnapshot {
    pub summary: PortfolioSummary,
    pub positions: Vec<PositionRow>,
    #[serde(default)]
    pub balances: Vec<VenueBalanceInfo>,
    pub risk: RiskSnapshot,
    pub server_now_ms: i64,
    #[serde(default)]
    pub snapshot_version: String,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default)]
    pub problems: Vec<RuntimeProblem>,
    #[serde(default)]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default)]
    pub account_state: AccountStateSnapshot,
    #[serde(default)]
    pub recent_close_runs: Vec<CloseRun>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioSnapshotStatus {
    #[default]
    Fresh,
    Degraded,
    Stale,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSnapshotEnvelope {
    pub status: PortfolioSnapshotStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<PortfolioSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
    #[serde(default)]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}
