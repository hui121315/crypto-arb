use serde::{Deserialize, Serialize};

use crate::{ApiProblem, DeterministicExecutionArtifact, ExecutionEnvironment, StrategyKind};

pub const AUTOMATION_DECISION_LIMIT: usize = 50;
pub const DEFAULT_AUTOMATION_CAPITAL_USD: f64 = 10.0;
pub const DEFAULT_AUTOMATION_MIN_DEPTH_USD: f64 = 10.0;
pub const MIN_AUTOMATION_ENTRY_COOLDOWN_SECS: u64 = 1;

/// A local, read-only projection of one execution and its explicitly linked exits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationExecutionReceipt {
    pub run: crate::ExecutionRun,
    /// Historical order modes, not the currently selected trading environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<crate::ExecutionMode>,
    pub close_runs: Vec<crate::CloseRun>,
    pub close_run_total: usize,
    pub observed_at_ms: i64,
}

impl AutomationExecutionReceipt {
    pub fn matches_pair(run: &crate::ExecutionRun, pair: &crate::PositionPairEvidence) -> bool {
        pair.run_id == run.run_id
            && pair.ticket_id == run.ticket_id
            && pair.opportunity_id == run.opportunity_id
    }

    pub fn matches_close(run: &crate::ExecutionRun, close: &crate::CloseRun) -> bool {
        close
            .legs
            .iter()
            .filter_map(|leg| leg.pair_evidence.as_ref())
            .any(|pair| Self::matches_pair(run, pair))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomatedArbitrageConfig {
    /// In live mode, `enabled && !paused` is the operator's standing authorization for automatic
    /// order submission. Individual opportunities do not require another confirmation step.
    pub enabled: bool,
    pub paused: bool,
    pub environment: ExecutionEnvironment,
    #[serde(default = "default_strategy_kind")]
    pub strategy_kind: StrategyKind,
    #[serde(default)]
    pub canonical_symbols: Vec<String>,
    pub capital_usd: f64,
    pub leverage: f64,
    pub min_one_cycle_net_bps: f64,
    pub min_depth_usd: f64,
    pub max_concurrent_runs: usize,
    pub cooldown_secs: u64,
}

impl Default for AutomatedArbitrageConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            paused: false,
            environment: ExecutionEnvironment::Paper,
            strategy_kind: default_strategy_kind(),
            canonical_symbols: Vec::new(),
            capital_usd: DEFAULT_AUTOMATION_CAPITAL_USD,
            leverage: 1.0,
            min_one_cycle_net_bps: 10.0,
            min_depth_usd: DEFAULT_AUTOMATION_MIN_DEPTH_USD,
            max_concurrent_runs: 1,
            cooldown_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomatedArbitrageConfigPatch {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub environment: Option<ExecutionEnvironment>,
    #[serde(default)]
    pub strategy_kind: Option<StrategyKind>,
    #[serde(default)]
    pub canonical_symbols: Option<Vec<String>>,
    #[serde(default)]
    pub capital_usd: Option<f64>,
    #[serde(default)]
    pub leverage: Option<f64>,
    #[serde(default)]
    pub min_one_cycle_net_bps: Option<f64>,
    #[serde(default)]
    pub min_depth_usd: Option<f64>,
    #[serde(default)]
    pub max_concurrent_runs: Option<usize>,
    #[serde(default)]
    pub cooldown_secs: Option<u64>,
}

const fn default_strategy_kind() -> StrategyKind {
    StrategyKind::PerpCross
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationControlAction {
    Pause,
    Resume,
    EmergencyStop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationControlRequest {
    pub action: AutomationControlAction,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRuntimeState {
    #[default]
    Disabled,
    Paused,
    Watching,
    Previewing,
    Submitting,
    Hedged,
    CoolingDown,
    Blocked,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationDecisionKind {
    CandidateSelected,
    OpportunityQualified,
    NoEligibleCandidate,
    PreviewBlocked,
    Submitted,
    Replayed,
    Paused,
    EmergencyStopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDecision {
    pub id: String,
    pub kind: AutomationDecisionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opportunity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_artifact: Option<DeterministicExecutionArtifact>,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRuntimeStatus {
    pub config: AutomatedArbitrageConfig,
    pub state: AutomationRuntimeState,
    pub active_run_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_decision: Option<AutomationDecision>,
    #[serde(default)]
    pub recent_decisions: Vec<AutomationDecision>,
    pub updated_at_ms: i64,
}

impl Default for AutomationRuntimeStatus {
    fn default() -> Self {
        Self {
            config: AutomatedArbitrageConfig::default(),
            state: AutomationRuntimeState::Disabled,
            active_run_count: 0,
            cooldown_until_ms: None,
            last_decision: None,
            recent_decisions: Vec::new(),
            updated_at_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_paper_disabled_and_single_run() {
        let status = AutomationRuntimeStatus::default();

        assert!(!status.config.enabled);
        assert_eq!(status.config.environment, ExecutionEnvironment::Paper);
        assert_eq!(status.config.strategy_kind, StrategyKind::PerpCross);
        assert!(status.config.canonical_symbols.is_empty());
        assert_eq!(status.config.capital_usd, DEFAULT_AUTOMATION_CAPITAL_USD);
        assert_eq!(
            status.config.min_depth_usd,
            DEFAULT_AUTOMATION_MIN_DEPTH_USD
        );
        assert_eq!(status.config.min_depth_usd, status.config.capital_usd);
        assert_eq!(status.config.max_concurrent_runs, 1);
    }

    #[test]
    fn older_config_payload_defaults_to_perp_cross() -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(AutomatedArbitrageConfig::default())?;
        let mut object = value.as_object().cloned().expect("config object");
        object.remove("strategyKind");
        object.remove("canonicalSymbols");

        let config = serde_json::from_value::<AutomatedArbitrageConfig>(object.into())?;

        assert_eq!(config.strategy_kind, StrategyKind::PerpCross);
        assert!(config.canonical_symbols.is_empty());
        Ok(())
    }
}
