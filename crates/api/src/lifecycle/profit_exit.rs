use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::services::profit_exit::{self, SubmitOutcome};
use crate::state::AppState;
use futures::{stream, StreamExt};
use portfolio::ProfitExitCandidate;
use shared_types::AutoProfitCloseConfig;
use std::collections::{BTreeMap, BTreeSet};
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_CONFIRMATION_GAP_MS: i64 = 6_000;
const MAX_CONCURRENT_EXITS: usize = 2;

#[derive(Debug, Clone, Copy)]
struct Observation {
    samples: u16,
    last_observed_at_ms: i64,
}

#[derive(Debug, Default)]
struct ProfitExitTracker {
    observations: BTreeMap<String, Observation>,
    cooldown_until_by_run: BTreeMap<String, i64>,
}

impl ProfitExitTracker {
    fn ready_candidates(
        &mut self,
        candidates: &[ProfitExitCandidate],
        config: &AutoProfitCloseConfig,
        now_ms: i64,
    ) -> Vec<ProfitExitCandidate> {
        if !protection_enabled(config) {
            self.reset();
            return Vec::new();
        }
        let present = candidates
            .iter()
            .map(ProfitExitCandidate::confirmation_key)
            .collect::<BTreeSet<_>>();
        self.observations.retain(|key, _| present.contains(key));
        for candidate in candidates {
            self.observe(candidate);
        }
        self.cooldown_until_by_run
            .retain(|_, cooldown_until_ms| now_ms < *cooldown_until_ms);
        candidates
            .iter()
            .filter_map(|candidate| {
                if self
                    .cooldown_until_by_run
                    .get(&candidate.run_id)
                    .is_some_and(|cooldown_until_ms| now_ms < *cooldown_until_ms)
                {
                    return None;
                }
                let key = candidate.confirmation_key();
                self.observations
                    .get(&key)
                    .filter(|observation| {
                        observation.samples >= required_confirmation_samples(candidate, config)
                    })
                    .map(|_| candidate.clone())
            })
            .collect()
    }

    fn observe(&mut self, candidate: &ProfitExitCandidate) {
        let key = candidate.confirmation_key();
        let observation = self.observations.entry(key).or_insert(Observation {
            samples: 0,
            last_observed_at_ms: 0,
        });
        if candidate.observed_at_ms <= observation.last_observed_at_ms {
            return;
        }
        let gap_ms = candidate
            .observed_at_ms
            .saturating_sub(observation.last_observed_at_ms);
        observation.samples =
            if observation.last_observed_at_ms == 0 || gap_ms > MAX_CONFIRMATION_GAP_MS {
                1
            } else {
                observation.samples.saturating_add(1)
            };
        observation.last_observed_at_ms = candidate.observed_at_ms;
    }

    fn record_attempt(
        &mut self,
        candidate: &ProfitExitCandidate,
        config: &AutoProfitCloseConfig,
        now_ms: i64,
    ) {
        self.observations.remove(&candidate.confirmation_key());
        let cooldown_ms = i64::try_from(config.cooldown_secs)
            .unwrap_or(i64::MAX)
            .saturating_mul(1_000);
        self.cooldown_until_by_run
            .insert(candidate.run_id.clone(), now_ms.saturating_add(cooldown_ms));
    }

    fn reset(&mut self) {
        self.observations.clear();
        self.cooldown_until_by_run.clear();
    }
}

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "auto-profit-close",
        RECOVERY_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_worker(state, shutdown).await }
        },
    );
    info!(
        recovery_period_secs = RECOVERY_INTERVAL.as_secs(),
        hot_path = "portfolio_snapshot_update",
        "automatic profit close worker started"
    );
}

async fn run_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tracker = ProfitExitTracker::default();
    let mut updates = state.portfolio_snapshot().subscribe_updates();
    record_evaluation(&state, &registry, &mut tracker).await;

    let mut recovery = tokio::time::interval(RECOVERY_INTERVAL);
    recovery.set_missed_tick_behavior(MissedTickBehavior::Skip);
    recovery.reset();
    loop {
        let snapshot_update = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            changed = updates.changed() => {
                if changed.is_err() {
                    break;
                }
                true
            }
            _ = recovery.tick() => false,
        };
        if snapshot_update {
            recovery.reset();
        }
        record_evaluation(&state, &registry, &mut tracker).await;
    }
}

async fn record_evaluation(
    state: &AppState,
    registry: &crate::task_registry::TaskRegistry,
    tracker: &mut ProfitExitTracker,
) {
    let started_at_ms = common::time::now_ms();
    let result = evaluate_once(state, tracker, started_at_ms).await;
    registry.record_result_timed("auto-profit-close", started_at_ms, result);
}

async fn evaluate_once(
    state: &AppState,
    tracker: &mut ProfitExitTracker,
    now_ms: i64,
) -> Result<(), String> {
    let config = state.trading_service().risk_config().auto_profit_close;
    if !protection_enabled(&config) {
        tracker.reset();
        return Ok(());
    }
    let candidates = profit_exit::candidates(state, &config, now_ms);
    let ready = tracker.ready_candidates(&candidates, &config, now_ms);
    if ready.is_empty() {
        return Ok(());
    }
    for candidate in &ready {
        tracker.record_attempt(candidate, &config, now_ms);
    }
    submit_ready_candidates(state, ready).await
}

async fn submit_ready_candidates(
    state: &AppState,
    candidates: Vec<ProfitExitCandidate>,
) -> Result<(), String> {
    let mut submissions = stream::iter(candidates)
        .map(|candidate| async move {
            let result = profit_exit::submit_candidate(state, &candidate).await;
            (candidate, result)
        })
        .buffer_unordered(MAX_CONCURRENT_EXITS);
    let mut errors = Vec::new();
    while let Some((candidate, result)) = submissions.next().await {
        match result {
            Ok(outcome) => log_submit_outcome(&candidate, outcome),
            Err(error) => errors.push(format!("{}: {error}", candidate.run_id)),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} automatic pair exits failed: {}",
            errors.len(),
            errors.join("; ")
        ))
    }
}

fn log_submit_outcome(candidate: &ProfitExitCandidate, outcome: SubmitOutcome) {
    match outcome {
        SubmitOutcome::Submitted(close_run_id) => log_submitted(candidate, &close_run_id),
        SubmitOutcome::Replayed(status) => log_replayed(candidate, status),
    }
}

fn log_submitted(candidate: &ProfitExitCandidate, close_run_id: &str) {
    let estimated_net_profit_usd = candidate
        .valuation
        .as_ref()
        .map(|valuation| valuation.estimated_net_profit_usd);
    info!(
        execution_run_id = %candidate.run_id,
        close_run_id,
        trigger = candidate.trigger.key(),
        ?estimated_net_profit_usd,
        minimum_liquidation_distance_pct = candidate.minimum_liquidation_distance_pct,
        "automatic pair protection close submitted"
    );
}

fn log_replayed(candidate: &ProfitExitCandidate, status: shared_types::ActionRunStatus) {
    warn!(
        execution_run_id = %candidate.run_id,
        trigger = candidate.trigger.key(),
        ?status,
        "automatic pair protection close replay suppressed"
    );
}

fn protection_enabled(config: &AutoProfitCloseConfig) -> bool {
    config.enabled || config.stop_loss_enabled || config.liquidation_guard_enabled
}

fn required_confirmation_samples(
    candidate: &ProfitExitCandidate,
    config: &AutoProfitCloseConfig,
) -> u16 {
    if candidate.trigger == portfolio::ProfitExitTrigger::LiquidationGuard
        && candidate
            .minimum_liquidation_distance_pct
            .is_some_and(|distance| distance <= 0.0)
    {
        1
    } else {
        config.confirmation_samples
    }
}

#[cfg(test)]
#[path = "profit_exit/tests.rs"]
mod tests;
