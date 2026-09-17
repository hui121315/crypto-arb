use common::config::AppConfig;
use dashmap::DashMap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use shared_types::{
    OnchainCrossChainAuthorizationEvidence, OnchainCrossChainBridgeExecution,
    OnchainCrossChainBuildResponse, OnchainCrossChainLegProgress, OnchainCrossChainLegRunStatus,
    OnchainCrossChainRun, OnchainCrossChainRunStatus, OnchainCrossChainRunsResponse,
    OnchainCrossChainSwapExecution,
};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub(crate) mod receipts;
pub(crate) mod accounting;
mod valuation_assets;
mod disposition;
mod verification;
pub(crate) mod recovery;
mod recovery_plans;
mod wallet_claims;

const AUTHORIZATION_VALIDITY_MS: i64 = 60_000;
const MAX_STORED_BUILDS: usize = 128;
const MAX_STORED_RUNS: usize = 128;
const RUN_ID_KEY: &[u8] = b"crossline-onchain-cross-chain-run-v1";
const LEG_ACTION_ID_KEY: &[u8] = b"crossline-onchain-cross-chain-leg-v1";
const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    build: Option<OnchainCrossChainBuildResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<OnchainCrossChainRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_plan: Option<recovery_plans::StoredPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CrossChainAuthorizeError {
    Missing,
    Expired,
    NotReady(String),
    IdempotencyConflict,
    Persistence(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CrossChainLegClaimError {
    Missing,
    AuthorizationExpired,
    ActorMismatch,
    InvalidPosition,
    PreviousLegIncomplete,
    InvalidQuote(String),
    Persistence(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CrossChainRecheckError {
    Missing,
    ActorMismatch,
    InvalidState(String),
    Persistence(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrossChainAuthorizeOutcome {
    pub(crate) run: OnchainCrossChainRun,
    pub(crate) replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrossChainLegClaimOutcome {
    pub(crate) run: OnchainCrossChainRun,
    pub(crate) replayed: bool,
}

#[derive(Debug)]
pub(crate) struct OnchainCrossChainRunStore {
    builds: DashMap<String, OnchainCrossChainBuildResponse>,
    runs: DashMap<String, OnchainCrossChainRun>,
    run_by_idempotency: DashMap<String, String>,
    recovery_plans: DashMap<String, recovery_plans::StoredPlan>,
    recovery_keys: DashMap<String, String>,
    path: Option<PathBuf>,
    ledger_lock: Mutex<()>,
    persistence_problem: Mutex<Option<String>>,
    wallet_claims: std::sync::Arc<super::onchain_wallet_claims::WalletClaims>,
}

impl OnchainCrossChainRunStore {
    pub(crate) fn load(config: &AppConfig) -> Self {
        let path = config
            .storage
            .onchain_cross_chain_ledger_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| config.storage.resolve_runtime_path(path));
        Self::load_path(path, common::time::now_ms())
    }

    fn load_path(path: Option<PathBuf>, now_ms: i64) -> Self {
        let replay = replay(path.as_deref(), now_ms);
        let persistence_problem = replay.problem.or_else(|| storage_problem(path.as_deref()));
        let run_by_idempotency = replay
            .runs
            .iter()
            .map(|run| (run.idempotency_key.clone(), run.run_id.clone()))
            .collect();
        let store = Self {
            builds: replay
                .builds
                .into_iter()
                .map(|build| (build.build_id.clone(), build))
                .collect(),
            runs: replay
                .runs
                .into_iter()
                .map(|run| (run.run_id.clone(), run))
                .collect(),
            run_by_idempotency,
            recovery_keys: replay.recovery_plans.iter().filter_map(|stored| stored.plan.idempotency_key.as_ref()
                .map(|key| (key.clone(), stored.plan.plan_id.clone()))).collect(),
            recovery_plans: replay.recovery_plans.into_iter().map(|stored| (stored.plan.plan_id.clone(), stored)).collect(),
            path,
            ledger_lock: Mutex::new(()),
            persistence_problem: Mutex::new(persistence_problem),
            wallet_claims: Default::default(),
        };
        store.prune_runs(now_ms);
        store.prune_recovery_plans(now_ms);
        store.restore_wallet_claims();
        store
    }

    pub(crate) fn insert_build(
        &self,
        build: OnchainCrossChainBuildResponse,
        now_ms: i64,
    ) -> Result<(), String> {
        self.prune_builds(now_ms);
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        if self.builds.get(&build.build_id).is_some_and(|old| *old != build) {
            return Err("同一跨链构建标识的合同不可修改，请重新构建".to_owned());
        }
        if !self.builds.contains_key(&build.build_id) {
            self.append_unlocked(&LogEntry {
                schema_version: SCHEMA_VERSION,
                build: Some(build.clone()),
                run: None,
                recovery_plan: None,
            })?;
        }
        self.builds.insert(build.build_id.clone(), build);
        if self.builds.len() > MAX_STORED_BUILDS {
            self.remove_oldest_build();
        }
        Ok(())
    }

    pub(crate) fn authorize(
        &self,
        build_id: &str,
        idempotency_key: &str,
        actor: &str,
        now_ms: i64,
    ) -> Result<CrossChainAuthorizeOutcome, CrossChainAuthorizeError> {
        let _guard = self.ledger_lock.lock();
        self.readiness().map_err(CrossChainAuthorizeError::Persistence)?;
        if let Some(run_id) = self.run_by_idempotency.get(idempotency_key) {
            let run = self
                .runs
                .get(run_id.value())
                .map(|entry| entry.value().clone())
                .ok_or(CrossChainAuthorizeError::IdempotencyConflict)?;
            if run.build.build_id != build_id || run.authorization.actor != actor {
                return Err(CrossChainAuthorizeError::IdempotencyConflict);
            }
            return Ok(CrossChainAuthorizeOutcome {
                run: project_expiry(run, now_ms),
                replayed: true,
            });
        }
        let build = self
            .builds
            .get(build_id)
            .map(|entry| entry.value().clone())
            .ok_or(CrossChainAuthorizeError::Missing)?;
        if build.valid_until_ms < now_ms {
            self.builds.remove(build_id);
            return Err(CrossChainAuthorizeError::Expired);
        }
        if build.monitor_only || !build.submit_ready || !build.blockers.is_empty() {
            return Err(CrossChainAuthorizeError::NotReady(
                build
                    .blockers
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "跨链闭环仍是监控预览，尚未开放真实资金提交".to_owned()),
            ));
        }
        let run_id = run_id(idempotency_key);
        let legs = build
            .legs
            .iter()
            .map(|leg| OnchainCrossChainLegProgress {
                position: leg.position,
                kind: leg.kind,
                client_action_id: leg_action_id(&run_id, leg.position),
                status: OnchainCrossChainLegRunStatus::RequoteRequired,
                attempts: 0,
                planned_input_amount_raw: leg.input_amount_raw.clone(),
                minimum_output_amount_raw: leg.minimum_output_amount_raw.clone(),
                submitted_input_amount_raw: None,
                actual_input_amount_raw: None,
                actual_output_amount_raw: None,
                source_receipt: None,
                destination_receipt: None,
                receipt_checks: 0,
                recovery_started_at_ms: None,
                recovery_checks: 0,
                bridge_recovery: None,
                bridge_reported_output_amount_raw: None,
                provider_transaction_id: None,
                swap_execution: None,
                bridge_execution: None,
                source_transaction_id: None,
                source_submitted_at_ms: None,
                destination_transaction_id: None,
                quote_observed_at_ms: None,
                quote_valid_until_ms: None,
                projected_final_quote_amount_raw: None,
                required_final_quote_amount_raw: None,
                projected_net_return_bps: None,
                last_checked_at_ms: None,
                evidence_source: None,
                problem: None,
            })
            .collect();
        let mut run = OnchainCrossChainRun {
            run_id: run_id.clone(),
            build,
            idempotency_key: idempotency_key.to_owned(),
            status: OnchainCrossChainRunStatus::AuthorizedAwaitingSubmit,
            authorization: OnchainCrossChainAuthorizationEvidence {
                actor: actor.to_owned(),
                authorized_at_ms: now_ms,
                valid_until_ms: now_ms.saturating_add(AUTHORIZATION_VALIDITY_MS),
                confirmation_version: "onchain-cross-chain-v1".to_owned(),
            },
            active_position: None,
            legs,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            next_action: "在授权过期前按实际余额重新构建第 1 腿".to_owned(),
            problem: None,
            accounting: None,
        };
        self.persist_run_unlocked(&mut run)
            .map_err(CrossChainAuthorizeError::Persistence)?;
        self.runs.insert(run_id.clone(), run.clone());
        self.run_by_idempotency
            .insert(idempotency_key.to_owned(), run_id);
        self.prune_runs(now_ms);
        Ok(CrossChainAuthorizeOutcome {
            run,
            replayed: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn claim_leg(
        &self,
        run_id: &str,
        actor: &str,
        position: u8,
        actual_input_amount_raw: String,
        minimum_output_amount_raw: String,
        provider_transaction_id: String,
        swap_execution: Option<OnchainCrossChainSwapExecution>,
        bridge_execution: Option<OnchainCrossChainBridgeExecution>,
        quote_observed_at_ms: i64,
        quote_valid_until_ms: i64,
        projected_final_quote_amount_raw: String,
        required_final_quote_amount_raw: String,
        projected_net_return_bps: String,
        now_ms: i64,
    ) -> Result<CrossChainLegClaimOutcome, CrossChainLegClaimError> {
        let _guard = self.ledger_lock.lock();
        self.readiness().map_err(CrossChainLegClaimError::Persistence)?;
        let mut run = self
            .runs
            .get(run_id)
            .map(|entry| entry.value().clone())
            .ok_or(CrossChainLegClaimError::Missing)?;
        if run.authorization.actor != actor {
            return Err(CrossChainLegClaimError::ActorMismatch);
        }
        if run.status == OnchainCrossChainRunStatus::AuthorizedAwaitingSubmit
            && run.authorization.valid_until_ms < now_ms
        {
            return Err(CrossChainLegClaimError::AuthorizationExpired);
        }
        let expected_position = run
            .legs
            .iter()
            .find(|leg| leg.status != OnchainCrossChainLegRunStatus::Completed)
            .map(|leg| leg.position)
            .ok_or(CrossChainLegClaimError::InvalidPosition)?;
        if position != expected_position {
            return Err(CrossChainLegClaimError::PreviousLegIncomplete);
        }
        let leg_index = run
            .legs
            .iter()
            .position(|leg| leg.position == position)
            .ok_or(CrossChainLegClaimError::InvalidPosition)?;
        let current = &run.legs[leg_index];
        if current.status != OnchainCrossChainLegRunStatus::RequoteRequired {
            return Ok(CrossChainLegClaimOutcome {
                run: project_expiry(run, now_ms),
                replayed: true,
            });
        }
        if !matches!(
            run.status,
            OnchainCrossChainRunStatus::AuthorizedAwaitingSubmit
                | OnchainCrossChainRunStatus::Running
        ) || run.active_position.is_some()
        {
            return Err(CrossChainLegClaimError::InvalidQuote(
                "跨链任务已暂停或已有执行步骤，不能通过旧的重报价请求恢复提交".into(),
            ));
        }
        positive_raw(&actual_input_amount_raw, "actual input")
            .map_err(CrossChainLegClaimError::InvalidQuote)?;
        positive_raw(&minimum_output_amount_raw, "minimum output")
            .map_err(CrossChainLegClaimError::InvalidQuote)?;
        positive_raw(&projected_final_quote_amount_raw, "projected final output")
            .map_err(CrossChainLegClaimError::InvalidQuote)?;
        positive_raw(&required_final_quote_amount_raw, "required final output")
            .map_err(CrossChainLegClaimError::InvalidQuote)?;
        if !projected_net_return_bps
            .parse::<f64>()
            .is_ok_and(f64::is_finite)
        {
            return Err(CrossChainLegClaimError::InvalidQuote(
                "projected net return bps is invalid".to_owned(),
            ));
        }
        if provider_transaction_id.trim().is_empty()
            || quote_observed_at_ms > now_ms
            || quote_valid_until_ms <= now_ms
        {
            return Err(CrossChainLegClaimError::InvalidQuote(
                "逐腿报价缺少有效 provider 身份或已过期".to_owned(),
            ));
        }
        validate_leg_execution(
            current.kind,
            position,
            &actual_input_amount_raw,
            &minimum_output_amount_raw,
            &provider_transaction_id,
            swap_execution.as_ref(),
            bridge_execution.as_ref(),
            now_ms,
        )
        .map_err(CrossChainLegClaimError::InvalidQuote)?;
        let source = swap_execution.as_ref().map(|execution| (execution.chain.clone(), execution.wallet_address.clone()))
            .or_else(|| bridge_execution.as_ref().and_then(|execution| run.build.legs.iter()
                .find(|leg| leg.position == position).map(|leg| (leg.from_chain.clone(), execution.from_address.clone()))));
        if let Some((chain, wallet)) = source {
            self.ensure_recovery_wallet_available(&chain, &wallet, None, now_ms)
                .map_err(CrossChainLegClaimError::InvalidQuote)?;
        }
        if let Some(previous) = leg_index
            .checked_sub(1)
            .and_then(|index| run.legs.get(index))
        {
            if previous.status != OnchainCrossChainLegRunStatus::Completed {
                return Err(CrossChainLegClaimError::PreviousLegIncomplete);
            }
            if previous.actual_output_amount_raw.as_deref()
                != Some(actual_input_amount_raw.as_str())
            {
                return Err(CrossChainLegClaimError::InvalidQuote(
                    "本腿输入必须等于上一腿已证实的真实到账数量".to_owned(),
                ));
            }
        }
        let leg = &mut run.legs[leg_index];
        leg.status = OnchainCrossChainLegRunStatus::SubmissionClaimed;
        leg.attempts = leg.attempts.saturating_add(1);
        leg.submitted_input_amount_raw = Some(actual_input_amount_raw);
        leg.minimum_output_amount_raw = Some(minimum_output_amount_raw);
        leg.provider_transaction_id = Some(provider_transaction_id);
        leg.swap_execution = swap_execution;
        leg.bridge_execution = bridge_execution;
        leg.quote_observed_at_ms = Some(quote_observed_at_ms);
        leg.quote_valid_until_ms = Some(quote_valid_until_ms);
        leg.projected_final_quote_amount_raw = Some(projected_final_quote_amount_raw);
        leg.required_final_quote_amount_raw = Some(required_final_quote_amount_raw);
        leg.projected_net_return_bps = Some(projected_net_return_bps);
        leg.last_checked_at_ms = Some(now_ms);
        leg.problem = None;
        run.active_position = Some(position);
        run.status = OnchainCrossChainRunStatus::Running;
        run.updated_at_ms = now_ms;
        run.next_action = format!(
            "第 {position} 腿提交占位已落盘；只允许记录这一笔交易哈希，不得重新构建后直接重复广播"
        );
        run.problem = None;
        self.persist_run_unlocked(&mut run)
            .map_err(CrossChainLegClaimError::Persistence)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(CrossChainLegClaimOutcome {
            run,
            replayed: false,
        })
    }

    pub(crate) fn record_submission_intent(
        &self,
        run_id: &str,
        source_transaction_id: String,
        evidence_source: String,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                let leg = active_leg_mut(run)?;
                if leg.status != OnchainCrossChainLegRunStatus::SubmissionClaimed {
                    return Err("cross-chain leg is not awaiting a submission intent".to_owned());
                }
                if source_transaction_id.trim().is_empty() {
                    return Err("cross-chain source transaction id is empty".to_owned());
                }
                leg.source_transaction_id = Some(source_transaction_id);
                leg.source_submitted_at_ms = Some(now_ms);
                leg.evidence_source = Some(evidence_source);
                leg.status = OnchainCrossChainLegRunStatus::Submitted;
                leg.last_checked_at_ms = Some(now_ms);
                run.status = OnchainCrossChainRunStatus::AwaitingSourceFinality;
                run.next_action = "交易哈希已持久化；恢复流程只查询终态，不重复广播".to_owned();
                run.problem = None;
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn record_source_confirmed(
        &self,
        run_id: &str,
        evidence_source: String,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                let leg = active_leg_mut(run)?;
                if leg.status != OnchainCrossChainLegRunStatus::Submitted {
                    return Err(
                        "cross-chain leg has no submitted transaction to confirm".to_owned()
                    );
                }
                leg.status = OnchainCrossChainLegRunStatus::SourceConfirmed;
                leg.last_checked_at_ms = Some(now_ms);
                leg.evidence_source = Some(evidence_source);
                run.status = OnchainCrossChainRunStatus::AwaitingDestinationEvidence;
                run.next_action =
                    "等待目标代币真实到账数量；provider 成功状态本身不算完成".to_owned();
                run.problem = None;
                Ok(())
            },
            now_ms,
        )
    }

    #[cfg(test)]
    fn record_destination_evidence(
        &self,
        run_id: &str,
        actual_output_amount_raw: String,
        destination_transaction_id: Option<String>,
        evidence_source: String,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        let actual = positive_raw(&actual_output_amount_raw, "actual output")?;
        if destination_transaction_id
            .as_deref()
            .is_none_or(|hash| hash.trim().is_empty())
        {
            return Err("cross-chain destination evidence requires a transaction id".into());
        }
        self.update_run(
            run_id,
            |run| {
                let active_position = run
                    .active_position
                    .ok_or_else(|| "cross-chain active leg is missing".to_owned())?;
                let leg = active_leg_mut(run)?;
                if leg.status != OnchainCrossChainLegRunStatus::SourceConfirmed {
                    return Err("cross-chain source finality is not proven".to_owned());
                }
                let minimum = leg
                    .minimum_output_amount_raw
                    .as_deref()
                    .map(|value| positive_raw(value, "minimum output"))
                    .transpose()?;
                leg.status = OnchainCrossChainLegRunStatus::Completed;
                leg.actual_output_amount_raw = Some(actual_output_amount_raw);
                leg.destination_transaction_id = destination_transaction_id;
                leg.last_checked_at_ms = Some(now_ms);
                leg.evidence_source = Some(evidence_source);
                leg.problem = None;
                run.active_position = None;
                if run
                    .legs
                    .iter()
                    .all(|leg| leg.status == OnchainCrossChainLegRunStatus::Completed)
                {
                    run.status = OnchainCrossChainRunStatus::Completed;
                    run.next_action = "四腿均已有真实到账证据；闭环完成".to_owned();
                } else {
                    run.status = OnchainCrossChainRunStatus::Running;
                    run.next_action = format!(
                        "第 {active_position} 腿已完成；按真实到账数量即时重建第 {} 腿",
                        active_position.saturating_add(1)
                    );
                }
                run.problem = None;
                if minimum.is_some_and(|minimum| actual < minimum) {
                    run.status = OnchainCrossChainRunStatus::Paused;
                    run.problem = Some("真实到账低于已授权最低数量，已暂停后续资金操作".into());
                    run.next_action = "保留已到账资产；核对扣费与剩余收益后决定恢复或补偿".into();
                }
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn pause(
        &self,
        run_id: &str,
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                if let Ok(leg) = active_leg_mut(run) {
                    leg.status = OnchainCrossChainLegRunStatus::Paused;
                    leg.last_checked_at_ms = Some(now_ms);
                    leg.problem = Some(problem.clone());
                }
                run.status = OnchainCrossChainRunStatus::Paused;
                run.next_action = "核对源链交易、目标到账和库存后再选择恢复或补偿".to_owned();
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn request_recheck(
        &self,
        run_id: &str,
        actor: &str,
        expected_position: u8,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, CrossChainRecheckError> {
        let _guard = self.ledger_lock.lock();
        let mut run = self
            .runs
            .get(run_id)
            .map(|row| row.value().clone())
            .ok_or(CrossChainRecheckError::Missing)?;
        if run.authorization.actor != actor {
            return Err(CrossChainRecheckError::ActorMismatch);
        }
        let index = run
            .legs
            .iter()
            .position(|leg| leg.position == expected_position)
            .ok_or_else(|| CrossChainRecheckError::InvalidState("指定步骤不存在".into()))?;
        let leg = &run.legs[index];
        // A retried read request must never rewind a completed step or follow the next one.
        if leg.status == OnchainCrossChainLegRunStatus::Completed {
            return Ok(project_expiry(run, now_ms));
        }
        if run.active_position != Some(expected_position)
            || leg
                .source_transaction_id
                .as_deref()
                .is_none_or(|hash| hash.trim().is_empty())
            || leg.swap_execution.is_none() && leg.bridge_execution.is_none()
        {
            return Err(CrossChainRecheckError::InvalidState(
                "缺少当前步骤的持久化交易哈希或执行合同；不能通过重新核验重复提交".into(),
            ));
        }
        if matches!(
            run.status,
            OnchainCrossChainRunStatus::AwaitingSourceFinality
                | OnchainCrossChainRunStatus::AwaitingDestinationEvidence
        ) && matches!(
            leg.status,
            OnchainCrossChainLegRunStatus::Submitted
                | OnchainCrossChainLegRunStatus::SourceConfirmed
        ) {
            return Ok(run);
        }
        if run.status != OnchainCrossChainRunStatus::Paused
            || leg.status != OnchainCrossChainLegRunStatus::Paused
        {
            return Err(CrossChainRecheckError::InvalidState(
                "当前步骤不能恢复到账核验；尚未广播、失败补偿或已到账的步骤需分别处理".into(),
            ));
        }
        let source_confirmed = leg.bridge_execution.is_some()
            && leg.actual_input_amount_raw.is_some()
            && leg.source_receipt.as_ref().is_some_and(|receipt|
                receipt.status == shared_types::OnchainChainSettlementStatus::Complete);
        let leg = &mut run.legs[index];
        leg.status = if source_confirmed { OnchainCrossChainLegRunStatus::SourceConfirmed }
            else { OnchainCrossChainLegRunStatus::Submitted };
        leg.last_checked_at_ms = None;
        leg.receipt_checks = 0;
        leg.recovery_started_at_ms = Some(now_ms);
        leg.recovery_checks = 0;
        leg.problem = None;
        run.status = if source_confirmed { OnchainCrossChainRunStatus::AwaitingDestinationEvidence }
            else { OnchainCrossChainRunStatus::AwaitingSourceFinality };
        run.problem = None;
        run.updated_at_ms = now_ms;
        run.next_action = format!(
            "第 {expected_position} 步已恢复只读核验，最多 12 轮；只查询原交易，不重新广播或提交下一步"
        );
        self.persist_run_unlocked(&mut run)
            .map_err(CrossChainRecheckError::Persistence)?;
        self.runs.insert(run_id.to_owned(), run.clone());
        Ok(run)
    }

    pub(crate) fn runs(&self, limit: usize, now_ms: i64) -> OnchainCrossChainRunsResponse {
        let mut rows = self
            .runs
            .iter()
            .map(|entry| project_expiry(entry.value().clone(), now_ms))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            settled(left, now_ms)
                .cmp(&settled(right, now_ms))
                .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
        });
        let pending_count = rows.iter().filter(|run| !settled(run, now_ms)).count();
        rows.truncate(limit.clamp(1, MAX_STORED_RUNS).max(pending_count));
        OnchainCrossChainRunsResponse {
            recovery_plans: self.recovery_plan_rows(now_ms),
            rows,
            observed_at_ms: now_ms,
            recovery_problem: self.readiness().err(),
        }
    }

    pub(crate) fn run(&self, run_id: &str, now_ms: i64) -> Option<OnchainCrossChainRun> {
        self.runs
            .get(run_id)
            .map(|entry| project_expiry(entry.value().clone(), now_ms))
    }

    pub(crate) fn record_check_problem(
        &self,
        run_id: &str,
        problem: String,
        evidence_source: String,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                let leg = active_leg_mut(run)?;
                leg.last_checked_at_ms = Some(now_ms);
                leg.evidence_source = Some(evidence_source);
                leg.problem = Some(problem.clone());
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn readiness(&self) -> Result<(), String> {
        if self.path.is_none() {
            return Err("跨链闭环恢复日志未配置，已阻止真实资金提交".to_owned());
        }
        self.persistence_problem
            .lock()
            .as_ref()
            .map_or(Ok(()), |problem| {
                Err(format!("跨链恢复日志异常，已停止新增资金动作；请保留日志并核对后重启：{problem}"))
            })
    }

    fn update_run(
        &self,
        run_id: &str,
        update: impl FnOnce(&mut OnchainCrossChainRun) -> Result<(), String>,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let mut run = self
            .runs
            .get(run_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| "cross-chain run is missing".to_owned())?;
        let before = run.clone();
        update(&mut run)?;
        accounting::project(&mut run);
        if run == before { return Ok(run); }
        run.updated_at_ms = now_ms;
        self.persist_run_unlocked(&mut run)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(run)
    }

    fn append_unlocked(&self, entry: &LogEntry) -> Result<(), String> {
        self.readiness()?;
        if let Some((owner, hold, now)) = wallet_claims::change(entry)? {
            return self.wallet_claims.commit(owner, hold, now, || self.write_entry(entry));
        }
        self.wallet_claims.persist_unclaimed(
            super::onchain_wallet_claims::Module::CrossChain,
            || self.write_entry(entry),
        )
    }

    fn write_entry(&self, entry: &LogEntry) -> Result<(), String> {
        let path = self.path.as_deref().ok_or("跨链恢复日志未配置")?;
        match append_jsonl(path, entry) {
            Ok(()) => Ok(()),
            Err(error) => {
                let problem = error.to_string();
                *self.persistence_problem.lock() = Some(problem.clone());
                tracing::warn!(path = %path.display(), %error, "failed to persist cross-chain run state");
                Err(problem)
            }
        }
    }

    fn persist_run_unlocked(&self, run: &mut OnchainCrossChainRun) -> Result<(), String> {
        accounting::project(run);
        self.append_unlocked(&LogEntry {
            schema_version: SCHEMA_VERSION,
            build: None,
            run: Some(run.clone()),
            recovery_plan: None,
        })
    }

    fn prune_builds(&self, now_ms: i64) {
        self.builds
            .retain(|_, build| build.valid_until_ms >= now_ms);
    }

    fn prune_runs(&self, now_ms: i64) {
        while self.runs.len() > MAX_STORED_RUNS {
            let oldest = self
                .runs
                .iter()
                .filter(|entry| settled(entry.value(), now_ms))
                .min_by_key(|entry| entry.updated_at_ms)
                .map(|entry| entry.key().clone());
            let Some(run_id) = oldest else {
                break;
            };
            self.runs.remove(&run_id);
            // Keep the key as a tombstone. Evicting history must never allow replay.
        }
    }

    fn remove_oldest_build(&self) {
        let oldest = self
            .builds
            .iter()
            .min_by_key(|entry| entry.built_at_ms)
            .map(|entry| entry.key().clone());
        if let Some(build_id) = oldest {
            self.builds.remove(&build_id);
        }
    }
}

fn settled(run: &OnchainCrossChainRun, now_ms: i64) -> bool {
    matches!(
        run.status,
        OnchainCrossChainRunStatus::Completed | OnchainCrossChainRunStatus::AuthorizationExpired
    ) || run.status == OnchainCrossChainRunStatus::AuthorizedAwaitingSubmit
        && run.authorization.valid_until_ms < now_ms
}

#[derive(Default)]
struct ReplayState {
    builds: Vec<OnchainCrossChainBuildResponse>,
    runs: Vec<OnchainCrossChainRun>,
    problem: Option<String>,
    recovery_plans: Vec<recovery_plans::StoredPlan>,
}

fn replay(path: Option<&Path>, now_ms: i64) -> ReplayState {
    let Some(path) = path else {
        return ReplayState::default();
    };
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ReplayState::default(),
        Err(error) => return ReplayState {
            problem: Some(format!("无法读取跨链恢复日志：{error}")),
            ..Default::default()
        },
    };
    let mut builds = std::collections::BTreeMap::new();
    let mut runs: std::collections::BTreeMap<String, OnchainCrossChainRun> = std::collections::BTreeMap::new();
    let mut keys = std::collections::BTreeMap::new();
    let mut recovery_plans = std::collections::BTreeMap::<String, recovery_plans::StoredPlan>::new();
    let mut recovery_keys = std::collections::BTreeMap::<String, String>::new();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_number = 0;
    let problem = loop {
        line.clear();
        line_number += 1;
        match reader.read_line(&mut line) {
            Ok(0) => break None,
            Err(error) => break Some(format!("恢复日志第 {line_number} 行读取失败：{error}")),
            Ok(_) => {}
        }
        if !line.ends_with('\n') {
            break Some(format!("恢复日志第 {line_number} 行未完整写入，不能跳过或继续追加"));
        }
        let entry = match serde_json::from_str::<LogEntry>(line.trim()) {
            Ok(entry) if entry.schema_version == SCHEMA_VERSION
                && [entry.build.is_some(), entry.run.is_some(), entry.recovery_plan.is_some()].into_iter().filter(|v| *v).count() == 1 => entry,
            _ => break Some(format!("恢复日志第 {line_number} 行损坏或版本不支持")),
        };
        if let Some(build) = entry.build {
            if build.build_id.trim().is_empty() || builds.get(&build.build_id).is_some_and(|old| old != &build) {
                break Some(format!("恢复日志第 {line_number} 行构建合同冲突"));
            }
            builds.insert(build.build_id.clone(), build);
        }
        if let Some(run) = entry.run {
            if let Err(problem) = recovery_plans::validate_run_reservations(&run, &recovery_plans) {
                break Some(format!("恢复日志第 {line_number} 行预留冲突：{problem}"));
            }
            if run.run_id.trim().is_empty() || run.idempotency_key.trim().is_empty()
                || run.authorization.actor.trim().is_empty() || run.build.build_id.trim().is_empty()
                || keys.get(&run.idempotency_key).is_some_and(|id| id != &run.run_id)
                || builds.get(&run.build.build_id).is_some_and(|build| build != &run.build)
                || runs.get(&run.run_id).is_some_and(|old| old.idempotency_key != run.idempotency_key
                    || old.authorization != run.authorization || old.build != run.build
                    || old.created_at_ms != run.created_at_ms || !recovery::preserves_confirmed(old, &run))
            {
                break Some(format!("恢复日志第 {line_number} 行运行身份、合同或费用归属冲突"));
            }
            keys.insert(run.idempotency_key.clone(), run.run_id.clone());
            runs.insert(run.run_id.clone(), run);
        }
        if let Some(stored) = entry.recovery_plan {
            let checked = recovery_plans::validate_replay(&stored, &recovery_plans, &runs, &recovery_keys);
            if let Err(problem) = checked {
                break Some(format!("恢复日志第 {line_number} 行处置计划冲突：{problem}"));
            }
            if let Some(key) = &stored.plan.idempotency_key {
                recovery_keys.insert(key.clone(), stored.plan.plan_id.clone());
            }
            recovery_plans.insert(stored.plan.plan_id.clone(), stored);
        }
    };
    ReplayState {
        builds: builds.into_values().filter(|build| build.valid_until_ms >= now_ms).collect(),
        runs: runs.into_values().map(recover_ambiguous_claim).collect(),
        problem,
        recovery_plans: recovery_plans.into_values().collect(),
    }
}

fn recover_ambiguous_claim(mut run: OnchainCrossChainRun) -> OnchainCrossChainRun {
    for leg in &mut run.legs {
        // Older journals stored the submitted quantity as an actual debit.
        if leg.submitted_input_amount_raw.is_none() && leg.source_receipt.is_none() {
            leg.submitted_input_amount_raw = leg.actual_input_amount_raw.take();
        }
    }
    let ambiguous = run.legs.iter_mut().find(|leg| {
        leg.status == OnchainCrossChainLegRunStatus::SubmissionClaimed
            && leg.source_transaction_id.is_none()
    });
    if let Some(leg) = ambiguous {
        let problem = format!(
            "第 {} 腿已持久化提交占位但没有交易哈希；重启后禁止自动重复提交",
            leg.position
        );
        leg.status = OnchainCrossChainLegRunStatus::Paused;
        leg.problem = Some(problem.clone());
        run.status = OnchainCrossChainRunStatus::Paused;
        run.next_action = "人工核对钱包历史与 provider 状态后恢复或补偿".to_owned();
        run.problem = Some(problem);
    }
    accounting::project(&mut run);
    run
}

fn project_expiry(mut run: OnchainCrossChainRun, now_ms: i64) -> OnchainCrossChainRun {
    if run.status == OnchainCrossChainRunStatus::AuthorizedAwaitingSubmit
        && run.authorization.valid_until_ms < now_ms
    {
        run.status = OnchainCrossChainRunStatus::AuthorizationExpired;
        run.updated_at_ms = run.authorization.valid_until_ms;
        run.next_action = "授权已过期；重新构建四腿合同并再次明确授权".to_owned();
        run.problem = Some("未在 60 秒授权窗口内认领第一条资金动作".to_owned());
    }
    run
}

fn active_leg_mut(
    run: &mut OnchainCrossChainRun,
) -> Result<&mut OnchainCrossChainLegProgress, String> {
    let position = run
        .active_position
        .ok_or_else(|| "cross-chain active leg is missing".to_owned())?;
    run.legs
        .iter_mut()
        .find(|leg| leg.position == position)
        .ok_or_else(|| "cross-chain active leg progress is missing".to_owned())
}

fn positive_raw(value: &str, label: &str) -> Result<u128, String> {
    value
        .parse::<u128>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or_else(|| format!("{label} amount is invalid"))
}

fn validate_leg_execution(
    kind: shared_types::OnchainCrossChainLegKind,
    position: u8,
    actual_input_amount_raw: &str,
    minimum_output_amount_raw: &str,
    provider_transaction_id: &str,
    swap_execution: Option<&OnchainCrossChainSwapExecution>,
    execution: Option<&OnchainCrossChainBridgeExecution>,
    now_ms: i64,
) -> Result<(), String> {
    let bridge_leg = matches!(
        kind,
        shared_types::OnchainCrossChainLegKind::OutboundBridge
            | shared_types::OnchainCrossChainLegKind::ReturnBridge
    );
    if bridge_leg {
        if swap_execution.is_some() {
            return Err("跨链资金腿不能携带同链兑换交易合同".to_owned());
        }
        let execution =
            execution.ok_or_else(|| "跨链资金腿缺少可恢复的 provider 交易合同".to_owned())?;
        if execution.position != position
            || execution.kind != kind
            || execution.transaction_id != provider_transaction_id
            || execution.from_amount_raw != actual_input_amount_raw
            || execution.to_amount_min_raw != minimum_output_amount_raw
            || execution.valid_until_ms <= now_ms
        {
            return Err("跨链桥交易合同与本腿位置、数量或有效期不一致".to_owned());
        }
        return Ok(());
    }
    if execution.is_some() {
        return Err("同链兑换腿不能携带跨链桥交易合同".to_owned());
    }
    let execution =
        swap_execution.ok_or_else(|| "同链兑换腿缺少可恢复的可签交易合同".to_owned())?;
    if execution.position != position
        || execution.kind != kind
        || execution.execution_id != provider_transaction_id
        || execution.input_amount_raw != actual_input_amount_raw
        || execution.minimum_output_amount_raw != minimum_output_amount_raw
        || execution.valid_until_ms <= now_ms
    {
        return Err("同链兑换交易合同与本腿位置、数量或有效期不一致".to_owned());
    }
    Ok(())
}

fn run_id(idempotency_key: &str) -> String {
    let digest = common::signing::hmac_sha256_hex(RUN_ID_KEY, idempotency_key.as_bytes());
    format!("onchain-cross-chain-run-{}", &digest[..24])
}

fn leg_action_id(run_id: &str, position: u8) -> String {
    let canonical = format!("{run_id}:{position}");
    let digest = common::signing::hmac_sha256_hex(LEG_ACTION_ID_KEY, canonical.as_bytes());
    format!("crossline-{}", &digest[..24])
}

fn storage_problem(path: Option<&Path>) -> Option<String> {
    let path = path?;
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(path)?.sync_all()?;
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    result.err().map(|error| error.to_string())
}

fn append_jsonl(path: &Path, entry: &LogEntry) -> std::io::Result<()> {
    let mut row = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    row.push(b'\n');
    let mut file = OpenOptions::new().append(true).open(path)?;
    file.write_all(&row)?;
    file.flush()?;
    file.sync_data()
}

#[cfg(test)]
mod tests {
    use shared_types::{
        OnchainCrossChainLeg, OnchainCrossChainLegKind, OnchainCrossChainRunStatus,
        OnchainCrossChainSwapExecution, OnchainUnsignedTransaction,
    };

    use super::*;

    fn build(id: &str, valid_until_ms: i64) -> OnchainCrossChainBuildResponse {
        let kinds = [
            OnchainCrossChainLegKind::SourceSwap,
            OnchainCrossChainLegKind::OutboundBridge,
            OnchainCrossChainLegKind::TargetSwap,
            OnchainCrossChainLegKind::ReturnBridge,
        ];
        let legs = kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| OnchainCrossChainLeg {
                position: (index + 1) as u8,
                kind,
                provider: "fixture".to_owned(),
                from_chain: "base".to_owned(),
                to_chain: "base".to_owned(),
                from_asset: "USDC".to_owned(),
                to_asset: "PUPS".to_owned(),
                from_token: format!("token-{index}"),
                to_token: format!("token-{}", index + 1),
                input_amount_raw: "100".to_owned(),
                expected_output_amount_raw: "99".to_owned(),
                minimum_output_amount_raw: Some("98".to_owned()),
                input_decimals: 6,
                output_decimals: 6,
                fee_usd: Some(0.01),
                gas_usd: Some(0.01),
                estimated_duration_seconds: Some(1),
                route_id: Some(format!("route-{index}")),
                route_tools: vec!["fixture".to_owned()],
                official_docs_url: "https://example.test/docs".to_owned(),
                observed_at_ms: 1,
            })
            .collect();
        OnchainCrossChainBuildResponse {
            approval_costs: Vec::new(),
            replenishment_costs: Vec::new(),
            build_id: id.to_owned(),
            provider: "fixture".to_owned(),
            source_chain: "base".to_owned(),
            peer_chain: "arbitrum".to_owned(),
            legs,
            bridge_executions: Vec::new(),
            swap_executions: Vec::new(),
            inventory: Vec::new(),
            initial_quote_amount_raw: "100".to_owned(),
            final_quote_amount_raw: "104".to_owned(),
            gross_return_bps: Some(500.0),
            stablecoin_risk_bps: 50,
            bridge_fee_usd: Some(0.01),
            gas_usd: Some(0.02),
            quote_usd_valuation: None,
            total_cost_bps: Some(10.0),
            net_return_bps: Some(490.0),
            estimated_duration_seconds: Some(10),
            quote_observed_at_ms: 1,
            quote_latency_ms: Some(1),
            built_at_ms: 1,
            valid_until_ms,
            atomic: false,
            monitor_only: false,
            preview_ready: true,
            submit_ready: true,
            blockers: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn swap_execution(
        position: u8,
        input_amount_raw: &str,
        minimum_output_amount_raw: &str,
        valid_until_ms: i64,
    ) -> OnchainCrossChainSwapExecution {
        OnchainCrossChainSwapExecution {
            execution_id: format!("provider-{position}"),
            position,
            kind: if position == 1 {
                OnchainCrossChainLegKind::SourceSwap
            } else {
                OnchainCrossChainLegKind::TargetSwap
            },
            provider: "fixture".to_owned(),
            chain: "base".to_owned(),
            wallet_address: "0x1111111111111111111111111111111111111111".to_owned(),
            input_token: "token-0".to_owned(),
            output_token: "token-1".to_owned(),
            input_amount_raw: input_amount_raw.to_owned(),
            quoted_output_amount_raw: minimum_output_amount_raw.to_owned(),
            minimum_output_amount_raw: minimum_output_amount_raw.to_owned(),
            transaction: OnchainUnsignedTransaction::EvmCall {
                chain_id: 8_453,
                from: "0x1111111111111111111111111111111111111111".to_owned(),
                to: "0x2222222222222222222222222222222222222222".to_owned(),
                data: "0x1234".to_owned(),
                value: "0x0".to_owned(),
                gas: "0x5208".to_owned(),
                gas_price: Some("0x1".to_owned()),
                max_priority_fee_per_gas: None,
                allowance_spender: None,
            },
            quote_observed_at_ms: 20,
            valid_until_ms,
            rebuild_after_position: position.saturating_sub(1),
            official_docs_url: "https://example.test/docs".to_owned(),
        }
    }

    #[test]
    fn cross_chain_recovery_corrupt_tail_preserves_prefix_and_blocks_new_actions() {
        for bad in ["{broken}\n", "{\"schemaVersion\":9}\n", "{unfinished", "\n"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("runs.jsonl");
            let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 10);
            store.insert_build(build("build", 10_000), 10).unwrap();
            let run = store.authorize("build", "key", "tester", 20).unwrap().run;
            OpenOptions::new().append(true).open(&path).unwrap().write_all(bad.as_bytes()).unwrap();
            let before = std::fs::read(&path).unwrap();
            let restored = OnchainCrossChainRunStore::load_path(Some(path.clone()), 30);
            assert_eq!(restored.runs(10, 30).rows[0].run_id, run.run_id);
            assert!(restored.runs(10, 30).recovery_problem.unwrap().contains("第 3 行"));
            assert!(restored.authorize("build", "key", "tester", 30).is_err());
            assert!(restored.insert_build(build("new", 10_000), 30).is_err());
            assert!(restored.pause(&run.run_id, "pause".into(), 30).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn cross_chain_recovery_rejects_frozen_cost_or_identity_changes() {
        for variant in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("runs.jsonl");
            let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 10);
            store.insert_build(build("build", 10_000), 10).unwrap();
            let run = store.authorize("build", "key", "tester", 20).unwrap().run;
            let mut changed = run.clone();
            match variant {
                0 => changed.build.approval_costs.push(crate::services::onchain_comparison::approval_allocation::tests::cost()),
                1 => changed.authorization.actor = "another".into(),
                _ => changed.run_id = "different-run-same-key".into(),
            }
            append_jsonl(&path, &LogEntry { schema_version: SCHEMA_VERSION, build: None, run: Some(changed), recovery_plan: None }).unwrap();
            let restored = OnchainCrossChainRunStore::load_path(Some(path), 30);
            assert!(restored.readiness().unwrap_err().contains("冲突"));
            let verified = restored.runs(10, 30).rows;
            assert_eq!(verified.len(), 1);
            assert_eq!(verified[0].build, run.build);
            assert_eq!(verified[0].authorization, run.authorization);
        }
    }

    #[test]
    fn cross_chain_recovery_missing_journal_and_write_failures_do_not_silently_reset() {
        assert!(OnchainCrossChainRunStore::load_path(None, 10).insert_build(build("b", 1000), 10).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 10);
        store.insert_build(build("build", 10_000), 10).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(store.authorize("build", "key", "tester", 20).is_err());
        assert!(!path.exists());
        std::fs::write(&path, &bytes).unwrap();
        assert!(store.authorize("build", "key", "tester", 20).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let restored = OnchainCrossChainRunStore::load_path(Some(path), 30);
        restored.readiness().unwrap();
        restored.authorize("build", "key", "tester", 30).unwrap();
    }

    #[test]
    fn authorization_is_idempotent_and_survives_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cross-chain.jsonl");
        let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 10);
        let mut preview = build("build-1", 10_000);
        let mut legacy = serde_json::to_value(&preview).unwrap();
        legacy.as_object_mut().unwrap().remove("quoteUsdValuation");
        let legacy: OnchainCrossChainBuildResponse = serde_json::from_value(legacy).unwrap();
        assert!(legacy.quote_usd_valuation.is_none());
        preview.quote_usd_valuation = Some(shared_types::OnchainUsdValuation {
            asset: "USDC".to_owned(),
            venue: "kraken".to_owned(),
            symbol: "USDC/USD".to_owned(),
            source: "ws_push".to_owned(),
            usd_bid: 0.8,
            usd_ask: 0.81,
            observed_at_ms: 10,
        });
        store.insert_build(preview.clone(), 10).expect("build");
        let first = store
            .authorize("build-1", "same-key", "tester", 20)
            .expect("authorize");
        let replay = store
            .authorize("build-1", "same-key", "tester", 30)
            .expect("replay");
        assert!(!first.replayed);
        assert!(replay.replayed);
        assert_eq!(first.run.run_id, replay.run.run_id);
        assert!(matches!(
            store.authorize("build-1", "same-key", "different-actor", 30),
            Err(CrossChainAuthorizeError::IdempotencyConflict)
        ));

        let restored = OnchainCrossChainRunStore::load_path(Some(path), 40);
        let rows = restored.runs(10, 40);
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0].run_id, first.run.run_id);
        assert_eq!(
            rows.rows[0].build.quote_usd_valuation,
            preview.quote_usd_valuation
        );
    }

    #[test]
    fn claimed_leg_is_paused_after_restart_instead_of_replayed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cross-chain.jsonl");
        let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 10);
        store
            .insert_build(build("build-1", 10_000), 10)
            .expect("build");
        let authorized = store
            .authorize("build-1", "claim-key", "tester", 20)
            .expect("authorize");
        store
            .claim_leg(
                &authorized.run.run_id,
                "tester",
                1,
                "100".to_owned(),
                "98".to_owned(),
                "provider-1".to_owned(),
                Some(swap_execution(1, "100", "98", 1_000)),
                None,
                20,
                1_000,
                "101".to_owned(),
                "100".to_owned(),
                "10".to_owned(),
                30,
            )
            .expect("claim");

        let restored = OnchainCrossChainRunStore::load_path(Some(path), 40);
        let run = restored.runs(10, 40).rows.remove(0);
        assert_eq!(run.status, OnchainCrossChainRunStatus::Paused);
        assert!(run
            .problem
            .as_deref()
            .is_some_and(|value| value.contains("禁止自动重复提交")));
    }

    #[test]
    fn destination_evidence_advances_using_exact_previous_output() {
        let dir = tempfile::tempdir().unwrap();
        let store = OnchainCrossChainRunStore::load_path(Some(dir.path().join("runs.jsonl")), 10);
        store
            .insert_build(build("build-1", 10_000), 10)
            .expect("build");
        let run = store
            .authorize("build-1", "advance-key", "tester", 20)
            .expect("authorize")
            .run;
        store
            .claim_leg(
                &run.run_id,
                "tester",
                1,
                "100".to_owned(),
                "98".to_owned(),
                "provider-1".to_owned(),
                Some(swap_execution(1, "100", "98", 1_000)),
                None,
                20,
                1_000,
                "101".to_owned(),
                "100".to_owned(),
                "10".to_owned(),
                30,
            )
            .expect("claim");
        store
            .record_submission_intent(&run.run_id, "0xsource".to_owned(), "rpc".to_owned(), 40)
            .expect("intent");
        store
            .record_source_confirmed(&run.run_id, "rpc_receipt".to_owned(), 50)
            .expect("source finality");
        let advanced = store
            .record_destination_evidence(
                &run.run_id,
                "97".to_owned(),
                Some("0xdestination".to_owned()),
                "wallet_delta".to_owned(),
                60,
            )
            .expect("destination");
        assert_eq!(advanced.active_position, None);
        assert_eq!(advanced.status, OnchainCrossChainRunStatus::Paused);
        assert_eq!(
            store.request_recheck(&run.run_id, "tester", 1, 61).unwrap(),
            advanced
        );
        assert_eq!(
            advanced.legs[0].actual_output_amount_raw.as_deref(),
            Some("97")
        );
        assert_eq!(
            advanced.legs[1].status,
            OnchainCrossChainLegRunStatus::RequoteRequired
        );

        let rejected = store.claim_leg(
            &run.run_id,
            "tester",
            2,
            "97".to_owned(),
            "95".to_owned(),
            "provider-2".to_owned(),
            None,
            None,
            60,
            2_000,
            "101".to_owned(),
            "100".to_owned(),
            "10".to_owned(),
            70,
        );
        assert!(matches!(
            rejected,
            Err(CrossChainLegClaimError::InvalidQuote(_))
        ));
    }

    #[test]
    fn pruning_preserves_unresolved_runs_and_idempotency_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cross-chain.jsonl");
        let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 10);
        store.insert_build(build("build", 1_000_000), 10).unwrap();
        let pending = store
            .authorize("build", "pending", "tester", 20)
            .unwrap()
            .run;
        store
            .pause(&pending.run_id, "awaiting manual reconciliation".into(), 21)
            .unwrap();
        let completed = store
            .authorize("build", "completed", "tester", 22)
            .unwrap()
            .run;
        store
            .update_run(
                &completed.run_id,
                |run| {
                    run.status = OnchainCrossChainRunStatus::Completed;
                    Ok(())
                },
                23,
            )
            .unwrap();
        for index in 0..MAX_STORED_RUNS {
            store
                .authorize(
                    "build",
                    &format!("pending-{index}"),
                    "tester",
                    30 + index as i64,
                )
                .unwrap();
        }
        assert!(store.run(&completed.run_id, 200).is_none());
        assert!(store.run(&pending.run_id, 200).is_some());
        assert_eq!(store.runs(1, 200).rows.len(), MAX_STORED_RUNS + 1);

        let restored = OnchainCrossChainRunStore::load_path(Some(path), 200);
        assert_eq!(restored.runs(1, 200).rows.len(), MAX_STORED_RUNS + 1);
        assert!(restored.run(&pending.run_id, 200).is_some());
        assert!(matches!(
            restored.authorize("build", "completed", "tester", 201),
            Err(CrossChainAuthorizeError::IdempotencyConflict)
        ));
    }

    #[test]
    fn paused_run_cannot_be_claimed_by_a_late_preflight_response() {
        let dir = tempfile::tempdir().unwrap();
        let store = OnchainCrossChainRunStore::load_path(Some(dir.path().join("runs.jsonl")), 10);
        store.insert_build(build("build", 10_000), 10).unwrap();
        let run = store.authorize("build", "key", "tester", 20).unwrap().run;
        store
            .pause(&run.run_id, "user paused during preflight".into(), 25)
            .unwrap();
        assert!(matches!(
            store.claim_leg(
                &run.run_id,
                "tester",
                1,
                "100".into(),
                "98".into(),
                "provider-1".into(),
                Some(swap_execution(1, "100", "98", 1_000)),
                None,
                20,
                1_000,
                "101".into(),
                "100".into(),
                "10".into(),
                30
            ),
            Err(CrossChainLegClaimError::InvalidQuote(_))
        ));
        assert_eq!(
            store.run(&run.run_id, 30).unwrap().status,
            OnchainCrossChainRunStatus::Paused
        );
    }

    #[test]
    fn recheck_restores_only_recorded_transaction_queries_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recheck.jsonl");
        let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 10);
        store.insert_build(build("build", 10_000), 10).unwrap();
        let run = store.authorize("build", "key", "tester", 20).unwrap().run;
        store
            .claim_leg(
                &run.run_id,
                "tester",
                1,
                "100".into(),
                "98".into(),
                "provider-1".into(),
                Some(swap_execution(1, "100", "98", 1_000)),
                None,
                20,
                1_000,
                "101".into(),
                "100".into(),
                "10".into(),
                30,
            )
            .unwrap();
        store
            .record_submission_intent(&run.run_id, "0xoriginal".into(), "rpc".into(), 40)
            .unwrap();
        store
            .pause(&run.run_id, "provider timeout".into(), 50)
            .unwrap();
        let queued = store.request_recheck(&run.run_id, "tester", 1, 60).unwrap();
        assert_eq!(
            queued.status,
            OnchainCrossChainRunStatus::AwaitingSourceFinality
        );
        assert_eq!(
            queued.legs[0].source_transaction_id.as_deref(),
            Some("0xoriginal")
        );
        assert_eq!(queued.legs[0].attempts, 1);
        assert!(queued.legs[0].last_checked_at_ms.is_none());
        assert_eq!(
            queued.legs[1].status,
            OnchainCrossChainLegRunStatus::RequoteRequired
        );
        assert_eq!(
            queued,
            store.request_recheck(&run.run_id, "tester", 1, 61).unwrap()
        );

        let restored = OnchainCrossChainRunStore::load_path(Some(path), 70);
        assert_eq!(restored.run(&run.run_id, 70).unwrap(), queued);
        restored
            .record_source_confirmed(&run.run_id, "rpc".into(), 80)
            .unwrap();
        let credited = restored
            .record_destination_evidence(
                &run.run_id,
                "99".into(),
                Some("0xoriginal".into()),
                "wallet net credit".into(),
                90,
            )
            .unwrap();
        assert_eq!(credited.status, OnchainCrossChainRunStatus::Running);
        assert_eq!(credited.active_position, None);
        assert_eq!(credited.legs[1].attempts, 0);
        assert_eq!(
            credited,
            restored
                .request_recheck(&run.run_id, "tester", 1, 100)
                .unwrap()
        );
    }

    #[test]
    fn recheck_rejects_wrong_actor_step_and_unknown_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let store = OnchainCrossChainRunStore::load_path(Some(dir.path().join("runs.jsonl")), 10);
        store.insert_build(build("build", 10_000), 10).unwrap();
        let run = store.authorize("build", "key", "tester", 20).unwrap().run;
        store
            .claim_leg(
                &run.run_id,
                "tester",
                1,
                "100".into(),
                "98".into(),
                "provider-1".into(),
                Some(swap_execution(1, "100", "98", 1_000)),
                None,
                20,
                1_000,
                "101".into(),
                "100".into(),
                "10".into(),
                30,
            )
            .unwrap();
        let paused = store
            .pause(&run.run_id, "interrupted before a durable hash".into(), 40)
            .unwrap();
        assert!(matches!(
            store.request_recheck(&run.run_id, "intruder", 1, 50),
            Err(CrossChainRecheckError::ActorMismatch)
        ));
        assert!(matches!(
            store.request_recheck(&run.run_id, "tester", 2, 50),
            Err(CrossChainRecheckError::InvalidState(_))
        ));
        assert!(matches!(
            store.request_recheck(&run.run_id, "tester", 1, 50),
            Err(CrossChainRecheckError::InvalidState(_))
        ));
        assert_eq!(store.run(&run.run_id, 50).unwrap(), paused);
    }
}
