use arc_swap::ArcSwap;
use shared_types::{
    AutomatedArbitrageConfig, AutomatedArbitrageConfigPatch, AutomationControlAction,
    AutomationDecision, AutomationDecisionKind, AutomationRuntimeState, AutomationRuntimeStatus,
    ExecutionEnvironment, StrategyKind, AUTOMATION_DECISION_LIMIT,
    MIN_AUTOMATION_ENTRY_COOLDOWN_SECS,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;

const MAX_CANONICAL_SYMBOLS: usize = 16;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum AutomationError {
    #[error("{0} must be finite and between {1} and {2}")]
    InvalidNumber(&'static str, f64, f64),
    #[error("{0} must be between {1} and {2}")]
    InvalidInteger(&'static str, u64, u64),
    #[error("strategy {0:?} is not supported by automated execution")]
    UnsupportedStrategy(StrategyKind),
    #[error("strategy {0:?} is not supported by live automated execution")]
    UnsupportedLiveStrategy(StrategyKind),
    #[error("canonicalSymbols contains an invalid symbol: {0}")]
    InvalidCanonicalSymbol(String),
    #[error("canonicalSymbols supports at most {MAX_CANONICAL_SYMBOLS} values")]
    TooManyCanonicalSymbols,
}

#[derive(Debug)]
pub struct AutomationController {
    status: ArcSwap<AutomationRuntimeStatus>,
}

impl Default for AutomationController {
    fn default() -> Self {
        Self {
            status: ArcSwap::from_pointee(AutomationRuntimeStatus::default()),
        }
    }
}

impl AutomationController {
    pub fn restored(
        config: AutomatedArbitrageConfig,
        now_ms: i64,
    ) -> Result<Self, AutomationError> {
        let mut config = validate_restored_config(config)?;
        if config.enabled && config.environment == ExecutionEnvironment::Live {
            config.paused = true;
        }
        let mut status = AutomationRuntimeStatus {
            config,
            updated_at_ms: now_ms,
            ..AutomationRuntimeStatus::default()
        };
        status.state = idle_state(&status);
        Ok(Self {
            status: ArcSwap::from_pointee(status),
        })
    }

    pub fn snapshot(&self) -> Arc<AutomationRuntimeStatus> {
        self.status.load_full()
    }

    pub fn preview_config(
        &self,
        patch: &AutomatedArbitrageConfigPatch,
    ) -> Result<AutomatedArbitrageConfig, AutomationError> {
        apply_patch(self.snapshot().config.clone(), patch)
    }

    pub fn commit_config(
        &self,
        config: AutomatedArbitrageConfig,
        now_ms: i64,
    ) -> Arc<AutomationRuntimeStatus> {
        let mut next = (*self.snapshot()).clone();
        next.config = config;
        next.state = idle_state(&next);
        next.updated_at_ms = now_ms;
        self.status.store(Arc::new(next));
        self.snapshot()
    }

    pub fn preview_control_config(
        &self,
        action: AutomationControlAction,
    ) -> AutomatedArbitrageConfig {
        let mut config = self.snapshot().config.clone();
        match action {
            AutomationControlAction::Pause => config.paused = true,
            AutomationControlAction::Resume => config.paused = false,
            AutomationControlAction::EmergencyStop => {
                config.enabled = false;
                config.paused = true;
            }
        }
        config
    }

    pub fn update_config(
        &self,
        patch: &AutomatedArbitrageConfigPatch,
        now_ms: i64,
    ) -> Result<Arc<AutomationRuntimeStatus>, AutomationError> {
        let config = self.preview_config(patch)?;
        Ok(self.commit_config(config, now_ms))
    }

    pub fn control(
        &self,
        action: AutomationControlAction,
        now_ms: i64,
    ) -> Arc<AutomationRuntimeStatus> {
        let mut next = (*self.snapshot()).clone();
        match action {
            AutomationControlAction::Pause => {
                next.config.paused = true;
                next.state = AutomationRuntimeState::Paused;
                push_decision(
                    &mut next,
                    control_decision(AutomationDecisionKind::Paused, now_ms),
                );
            }
            AutomationControlAction::Resume => {
                next.config.paused = false;
                next.state = idle_state(&next);
            }
            AutomationControlAction::EmergencyStop => {
                next.config.enabled = false;
                next.config.paused = true;
                next.state = AutomationRuntimeState::Disabled;
                push_decision(
                    &mut next,
                    control_decision(AutomationDecisionKind::EmergencyStopped, now_ms),
                );
            }
        }
        next.updated_at_ms = now_ms;
        self.status.store(Arc::new(next));
        self.snapshot()
    }

    pub fn record_decision(
        &self,
        decision: AutomationDecision,
        state: AutomationRuntimeState,
        active_run_count: usize,
        cooldown_until_ms: Option<i64>,
    ) -> Arc<AutomationRuntimeStatus> {
        let mut next = (*self.snapshot()).clone();
        next.state = state;
        next.active_run_count = active_run_count;
        next.cooldown_until_ms = cooldown_until_ms;
        next.updated_at_ms = decision.occurred_at_ms;
        push_decision(&mut next, decision);
        self.status.store(Arc::new(next));
        self.snapshot()
    }

    /// Synchronizes transient worker state without fabricating a new business decision.
    ///
    /// Execution activity can move an accepted run from submitting to hedged while the last
    /// decision remains the original submission. Returning `None` keeps self-generated activity
    /// pulses from becoming a publish/evaluate loop.
    pub fn sync_runtime_state(
        &self,
        state: AutomationRuntimeState,
        active_run_count: usize,
        now_ms: i64,
    ) -> Option<Arc<AutomationRuntimeStatus>> {
        let current = self.snapshot();
        if current.state == state && current.active_run_count == active_run_count {
            return None;
        }
        let mut next = (*current).clone();
        next.state = state;
        next.active_run_count = active_run_count;
        next.updated_at_ms = now_ms;
        self.status.store(Arc::new(next));
        Some(self.snapshot())
    }
}

fn validate_restored_config(
    config: AutomatedArbitrageConfig,
) -> Result<AutomatedArbitrageConfig, AutomationError> {
    apply_patch(
        AutomatedArbitrageConfig::default(),
        &AutomatedArbitrageConfigPatch {
            enabled: Some(config.enabled),
            paused: Some(config.paused),
            environment: Some(config.environment),
            strategy_kind: Some(config.strategy_kind),
            canonical_symbols: Some(config.canonical_symbols),
            capital_usd: Some(config.capital_usd),
            leverage: Some(config.leverage),
            min_one_cycle_net_bps: Some(config.min_one_cycle_net_bps),
            min_depth_usd: Some(config.min_depth_usd),
            max_concurrent_runs: Some(config.max_concurrent_runs),
            cooldown_secs: Some(config.cooldown_secs),
        },
    )
}

fn apply_patch(
    mut config: AutomatedArbitrageConfig,
    patch: &AutomatedArbitrageConfigPatch,
) -> Result<AutomatedArbitrageConfig, AutomationError> {
    if let Some(value) = patch.enabled {
        config.enabled = value;
    }
    if let Some(value) = patch.paused {
        config.paused = value;
    }
    if let Some(value) = patch.environment {
        config.environment = value;
    }
    if let Some(value) = patch.strategy_kind {
        config.strategy_kind = value;
    }
    if let Some(values) = patch.canonical_symbols.as_ref() {
        config.canonical_symbols = normalize_canonical_symbols(values)?;
    }
    if let Some(value) = patch.capital_usd {
        config.capital_usd = bounded("capitalUsd", value, 1.0, 1_000_000.0)?;
    }
    if let Some(value) = patch.leverage {
        config.leverage = bounded("leverage", value, 1.0, 20.0)?;
    }
    if let Some(value) = patch.min_one_cycle_net_bps {
        config.min_one_cycle_net_bps = bounded("minOneCycleNetBps", value, 0.01, 10_000.0)?;
    }
    if let Some(value) = patch.min_depth_usd {
        config.min_depth_usd = bounded("minDepthUsd", value, 1.0, 1_000_000_000.0)?;
    }
    if let Some(value) = patch.max_concurrent_runs {
        validate_integer("maxConcurrentRuns", value as u64, 1, 8)?;
        config.max_concurrent_runs = value;
    }
    if let Some(value) = patch.cooldown_secs {
        validate_integer(
            "cooldownSecs",
            value,
            MIN_AUTOMATION_ENTRY_COOLDOWN_SECS,
            86_400,
        )?;
        config.cooldown_secs = value;
    }
    validate_strategy(config.strategy_kind, config.environment)?;
    Ok(config)
}

fn validate_strategy(
    strategy_kind: StrategyKind,
    environment: ExecutionEnvironment,
) -> Result<(), AutomationError> {
    if !shared_types::is_p0_executable_strategy(strategy_kind) {
        return Err(AutomationError::UnsupportedStrategy(strategy_kind));
    }
    if environment == ExecutionEnvironment::Live
        && !shared_types::is_live_executable_strategy(strategy_kind)
    {
        return Err(AutomationError::UnsupportedLiveStrategy(strategy_kind));
    }
    Ok(())
}

fn normalize_canonical_symbols(values: &[String]) -> Result<Vec<String>, AutomationError> {
    let symbols = values
        .iter()
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    if symbols.len() > MAX_CANONICAL_SYMBOLS {
        return Err(AutomationError::TooManyCanonicalSymbols);
    }
    if let Some(value) = symbols.iter().find(|value| !valid_canonical_symbol(value)) {
        return Err(AutomationError::InvalidCanonicalSymbol(value.clone()));
    }
    Ok(symbols.into_iter().collect())
}

fn valid_canonical_symbol(value: &str) -> bool {
    value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn bounded(field: &'static str, value: f64, min: f64, max: f64) -> Result<f64, AutomationError> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(AutomationError::InvalidNumber(field, min, max))
    }
}

fn validate_integer(
    field: &'static str,
    value: u64,
    min: u64,
    max: u64,
) -> Result<(), AutomationError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(AutomationError::InvalidInteger(field, min, max))
    }
}

fn idle_state(status: &AutomationRuntimeStatus) -> AutomationRuntimeState {
    if !status.config.enabled {
        AutomationRuntimeState::Disabled
    } else if status.config.paused {
        AutomationRuntimeState::Paused
    } else {
        AutomationRuntimeState::Watching
    }
}

fn control_decision(kind: AutomationDecisionKind, now_ms: i64) -> AutomationDecision {
    AutomationDecision {
        id: format!("automation-control-{now_ms}"),
        kind,
        opportunity_id: None,
        symbol: None,
        reason: match kind {
            AutomationDecisionKind::Paused => "automation paused by operator",
            AutomationDecisionKind::EmergencyStopped => {
                "automation emergency stop disabled new entries"
            }
            _ => "automation control updated",
        }
        .to_owned(),
        execution_run_id: None,
        problem: None,
        execution_artifact: None,
        occurred_at_ms: now_ms,
    }
}

fn push_decision(status: &mut AutomationRuntimeStatus, decision: AutomationDecision) {
    status.last_decision = Some(decision.clone());
    status.recent_decisions.insert(0, decision);
    status.recent_decisions.truncate(AUTOMATION_DECISION_LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_live_mode_is_standing_authorization_without_per_order_confirmation(
    ) -> Result<(), AutomationError> {
        let controller = AutomationController::default();
        let status = controller.update_config(
            &AutomatedArbitrageConfigPatch {
                enabled: Some(true),
                environment: Some(ExecutionEnvironment::Live),
                ..AutomatedArbitrageConfigPatch::default()
            },
            10,
        )?;

        assert_eq!(status.state, AutomationRuntimeState::Watching);
        Ok(())
    }

    #[test]
    fn restored_live_config_keeps_thresholds_and_starts_paused() -> Result<(), AutomationError> {
        let controller = AutomationController::restored(
            AutomatedArbitrageConfig {
                enabled: true,
                paused: false,
                environment: ExecutionEnvironment::Live,
                capital_usd: 10.0,
                min_depth_usd: 10.0,
                cooldown_secs: 30,
                ..AutomatedArbitrageConfig::default()
            },
            42,
        )?;
        let status = controller.snapshot();

        assert!(status.config.enabled);
        assert!(status.config.paused);
        assert_eq!(status.config.environment, ExecutionEnvironment::Live);
        assert_eq!(status.config.capital_usd, 10.0);
        assert_eq!(status.config.min_depth_usd, 10.0);
        assert_eq!(status.state, AutomationRuntimeState::Paused);
        assert_eq!(status.updated_at_ms, 42);
        Ok(())
    }

    #[test]
    fn emergency_stop_disables_entries() {
        let controller = AutomationController::default();

        let status = controller.control(AutomationControlAction::EmergencyStop, 2);

        assert!(!status.config.enabled);
        assert!(status.config.paused);
        assert_eq!(status.state, AutomationRuntimeState::Disabled);
    }

    #[test]
    fn rejects_unsafe_limits() {
        let controller = AutomationController::default();
        let result = controller.update_config(
            &AutomatedArbitrageConfigPatch {
                leverage: Some(100.0),
                max_concurrent_runs: Some(0),
                ..AutomatedArbitrageConfigPatch::default()
            },
            1,
        );

        assert!(result.is_err());
    }

    #[test]
    fn accepts_one_second_successful_entry_cooldown() -> Result<(), AutomationError> {
        let controller = AutomationController::default();
        let status = controller.update_config(
            &AutomatedArbitrageConfigPatch {
                cooldown_secs: Some(MIN_AUTOMATION_ENTRY_COOLDOWN_SECS),
                ..AutomatedArbitrageConfigPatch::default()
            },
            1,
        )?;

        assert_eq!(
            status.config.cooldown_secs,
            MIN_AUTOMATION_ENTRY_COOLDOWN_SECS
        );
        Ok(())
    }

    #[test]
    fn execution_state_sync_preserves_the_last_business_decision() -> Result<(), &'static str> {
        let controller = AutomationController::default();
        controller.control(AutomationControlAction::Pause, 1);

        let status = controller
            .sync_runtime_state(AutomationRuntimeState::Hedged, 1, 2)
            .ok_or("runtime transition missing")?;

        assert_eq!(status.state, AutomationRuntimeState::Hedged);
        assert_eq!(status.active_run_count, 1);
        assert_eq!(status.updated_at_ms, 2);
        assert_eq!(status.recent_decisions.len(), 1);
        assert_eq!(
            status.last_decision.as_ref().map(|decision| decision.kind),
            Some(AutomationDecisionKind::Paused)
        );
        assert!(controller
            .sync_runtime_state(AutomationRuntimeState::Hedged, 1, 3)
            .is_none());
        Ok(())
    }

    #[test]
    fn canonical_symbol_scope_is_normalized_and_validated() -> Result<(), AutomationError> {
        let controller = AutomationController::default();
        let status = controller.update_config(
            &AutomatedArbitrageConfigPatch {
                canonical_symbols: Some(vec![" coti ".into(), "BTW".into(), "btw".into()]),
                ..AutomatedArbitrageConfigPatch::default()
            },
            1,
        )?;
        assert_eq!(status.config.canonical_symbols, ["BTW", "COTI"]);

        let invalid = controller.update_config(
            &AutomatedArbitrageConfigPatch {
                canonical_symbols: Some(vec!["BAD SYMBOL".into()]),
                ..AutomatedArbitrageConfigPatch::default()
            },
            2,
        );
        assert_eq!(
            invalid,
            Err(AutomationError::InvalidCanonicalSymbol("BAD SYMBOL".into()))
        );
        Ok(())
    }

    #[test]
    fn rejects_non_p0_and_non_live_strategy_scopes() {
        let controller = AutomationController::default();
        let diagnostic = controller.update_config(
            &AutomatedArbitrageConfigPatch {
                strategy_kind: Some(StrategyKind::Triangular),
                ..AutomatedArbitrageConfigPatch::default()
            },
            1,
        );
        assert_eq!(
            diagnostic,
            Err(AutomationError::UnsupportedStrategy(
                StrategyKind::Triangular
            ))
        );

        let live_spot = controller.update_config(
            &AutomatedArbitrageConfigPatch {
                environment: Some(ExecutionEnvironment::Live),
                strategy_kind: Some(StrategyKind::SpotCross),
                ..AutomatedArbitrageConfigPatch::default()
            },
            2,
        );
        assert_eq!(
            live_spot,
            Err(AutomationError::UnsupportedLiveStrategy(
                StrategyKind::SpotCross
            ))
        );
    }
}
