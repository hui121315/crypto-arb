use common::config::AppConfig;
use dashmap::DashMap;
use parking_lot::Mutex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use shared_types::{
    OnchainReplenishmentAuthorizationEvidence, OnchainReplenishmentCostValuation,
    OnchainReplenishmentNetworkCost, OnchainReplenishmentPlanResponse,
    OnchainReplenishmentPlanStatus, OnchainReplenishmentPlansResponse, OnchainReplenishmentRun,
    OnchainReplenishmentRunStatus, OnchainReplenishmentRunsResponse,
    OnchainReplenishmentTransferProgress, OnchainReplenishmentTransferStatus,
    OnchainReplenishmentWithdrawalCost,
};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const AUTHORIZATION_VALIDITY_MS: i64 = 60_000;
const MAX_STORED_PLANS: usize = 128;
// A history cache limit, never a limit on unresolved funds tracking.
const MAX_STORED_RUNS: usize = 128;
const RUN_ID_KEY: &[u8] = b"crossline-onchain-replenishment-run-v1";
const TRANSFER_ID_KEY: &[u8] = b"crossline-onchain-replenishment-transfer-v1";
const SCHEMA_VERSION: u8 = 1;

mod source_guard;
mod wallet_claims;
pub(crate) use source_guard::ReplenishmentSubmissionSnapshot;

struct SourceStatusUpdate {
    status: OnchainReplenishmentTransferStatus,
    provider_transfer_id: String,
    transaction_id: Option<String>,
    confirmations: Option<u64>,
    evidence_source: String,
    problem: Option<String>,
    withdrawal_evidence: Option<exchange::WithdrawalStatusEvidence>,
    network_cost: Option<OnchainReplenishmentNetworkCost>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<OnchainReplenishmentPlanResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<OnchainReplenishmentRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplenishmentAuthorizeError {
    Missing,
    Expired,
    NotReady(String),
    IdempotencyConflict,
    Persistence(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplenishmentSubmitClaimError {
    Missing,
    AuthorizationExpired,
    ActorMismatch,
    InvalidLeg,
    PreviousLegIncomplete,
    SourceBusy(String),
    SourceChanged,
    Persistence(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReplenishmentSubmitClaimOutcome {
    pub(crate) run: OnchainReplenishmentRun,
    pub(crate) replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReplenishmentAuthorizeOutcome {
    pub(crate) run: OnchainReplenishmentRun,
    pub(crate) replayed: bool,
}

#[derive(Debug)]
pub(crate) struct OnchainReplenishmentPlanStore {
    plans: DashMap<String, OnchainReplenishmentPlanResponse>,
    runs: DashMap<String, OnchainReplenishmentRun>,
    run_by_idempotency: DashMap<String, String>,
    path: Option<PathBuf>,
    ledger_lock: Mutex<()>,
    persistence_problem: Mutex<Option<String>>,
    recovery_problem: Option<String>,
    wallet_claims: std::sync::Arc<super::onchain_wallet_claims::WalletClaims>,
}

impl OnchainReplenishmentPlanStore {
    pub(crate) fn load(config: &AppConfig) -> Self {
        let path = config
            .storage
            .onchain_replenishment_ledger_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| config.storage.resolve_runtime_path(path));
        Self::load_path(path, common::time::now_ms())
    }

    fn load_path(path: Option<PathBuf>, now_ms: i64) -> Self {
        let replay = replay(path.as_deref(), now_ms);
        let persistence_problem = storage_problem(path.as_deref());
        let run_by_idempotency = replay
            .runs
            .iter()
            .map(|run| (run.idempotency_key.clone(), run.run_id.clone()))
            .collect();
        let store = Self {
            plans: replay
                .plans
                .into_iter()
                .map(|plan| (plan.plan_id.clone(), plan))
                .collect(),
            runs: replay
                .runs
                .into_iter()
                .map(|run| (run.run_id.clone(), run))
                .collect(),
            run_by_idempotency,
            path,
            ledger_lock: Mutex::new(()),
            persistence_problem: Mutex::new(persistence_problem),
            recovery_problem: replay.problem,
            wallet_claims: Default::default(),
        };
        store.prune_runs(now_ms);
        while store.plans.len() > MAX_STORED_PLANS {
            store.remove_oldest_plan();
        }
        store.restore_wallet_claims();
        store
    }

    pub(crate) fn insert(
        &self,
        response: OnchainReplenishmentPlanResponse,
        now_ms: i64,
    ) -> Result<(), String> {
        self.prune_plans(now_ms);
        let _guard = self.ledger_lock.lock();
        let is_new = !self.plans.contains_key(&response.plan_id);
        if is_new {
            self.append_unlocked(&LogEntry {
                schema_version: SCHEMA_VERSION,
                plan: Some(response.clone()),
                run: None,
            })?;
        }
        self.plans.insert(response.plan_id.clone(), response);
        if self.plans.len() > MAX_STORED_PLANS {
            self.remove_oldest_plan();
        }
        Ok(())
    }

    pub(crate) fn authorize(
        &self,
        plan_id: &str,
        idempotency_key: &str,
        actor: &str,
        now_ms: i64,
    ) -> Result<ReplenishmentAuthorizeOutcome, ReplenishmentAuthorizeError> {
        let _guard = self.ledger_lock.lock();
        if let Some(run_id) = self.run_by_idempotency.get(idempotency_key) {
            let run = self
                .runs
                .get(run_id.value())
                .map(|entry| entry.value().clone())
                .ok_or(ReplenishmentAuthorizeError::IdempotencyConflict)?;
            if run.plan.plan_id != plan_id || run.authorization.actor != actor {
                return Err(ReplenishmentAuthorizeError::IdempotencyConflict);
            }
            return Ok(ReplenishmentAuthorizeOutcome {
                run: project_expiry(run, now_ms),
                replayed: true,
            });
        }
        let plan = self
            .plans
            .get(plan_id)
            .map(|entry| entry.value().clone())
            .ok_or(ReplenishmentAuthorizeError::Missing)?;
        if plan.valid_until_ms < now_ms {
            self.plans.remove(plan_id);
            return Err(ReplenishmentAuthorizeError::Expired);
        }
        if !plan.submit_ready
            || plan.status != OnchainReplenishmentPlanStatus::ReadyForAuthorization
            || !plan.blockers.is_empty()
        {
            return Err(ReplenishmentAuthorizeError::NotReady(
                plan.blockers
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "补仓计划尚未满足授权条件".to_owned()),
            ));
        }
        let run_id = run_id(idempotency_key);
        let run = OnchainReplenishmentRun {
            run_id: run_id.clone(),
            plan,
            idempotency_key: idempotency_key.to_owned(),
            status: OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit,
            authorization: OnchainReplenishmentAuthorizationEvidence {
                actor: actor.to_owned(),
                authorized_at_ms: now_ms,
                valid_until_ms: now_ms.saturating_add(AUTHORIZATION_VALIDITY_MS),
                confirmation_version: "onchain-replenishment-v1".to_owned(),
            },
            transfers: Vec::new(),
            revalidated_at_ms: None,
            read_only_recovery: false,
            recovery_checks: 0,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            next_action: "在授权过期前重新核验计划并提交一次资金动作".to_owned(),
            problem: None,
        };
        self.append_unlocked(&LogEntry {
            schema_version: SCHEMA_VERSION,
            plan: None,
            run: Some(run.clone()),
        })
        .map_err(ReplenishmentAuthorizeError::Persistence)?;
        self.runs.insert(run_id.clone(), run.clone());
        self.run_by_idempotency
            .insert(idempotency_key.to_owned(), run_id);
        self.prune_runs(now_ms);
        Ok(ReplenishmentAuthorizeOutcome {
            run,
            replayed: false,
        })
    }

    pub(crate) fn plans(&self, limit: usize, now_ms: i64) -> OnchainReplenishmentPlansResponse {
        self.prune_plans(now_ms);
        let mut rows = self
            .plans
            .iter()
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| right.built_at_ms.cmp(&left.built_at_ms));
        rows.truncate(limit.clamp(1, MAX_STORED_PLANS));
        OnchainReplenishmentPlansResponse {
            rows,
            observed_at_ms: now_ms,
        }
    }

    pub(crate) fn runs(&self, limit: usize, now_ms: i64) -> OnchainReplenishmentRunsResponse {
        let mut rows = self
            .runs
            .iter()
            .map(|entry| project_expiry(entry.value().clone(), now_ms))
            .collect::<Vec<_>>();
        rows.sort_by_key(|run| {
            (
                run_priority(run, now_ms),
                std::cmp::Reverse(run.updated_at_ms),
            )
        });
        // The history page limit must never hide a still-unresolved transfer.
        let unresolved = rows.iter().filter(|run| !settled(run, now_ms)).count();
        rows.truncate(limit.clamp(1, MAX_STORED_RUNS).max(unresolved));
        OnchainReplenishmentRunsResponse {
            rows,
            observed_at_ms: now_ms,
            recovery_problem: self.recovery_problem.clone().or_else(|| {
                self.persistence_problem.lock().as_ref().map(|problem| {
                    format!("补仓日志不可写，资金动作已停用；请核对日志并重启：{problem}")
                })
            }),
        }
    }

    pub(crate) fn run(&self, run_id: &str, now_ms: i64) -> Option<OnchainReplenishmentRun> {
        self.runs
            .get(run_id)
            .map(|entry| project_expiry(entry.value().clone(), now_ms))
    }

    pub(crate) fn claim_submission(
        &self,
        run_id: &str,
        actor: &str,
        revalidated_plan: OnchainReplenishmentPlanResponse,
        leg_index: usize,
        source_snapshot: &ReplenishmentSubmissionSnapshot,
        now_ms: i64,
    ) -> Result<ReplenishmentSubmitClaimOutcome, ReplenishmentSubmitClaimError> {
        let _guard = self.ledger_lock.lock();
        let mut run = self
            .runs
            .get(run_id)
            .map(|entry| entry.value().clone())
            .ok_or(ReplenishmentSubmitClaimError::Missing)?;
        if run.authorization.actor != actor {
            return Err(ReplenishmentSubmitClaimError::ActorMismatch);
        }
        if run.read_only_recovery {
            return Err(ReplenishmentSubmitClaimError::PreviousLegIncomplete);
        }
        let first_transfer = run.status == OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit;
        let next_transfer = run.status == OnchainReplenishmentRunStatus::ReadyForNextTransfer;
        if !first_transfer && !next_transfer {
            return Ok(ReplenishmentSubmitClaimOutcome {
                run: project_expiry(run, now_ms),
                replayed: true,
            });
        }
        if first_transfer && run.authorization.valid_until_ms < now_ms {
            return Err(ReplenishmentSubmitClaimError::AuthorizationExpired);
        }
        if revalidated_plan.legs.get(leg_index).is_none() || leg_index != run.transfers.len() {
            return Err(ReplenishmentSubmitClaimError::InvalidLeg);
        }
        if run.transfers.iter().any(|transfer| {
            transfer.status != OnchainReplenishmentTransferStatus::DestinationCredited
        }) {
            return Err(ReplenishmentSubmitClaimError::PreviousLegIncomplete);
        }
        self.check_submission_source(
            &revalidated_plan.legs[leg_index],
            source_snapshot,
        )?;
        let leg_index =
            u32::try_from(leg_index).map_err(|_| ReplenishmentSubmitClaimError::InvalidLeg)?;
        let direction = revalidated_plan.legs[leg_index as usize].direction;
        run.plan = revalidated_plan;
        run.status = OnchainReplenishmentRunStatus::Submitting;
        run.revalidated_at_ms = Some(now_ms);
        run.updated_at_ms = now_ms;
        run.next_action = match direction {
            shared_types::OnchainTransferDirection::WithdrawToChain => {
                "已持久化提交占位；等待交易所确认本次提币请求".to_owned()
            }
            shared_types::OnchainTransferDirection::DepositToCex => {
                "已持久化提交占位；下一步仅广播已签名链上转账".to_owned()
            }
        };
        run.problem = None;
        run.transfers.push(OnchainReplenishmentTransferProgress {
            leg_index,
            client_transfer_id: transfer_id(run_id, leg_index),
            provider_transfer_id: None,
            status: OnchainReplenishmentTransferStatus::SubmissionClaimed,
            submission_attempted_at_ms: now_ms,
            last_checked_at_ms: None,
            transaction_id: None,
            confirmations: None,
            credited_amount_exact: None,
            reported_deposit_amount_exact: None,
            deposit_fee_exact: None,
            withdrawal_unlocked: None,
            withdrawal_cost: None,
            network_cost: None,
            evidence_source: None,
            problem: None,
        });
        self.persist_run_unlocked(&run)
            .map_err(ReplenishmentSubmitClaimError::Persistence)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(ReplenishmentSubmitClaimOutcome {
            run,
            replayed: false,
        })
    }

    pub(crate) fn record_submission_ack(
        &self,
        run_id: &str,
        provider_transfer_id: String,
        transaction_id: Option<String>,
        submitted_at_ms: i64,
        evidence_source: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                let transfer = active_transfer_mut(run)?;
                transfer.provider_transfer_id = Some(provider_transfer_id);
                transfer.transaction_id = transaction_id;
                transfer.status = OnchainReplenishmentTransferStatus::Submitted;
                transfer.submission_attempted_at_ms = submitted_at_ms;
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.evidence_source = Some(evidence_source);
                transfer.problem = None;
                run.status = OnchainReplenishmentRunStatus::AwaitingSourceFinality;
                run.next_action = match active_leg_direction(run)? {
                    shared_types::OnchainTransferDirection::WithdrawToChain => {
                        "等待交易所提币终态；恢复流程只查询，不重复提交".to_owned()
                    }
                    shared_types::OnchainTransferDirection::DepositToCex => {
                        "等待链上转账终态；恢复流程只查询交易哈希，不重复广播".to_owned()
                    }
                };
                run.problem = None;
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn record_chain_submission_intent(
        &self,
        run_id: &str,
        transaction_id: String,
        evidence_source: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                if active_leg_direction(run)?
                    != shared_types::OnchainTransferDirection::DepositToCex
                {
                    return Err("chain submission intent requires a deposit-to-CEX leg".to_owned());
                }
                let transfer = active_transfer_mut(run)?;
                transfer.provider_transfer_id = Some(transaction_id.clone());
                transfer.transaction_id = Some(transaction_id);
                transfer.evidence_source = Some(evidence_source);
                transfer.last_checked_at_ms = Some(now_ms);
                run.next_action =
                    "链上交易哈希已持久化；下一步只允许广播这一笔已签名交易".to_owned();
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn pause_submission(
        &self,
        run_id: &str,
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                let transfer = active_transfer_mut(run)?;
                if !matches!(transfer.status, OnchainReplenishmentTransferStatus::SourceCompleted
                    | OnchainReplenishmentTransferStatus::DestinationCredited) {
                    transfer.status = OnchainReplenishmentTransferStatus::Paused;
                }
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.problem = Some(problem.clone());
                run.status = OnchainReplenishmentRunStatus::Paused;
                run.next_action = match active_leg_direction(run)? {
                    shared_types::OnchainTransferDirection::WithdrawToChain => {
                        "人工核对交易所提币历史与目标地址到账记录后再决定恢复".to_owned()
                    }
                    shared_types::OnchainTransferDirection::DepositToCex => {
                        "人工核对链上交易哈希与交易所充值历史后再决定恢复".to_owned()
                    }
                };
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn reject_before_send(
        &self,
        run_id: &str,
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(run_id, |run| {
            if run.status != OnchainReplenishmentRunStatus::Submitting {
                return Err("只能结束尚未发送的提交占位".into());
            }
            let transfer = active_transfer_mut(run)?;
            if transfer.status != OnchainReplenishmentTransferStatus::SubmissionClaimed
                || transfer.provider_transfer_id.is_some()
                || transfer.transaction_id.is_some()
            {
                return Err("已有发送证据，不能标记为未发送".into());
            }
            transfer.status = OnchainReplenishmentTransferStatus::Failed;
            transfer.last_checked_at_ms = Some(now_ms);
            transfer.evidence_source = Some("local_withdrawal_preflight".into());
            transfer.problem = Some(problem.clone());
            run.status = OnchainReplenishmentRunStatus::Failed;
            run.problem = Some(problem);
            run.next_action = "本次未发送资金请求；修正配置后重新生成计划并授权".into();
            Ok(())
        }, now_ms)
    }

    pub(crate) fn record_source_status(
        &self,
        run_id: &str,
        status: OnchainReplenishmentTransferStatus,
        provider_transfer_id: String,
        transaction_id: Option<String>,
        confirmations: Option<u64>,
        evidence_source: String,
        problem: Option<String>,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.record_source_update(
            run_id,
            SourceStatusUpdate {
                status,
                provider_transfer_id,
                transaction_id,
                confirmations,
                evidence_source,
                problem,
                withdrawal_evidence: None,
                network_cost: None,
            },
            now_ms,
        )
    }

    pub(crate) fn record_withdrawal_status(
        &self,
        run_id: &str,
        evidence: exchange::WithdrawalStatusEvidence,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        let status = match evidence.status {
            exchange::WithdrawalStatus::Pending => OnchainReplenishmentTransferStatus::Submitted,
            exchange::WithdrawalStatus::Completed => {
                OnchainReplenishmentTransferStatus::SourceCompleted
            }
            _ => OnchainReplenishmentTransferStatus::Failed,
        };
        let problem = evidence.problem.clone().or_else(|| {
            (status == OnchainReplenishmentTransferStatus::Failed)
                .then(|| format!("交易所提币终态：{:?}", evidence.status))
        });
        self.record_source_update(
            run_id,
            SourceStatusUpdate {
                status,
                provider_transfer_id: evidence.provider_withdrawal_id.clone(),
                transaction_id: evidence.transaction_id.clone(),
                confirmations: evidence.confirmations,
                evidence_source: evidence.source_url.clone(),
                problem,
                withdrawal_evidence: Some(evidence),
                network_cost: None,
            },
            now_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_chain_source_status(
        &self,
        run_id: &str,
        transaction_id: String,
        status: OnchainReplenishmentTransferStatus,
        confirmations: Option<u64>,
        source: String,
        problem: Option<String>,
        network_cost: Option<OnchainReplenishmentNetworkCost>,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.record_source_update(
            run_id,
            SourceStatusUpdate {
                status,
                provider_transfer_id: transaction_id.clone(),
                transaction_id: Some(transaction_id),
                confirmations,
                evidence_source: source,
                problem,
                withdrawal_evidence: None,
                network_cost,
            },
            now_ms,
        )
    }

    fn record_source_update(
        &self,
        run_id: &str,
        update: SourceStatusUpdate,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        let now_ms = update
            .withdrawal_evidence
            .as_ref()
            .map_or(now_ms, |evidence| now_ms.max(evidence.checked_at_ms));
        let now_ms = update
            .network_cost
            .as_ref()
            .map_or(now_ms, |cost| now_ms.max(cost.observed_at_ms));
        let SourceStatusUpdate {
            status,
            provider_transfer_id,
            transaction_id,
            confirmations,
            evidence_source,
            mut problem,
            withdrawal_evidence,
            mut network_cost,
        } = update;
        self.update_run(
            run_id,
            |run| {
                if let Some(cost) = &mut network_cost {
                    if cost.usd_valuation.is_none() {
                        cost.usd_valuation = run.transfers.last().and_then(|transfer| transfer.network_cost.as_ref())
                            .and_then(|old| old.usd_valuation.clone());
                    }
                }
                if let Some(cost) = &network_cost {
                    validate_network_cost(run, cost)?;
                    if problem.is_none() && cost.total_fee_exact.is_none() {
                        problem = Some(cost.problem.clone().unwrap_or_else(|| "链上网络费待核验；继续追踪到账，不按零费用计算收益".into()));
                    }
                }
                let cost = withdrawal_evidence.as_ref().map(|evidence| {
                    validate_withdrawal_cost(run, evidence)?;
                    if let Some(old) = run.transfers.last().and_then(|transfer| transfer.withdrawal_cost.as_ref()).filter(|cost| cost.confirmed) {
                        return Ok(old.clone());
                    }
                    Ok::<_, String>(OnchainReplenishmentWithdrawalCost {
                        asset: evidence.currency.clone(),
                        reported_amount_exact: evidence.amount.normalize().to_string(),
                        fee_exact: evidence.transaction_fee.normalize().to_string(),
                        confirmed: evidence.status == exchange::WithdrawalStatus::Completed,
                        source: evidence.source_url.clone(),
                        observed_at_ms: evidence.checked_at_ms,
                        usd_valuation: None,
                    })
                }).transpose()?;
                if let Some(evidence) = &withdrawal_evidence {
                    let leg = run.transfers.last().and_then(|transfer| run.plan.legs.get(transfer.leg_index as usize))
                        .ok_or("提币步骤不存在")?;
                    let planned_fee = leg.economics.fee_amount_exact.as_deref()
                        .and_then(|value| value.parse::<Decimal>().ok());
                    if evidence.status == exchange::WithdrawalStatus::Completed
                        && planned_fee.is_none_or(|fee| fee < Decimal::ZERO || evidence.transaction_fee > fee)
                    {
                        problem = Some(format!(
                            "实扣提币费 {} {} 超出计划费用或原费用未留存；继续核验到账，后续资金步骤需重新确认",
                            evidence.transaction_fee.normalize(), evidence.currency
                        ));
                    }
                }
                let transfer = active_transfer_mut(run)?;
                if let Some(cost) = cost {
                    transfer.withdrawal_cost = Some(cost);
                }
                if let Some(cost) = network_cost {
                    transfer.network_cost = Some(cost);
                }
                transfer.status = status;
                transfer.provider_transfer_id = Some(provider_transfer_id);
                transfer.transaction_id = transaction_id.or_else(|| transfer.transaction_id.clone());
                transfer.confirmations = confirmations;
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.evidence_source = Some(evidence_source);
                transfer.problem = problem.clone();
                match status {
                    OnchainReplenishmentTransferStatus::Submitted => {
                        run.status = OnchainReplenishmentRunStatus::AwaitingSourceFinality;
                        run.next_action = match active_leg_direction(run)? {
                            shared_types::OnchainTransferDirection::WithdrawToChain => {
                                "继续查询交易所提币终态".to_owned()
                            }
                            shared_types::OnchainTransferDirection::DepositToCex => {
                                "继续按交易哈希查询链上转账终态".to_owned()
                            }
                        };
                    }
                    OnchainReplenishmentTransferStatus::SourceCompleted => {
                        run.status = OnchainReplenishmentRunStatus::AwaitingDestinationCredit;
                        run.next_action = match active_leg_direction(run)? {
                            shared_types::OnchainTransferDirection::WithdrawToChain => {
                                "交易所已完成提币；等待目标链到账确认".to_owned()
                            }
                            shared_types::OnchainTransferDirection::DepositToCex => {
                                "链上精确转账已确认；等待交易所官方充值历史入账".to_owned()
                            }
                        };
                    }
                    OnchainReplenishmentTransferStatus::Failed => {
                        run.status = OnchainReplenishmentRunStatus::Failed;
                        run.next_action = "核对失败原因并重新生成补仓计划".to_owned();
                    }
                    OnchainReplenishmentTransferStatus::Paused => {
                        run.status = OnchainReplenishmentRunStatus::Paused;
                        run.next_action =
                            "已保留交易哈希；先核对原交易、资产形态与实际到账，不要重复转账"
                                .to_owned();
                    }
                    _ => return Err("unsupported source status transition".to_owned()),
                }
                run.problem = problem;
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn record_source_check_problem(
        &self,
        run_id: &str,
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                let transfer = active_transfer_mut(run)?;
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.problem = Some(problem.clone());
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn record_destination_credit(
        &self,
        run_id: &str,
        amount: Decimal,
        withdrawal_unlocked: Option<bool>,
        confirmations: Option<u64>,
        evidence_source: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.record_credit(
            run_id,
            amount,
            withdrawal_unlocked,
            confirmations,
            evidence_source,
            None,
            now_ms,
        )
    }

    pub(crate) fn record_cex_destination_credit(
        &self,
        run_id: &str,
        evidence: &exchange::DepositStatusEvidence,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        if evidence.deposit_fee.is_some_and(|fee| fee < Decimal::ZERO) {
            return Err("充值费不能为负数".into());
        }
        if evidence.deposit_fee.is_some_and(|fee| fee > Decimal::ZERO) {
            return self.record_destination_fee_review(run_id, evidence, now_ms);
        }
        let unlocked = match evidence.status {
            exchange::DepositStatus::CreditedLocked => Some(false),
            _ if evidence.venue.eq_ignore_ascii_case("binance") => Some(true),
            _ => None,
        };
        self.record_credit(
            run_id,
            evidence.amount,
            unlocked,
            evidence.confirmations,
            evidence.source_url.clone(),
            Some(evidence),
            now_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record_credit(
        &self,
        run_id: &str,
        amount: Decimal,
        withdrawal_unlocked: Option<bool>,
        confirmations: Option<u64>,
        evidence_source: String,
        deposit: Option<&exchange::DepositStatusEvidence>,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                if run.status != OnchainReplenishmentRunStatus::AwaitingDestinationCredit {
                    return Err("replenishment run is not awaiting destination credit".to_owned());
                }
                let direction = active_leg_direction(run)?;
                let leg_index = active_transfer_mut(run)?.leg_index as usize;
                let expected = run
                    .plan
                    .legs
                    .get(leg_index)
                    .and_then(|leg| leg.transfer_amount_exact.as_deref())
                    .and_then(|amount| amount.parse::<Decimal>().ok())
                    .filter(|amount| *amount > Decimal::ZERO)
                    .ok_or_else(|| "replenishment expected amount is missing".to_owned())?;
                if amount < Decimal::ZERO {
                    return Err("replenishment credit amount cannot be negative".to_owned());
                }
                let shortfall = amount < expected;
                let transfer = active_transfer_mut(run)?;
                transfer.status = OnchainReplenishmentTransferStatus::DestinationCredited;
                if let Some(deposit) = deposit {
                    transfer.reported_deposit_amount_exact =
                        Some(deposit.amount.normalize().to_string());
                    transfer.deposit_fee_exact =
                        deposit.deposit_fee.map(|fee| fee.normalize().to_string());
                }
                transfer.credited_amount_exact = Some(amount.normalize().to_string());
                transfer.withdrawal_unlocked = withdrawal_unlocked;
                transfer.confirmations = confirmations;
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.evidence_source = Some(evidence_source);
                transfer.problem = None;
                if shortfall {
                    let problem = format!(
                        "实际到账 {}，低于计划 {}；请核对充值费用或扣减后重新规划",
                        amount.normalize(),
                        expected.normalize()
                    );
                    transfer.problem = Some(problem.clone());
                    run.status = OnchainReplenishmentRunStatus::Paused;
                    run.next_action = "已记录实际到账，后续资金动作已暂停".to_owned();
                    run.problem = Some(problem);
                    return Ok(());
                }
                if withdrawal_unlocked == Some(false) {
                    run.next_action =
                        "交易所已入账并可交易；等待解锁提币，仅查询原充值记录，不重复转账"
                            .to_owned();
                    run.problem = None;
                    return Ok(());
                }
                let destination = match direction {
                    shared_types::OnchainTransferDirection::WithdrawToChain => {
                        "目标地址、代币身份与精确链上到账数量均已确认"
                    }
                    shared_types::OnchainTransferDirection::DepositToCex => {
                        "交易所官方充值历史已确认精确入账"
                    }
                };
                if leg_index + 1 < run.plan.legs.len() {
                    if run.read_only_recovery {
                        run.status = OnchainReplenishmentRunStatus::Paused;
                        run.next_action = format!("{destination}；本次仅核验原转账，剩余资金步骤需按当前库存重新规划并授权");
                    } else {
                        run.status = OnchainReplenishmentRunStatus::ReadyForNextTransfer;
                        run.next_action = format!(
                            "{destination}；正在重新核验并自动提交第 {} 条已授权资金腿",
                            leg_index + 2
                        );
                    }
                } else {
                    run.status = OnchainReplenishmentRunStatus::Completed;
                    run.next_action = destination.to_owned();
                }
                run.problem = None;
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn record_destination_fee_review(
        &self,
        run_id: &str,
        evidence: &exchange::DepositStatusEvidence,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        let fee = evidence
            .deposit_fee
            .filter(|fee| *fee > Decimal::ZERO)
            .ok_or_else(|| "deposit fee review requires a positive reported fee".to_owned())?;
        if evidence.amount < Decimal::ZERO {
            return Err("reported deposit amount cannot be negative".to_owned());
        }
        self.update_run(
            run_id,
            |run| {
                if run.status != OnchainReplenishmentRunStatus::AwaitingDestinationCredit
                    || active_leg_direction(run)?
                        != shared_types::OnchainTransferDirection::DepositToCex
                {
                    return Err("replenishment run is not awaiting a CEX deposit".to_owned());
                }
                let problem = format!(
                    "{} 充值记录金额 {}，另列充值费 {}；官方金额是否已扣费尚未明确，不能确认净到账数量",
                    evidence.venue,
                    evidence.amount.normalize(),
                    fee.normalize(),
                );
                let transfer = active_transfer_mut(run)?;
                transfer.reported_deposit_amount_exact =
                    Some(evidence.amount.normalize().to_string());
                transfer.status = OnchainReplenishmentTransferStatus::Paused;
                transfer.deposit_fee_exact = Some(fee.normalize().to_string());
                transfer.credited_amount_exact = None;
                transfer.withdrawal_unlocked = None;
                transfer.confirmations = evidence.confirmations;
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.evidence_source = Some(evidence.source_url.clone());
                transfer.problem = Some(problem.clone());
                run.status = OnchainReplenishmentRunStatus::Paused;
                run.next_action =
                    "已保留充值金额和费用；核实净到账后重新规划，不重复转账".to_owned();
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn pause_before_next_transfer(
        &self,
        run_id: &str,
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                if run.status != OnchainReplenishmentRunStatus::ReadyForNextTransfer {
                    return Err("replenishment run is not awaiting the next transfer".to_owned());
                }
                run.status = OnchainReplenishmentRunStatus::Paused;
                run.next_action =
                    "上一条资金腿已到账；下一条因范围或证据变化暂停，重新生成计划后恢复".to_owned();
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn record_destination_failure(
        &self,
        run_id: &str,
        confirmations: Option<u64>,
        evidence_source: String,
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                let transfer = active_transfer_mut(run)?;
                transfer.status = OnchainReplenishmentTransferStatus::Failed;
                transfer.confirmations = confirmations;
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.evidence_source = Some(evidence_source);
                transfer.problem = Some(problem.clone());
                run.status = OnchainReplenishmentRunStatus::Failed;
                run.next_action =
                    "交易所充值已终态失败；核对链上交易与交易所记录后重新规划".to_owned();
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn record_destination_check_problem(
        &self,
        run_id: &str,
        confirmations: Option<u64>,
        evidence_source: String,
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        self.update_run(
            run_id,
            |run| {
                let transfer = active_transfer_mut(run)?;
                transfer.confirmations = confirmations;
                transfer.last_checked_at_ms = Some(now_ms);
                transfer.evidence_source = Some(evidence_source);
                transfer.problem = Some(problem.clone());
                run.next_action = match active_leg_direction(run)? {
                    shared_types::OnchainTransferDirection::WithdrawToChain => {
                        "继续核验交易哈希、目标地址、代币身份与精确到账数量".to_owned()
                    }
                    shared_types::OnchainTransferDirection::DepositToCex => {
                        "继续按交易哈希核验交易所官方充值历史".to_owned()
                    }
                };
                run.problem = Some(problem);
                Ok(())
            },
            now_ms,
        )
    }

    pub(crate) fn readiness(&self) -> Result<(), String> {
        self.recovery_readiness()?;
        if self.path.is_none() {
            return Err("链上补仓恢复日志未配置，已阻止真实资金提交".to_owned());
        }
        self.persistence_problem
            .lock()
            .as_ref()
            .map_or(Ok(()), |problem| {
                Err(format!("链上补仓恢复日志不可写：{problem}"))
            })
    }

    pub(crate) fn request_recheck(
        &self,
        request: &shared_types::OnchainReplenishmentRecheckRequest,
        actor: &str,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let mut run = self.runs.get(&request.run_id).map(|r| r.value().clone()).ok_or("补库记录不存在")?;
        if run.authorization.actor != actor { return Err("只能核验当前操作人授权的补库记录".into()); }
        let transfer = run.transfers.last().ok_or("没有已提交的转账可核验")?;
        if transfer.client_transfer_id != request.expected_client_transfer_id { return Err("资金步骤已变化，请重新读取记录".into()); }
        if run.read_only_recovery && matches!(run.status, OnchainReplenishmentRunStatus::AwaitingSourceFinality
            | OnchainReplenishmentRunStatus::AwaitingDestinationCredit | OnchainReplenishmentRunStatus::Completed) {
            return Ok(run);
        }
        if run.recheck_request().is_none() { return Err("当前记录不支持重新核验；不能重发转账或跳过未确认步骤".into()); }
        let source_complete = matches!(transfer.status, OnchainReplenishmentTransferStatus::SourceCompleted
            | OnchainReplenishmentTransferStatus::DestinationCredited);
        let transfer = run.transfers.last_mut().unwrap();
        if !source_complete { transfer.status = OnchainReplenishmentTransferStatus::Submitted; }
        transfer.last_checked_at_ms = None;
        // Original amount, IDs, submission time, receipts and errors remain audit evidence.
        run.read_only_recovery = true;
        run.recovery_checks = 0;
        run.status = if source_complete { OnchainReplenishmentRunStatus::AwaitingDestinationCredit } else { OnchainReplenishmentRunStatus::AwaitingSourceFinality };
        run.updated_at_ms = now_ms;
        run.next_action = "正在重新核验原转账，最多 12 次；不重新转账，不自动执行下一步".into();
        self.persist_run_unlocked(&run)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(run)
    }

    pub(crate) fn claim_recovery_check(&self, run_id: &str, now_ms: i64) -> Result<Option<OnchainReplenishmentRun>, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let mut run = self.runs.get(run_id).map(|r| r.value().clone()).ok_or("补库记录不存在")?;
        if !run.read_only_recovery || !matches!(run.status, OnchainReplenishmentRunStatus::AwaitingSourceFinality | OnchainReplenishmentRunStatus::AwaitingDestinationCredit) {
            return Ok(None);
        }
        if run.transfers.last().and_then(|t| t.last_checked_at_ms).is_some_and(|ms| now_ms.saturating_sub(ms) < 60_000) { return Ok(None); }
        if run.recovery_checks >= shared_types::ONCHAIN_REPLENISHMENT_RECOVERY_LIMIT {
            run.status = OnchainReplenishmentRunStatus::Paused;
            let transfer = run.transfers.last_mut().ok_or("资金步骤不存在")?;
            if !matches!(transfer.status, OnchainReplenishmentTransferStatus::SourceCompleted
                | OnchainReplenishmentTransferStatus::DestinationCredited) {
                transfer.status = OnchainReplenishmentTransferStatus::Paused;
            }
            run.next_action = "原转账核验次数已用完，请核对交易所或链上记录后再刷新；不会重发转账".into();
            run.problem = Some("12 次只读核验仍未完成到账确认".into());
        } else {
            run.recovery_checks += 1;
            run.transfers.last_mut().ok_or("资金步骤不存在")?.last_checked_at_ms = Some(now_ms);
        }
        run.updated_at_ms = now_ms;
        self.persist_run_unlocked(&run)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(Some(run))
    }

    fn append_unlocked(&self, entry: &LogEntry) -> Result<(), String> {
        self.recovery_readiness()?;
        if let Some(problem) = self.persistence_problem.lock().as_ref() {
            return Err(format!(
                "补仓日志写入曾失败，请核对日志并重启后恢复：{problem}"
            ));
        }
        use super::onchain_wallet_claims::{Module, Owner};
        if let Some(run) = &entry.run {
            let hold = match wallet_claims::hold(run) {
                Ok(hold) => hold,
                Err(problem) if self.is_read_only_wallet_update(run) => {
                    return self.wallet_claims.persist_unresolved(
                        Module::Replenishment,
                        problem,
                        || self.write_entry(entry),
                    );
                }
                Err(problem) => return Err(problem),
            };
            return self.wallet_claims.commit(
                Owner::new(Module::Replenishment, &run.run_id),
                hold,
                run.updated_at_ms,
                || self.write_entry(entry),
            );
        }
        self.wallet_claims
            .persist_unclaimed(Module::Replenishment, || self.write_entry(entry))
    }

    fn write_entry(&self, entry: &LogEntry) -> Result<(), String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        match append_jsonl(path, entry) {
            Ok(()) => {
                *self.persistence_problem.lock() = None;
                Ok(())
            }
            Err(error) => {
                let problem = error.to_string();
                *self.persistence_problem.lock() = Some(problem.clone());
                tracing::warn!(path = %path.display(), %error, "failed to persist on-chain replenishment state");
                Err(problem)
            }
        }
    }

    pub(crate) fn record_network_cost_valuation(
        &self,
        run_id: &str,
        leg_index: u32,
        value: OnchainReplenishmentCostValuation,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let mut run = self
            .runs
            .get(run_id)
            .map(|entry| entry.value().clone())
            .ok_or("美元折算运行记录不存在")?;
        let transfer = run
            .transfers
            .get_mut(leg_index as usize)
            .filter(|transfer| transfer.leg_index == leg_index)
            .ok_or("美元折算缺少对应资金步骤")?;
        if value.valued_at_ms < transfer.submission_attempted_at_ms || value.valued_at_ms > now_ms {
            return Err("美元折算时间与原资金步骤不一致".into());
        }
        let cost = transfer
            .network_cost
            .as_mut()
            .ok_or("美元折算缺少原交易网络费")?;
        if let Some(old) = &cost.usd_valuation {
            return if old == &value {
                Ok(run)
            } else {
                Err("已保存的网络费美元折算不能随行情重新改写".into())
            };
        }
        cost.usd_valuation = Some(value);
        super::onchain_comparison::replenishment_costs::network_fee_usd(cost)?;
        run.updated_at_ms = now_ms.max(run.updated_at_ms);
        self.persist_run_unlocked(&run)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(run)
    }

    pub(crate) fn record_withdrawal_cost_valuation(
        &self,
        run_id: &str,
        leg_index: u32,
        value: OnchainReplenishmentCostValuation,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let mut run = self.runs.get(run_id).map(|entry| entry.value().clone())
            .ok_or("提币费美元折算运行记录不存在")?;
        let leg = run.plan.legs.get_mut(leg_index as usize)
            .filter(|leg| leg.direction == shared_types::OnchainTransferDirection::WithdrawToChain)
            .ok_or("美元折算缺少原提币资金腿")?;
        let transfer = run.transfers.get_mut(leg_index as usize)
            .filter(|transfer| transfer.leg_index == leg_index)
            .ok_or("美元折算缺少对应提币步骤")?;
        if value.valued_at_ms < transfer.submission_attempted_at_ms || value.valued_at_ms > now_ms {
            return Err("美元折算时间与原资金步骤不一致".into());
        }
        let cost = transfer.withdrawal_cost.as_mut().ok_or("美元折算缺少已确认提币费")?;
        if !cost.asset.eq_ignore_ascii_case(&leg.asset) {
            return Err("提币费币种与原资金腿不一致".into());
        }
        if let Some(old) = &cost.usd_valuation {
            return if old == &value { Ok(run) } else { Err("已保存的提币费美元折算不能随行情重新改写".into()) };
        }
        cost.usd_valuation = Some(value);
        // Withdrawal gas is paid by the venue and already covered by its fee.
        leg.economics.reconciled_cost_usd = Some(super::onchain_comparison::replenishment_costs::withdrawal_fee_usd(cost)?);
        run.updated_at_ms = now_ms.max(run.updated_at_ms);
        self.persist_run_unlocked(&run)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(run)
    }

    fn update_run(
        &self,
        run_id: &str,
        update: impl FnOnce(&mut OnchainReplenishmentRun) -> Result<(), String>,
        now_ms: i64,
    ) -> Result<OnchainReplenishmentRun, String> {
        let _guard = self.ledger_lock.lock();
        let mut run = self
            .runs
            .get(run_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| "replenishment run is missing".to_owned())?;
        update(&mut run)?;
        run.updated_at_ms = now_ms;
        self.persist_run_unlocked(&run)?;
        self.runs.insert(run.run_id.clone(), run.clone());
        Ok(run)
    }

    fn persist_run_unlocked(&self, run: &OnchainReplenishmentRun) -> Result<(), String> {
        self.append_unlocked(&LogEntry {
            schema_version: SCHEMA_VERSION,
            plan: None,
            run: Some(run.clone()),
        })
    }

    fn prune_plans(&self, now_ms: i64) {
        self.plans
            .retain(|_, response| response.valid_until_ms >= now_ms);
    }

    fn recovery_readiness(&self) -> Result<(), String> {
        self.recovery_problem
            .as_ref()
            .map_or(Ok(()), |problem| Err(problem.clone()))
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
            // Retain the key even when its completed snapshot leaves memory.
        }
    }

    fn remove_oldest_plan(&self) {
        let oldest = self
            .plans
            .iter()
            .min_by_key(|entry| entry.built_at_ms)
            .map(|entry| entry.key().clone());
        if let Some(plan_id) = oldest {
            self.plans.remove(&plan_id);
        }
    }
}

#[derive(Default)]
struct ReplayState {
    plans: Vec<OnchainReplenishmentPlanResponse>,
    runs: Vec<OnchainReplenishmentRun>,
    problem: Option<String>,
}

fn settled(run: &OnchainReplenishmentRun, now_ms: i64) -> bool {
    matches!(
        run.status,
        OnchainReplenishmentRunStatus::Completed
            | OnchainReplenishmentRunStatus::AuthorizationExpired
    ) || run.status == OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
        && run.authorization.valid_until_ms < now_ms
}

fn run_priority(run: &OnchainReplenishmentRun, now_ms: i64) -> u8 {
    if settled(run, now_ms) {
        return 3;
    }
    match run.status {
        OnchainReplenishmentRunStatus::Submitting
        | OnchainReplenishmentRunStatus::AwaitingSourceFinality
        | OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        | OnchainReplenishmentRunStatus::ReadyForNextTransfer => 0,
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit => 1,
        _ => 2,
    }
}

fn replay(path: Option<&Path>, now_ms: i64) -> ReplayState {
    let Some(path) = path else {
        return ReplayState::default();
    };
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ReplayState::default();
        }
        Err(error) => {
            return ReplayState {
                problem: Some(format!("补仓恢复日志无法读取，资金动作已停用：{error}")),
                ..Default::default()
            };
        }
    };
    let mut plans = std::collections::BTreeMap::new();
    let mut runs = std::collections::BTreeMap::new();
    let mut reader = BufReader::new(file);
    let mut problem = None;
    let mut line_number = 0;
    loop {
        let mut line = String::new();
        line_number += 1;
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                problem = Some(format!(
                    "补仓恢复日志第 {line_number} 行无法读取，资金动作已停用：{error}"
                ));
                break;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        let entry = match decode_entry(&line) {
            Ok(entry) => entry,
            Err(reason) => {
                problem = Some(format!(
                    "补仓恢复日志第 {line_number} 行{reason}，资金动作已停用；请保留原日志并核对备份后重启"
                ));
                break;
            }
        };
        if let Some(plan) = entry.plan.filter(|plan| plan.valid_until_ms >= now_ms) {
            plans.insert(plan.plan_id.clone(), plan);
        }
        if let Some(run) = entry.run {
            runs.insert(run.run_id.clone(), run);
        }
    }
    ReplayState {
        plans: plans.into_values().collect(),
        runs: runs.into_values().collect(),
        problem,
    }
}

fn project_expiry(mut run: OnchainReplenishmentRun, now_ms: i64) -> OnchainReplenishmentRun {
    if run.status == OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
        && run.authorization.valid_until_ms < now_ms
    {
        run.status = OnchainReplenishmentRunStatus::AuthorizationExpired;
        run.updated_at_ms = run.authorization.valid_until_ms;
        run.next_action = "授权已过期；重新生成计划并再次明确授权".to_owned();
        run.problem = Some("未在 60 秒授权窗口内提交资金动作".to_owned());
    }
    run
}

fn run_id(idempotency_key: &str) -> String {
    let digest = common::signing::hmac_sha256_hex(RUN_ID_KEY, idempotency_key.as_bytes());
    format!("onchain-replenishment-run-{}", &digest[..24])
}

fn transfer_id(run_id: &str, leg_index: u32) -> String {
    let canonical = format!("{run_id}:{leg_index}");
    let digest = common::signing::hmac_sha256_hex(TRANSFER_ID_KEY, canonical.as_bytes());
    format!("crossline-{}", &digest[..24])
}

fn active_transfer_mut(
    run: &mut OnchainReplenishmentRun,
) -> Result<&mut OnchainReplenishmentTransferProgress, String> {
    run.transfers
        .last_mut()
        .ok_or_else(|| "replenishment transfer claim is missing".to_owned())
}

fn validate_network_cost(
    run: &OnchainReplenishmentRun,
    cost: &OnchainReplenishmentNetworkCost,
) -> Result<(), String> {
    let transfer = run.transfers.last().ok_or("链上费用缺少对应转账")?;
    let leg = run
        .plan
        .legs
        .get(transfer.leg_index as usize)
        .ok_or("链上费用缺少对应步骤")?;
    let same_address = |left: &str, right: &str| {
        if leg.chain == "solana" {
            left == right
        } else {
            left.eq_ignore_ascii_case(right)
        }
    };
    if !matches!(
        run.status,
        OnchainReplenishmentRunStatus::Submitting
            | OnchainReplenishmentRunStatus::AwaitingSourceFinality
            | OnchainReplenishmentRunStatus::AwaitingDestinationCredit
    ) || leg.direction != shared_types::OnchainTransferDirection::DepositToCex
        || leg.chain != cost.chain
        || shared_types::onchain_chain_preset(&cost.chain)
            .is_none_or(|preset| preset.base_token != cost.asset)
        || leg
            .source_address
            .as_deref()
            .is_none_or(|address| !same_address(address, &cost.payer))
        || transfer
            .transaction_id
            .as_deref()
            .is_none_or(|hash| !same_address(hash, &cost.transaction_id))
        || cost.block_ref.is_empty()
        || cost.source.is_empty()
        || cost.observed_at_ms < transfer.submission_attempted_at_ms
        || transfer
            .last_checked_at_ms
            .is_some_and(|time| cost.observed_at_ms < time)
    {
        return Err("链上费用回执与原交易、付款钱包、链或时间不一致，未覆盖记录".into());
    }
    let parse = |value: &Option<String>| -> Result<Option<Decimal>, String> {
        value
            .as_deref()
            .map(|value| {
                value
                    .parse::<Decimal>()
                    .ok()
                    .filter(|value| *value >= Decimal::ZERO)
                    .ok_or_else(|| "链上费用不是有效非负精确数值".into())
            })
            .transpose()
    };
    let execution = parse(&cost.execution_fee_exact)?;
    let additional = parse(&cost.additional_fee_exact)?;
    let total = parse(&cost.total_fee_exact)?;
    if total.is_some()
        && execution
            .zip(additional)
            .and_then(|(a, b)| a.checked_add(b))
            != total
    {
        return Err("链上费用分项与总额不一致，未覆盖记录".into());
    }
    if let Some(old) = &transfer.network_cost {
        if old.usd_valuation.is_some() && old.usd_valuation != cost.usd_valuation {
            return Err("已记录的网络费美元折算发生回退，未覆盖原记录".into());
        }
        if old.block_ref != cost.block_ref
            || old.observed_at_ms > cost.observed_at_ms
            || [
                (&old.execution_fee_exact, &cost.execution_fee_exact),
                (&old.additional_fee_exact, &cost.additional_fee_exact),
                (&old.total_fee_exact, &cost.total_fee_exact),
            ]
            .iter()
            .any(|(old, new)| old.is_some() && old != new)
        {
            return Err("已记录的链上实扣费用发生变化或回退，需核对原交易".into());
        }
    }
    if cost.usd_valuation.is_some() {
        super::onchain_comparison::replenishment_costs::network_fee_usd(cost)?;
    }
    Ok(())
}

fn validate_withdrawal_cost(
    run: &OnchainReplenishmentRun,
    evidence: &exchange::WithdrawalStatusEvidence,
) -> Result<(), String> {
    let transfer = run.transfers.last().ok_or("提币缺少提交记录")?;
    let leg = run
        .plan
        .legs
        .get(transfer.leg_index as usize)
        .ok_or("提币步骤不存在")?;
    if !matches!(
        run.status,
        OnchainReplenishmentRunStatus::Submitting
            | OnchainReplenishmentRunStatus::AwaitingSourceFinality
            | OnchainReplenishmentRunStatus::AwaitingDestinationCredit
    ) || leg.direction != shared_types::OnchainTransferDirection::WithdrawToChain
        || !shared_types::venue_names_equal(&leg.venue, &evidence.venue)
        || !leg.asset.eq_ignore_ascii_case(&evidence.currency)
        || leg.network_evidence.network.as_deref() != Some(evidence.network.as_str())
        || leg.destination.address.as_deref() != Some(evidence.address.as_str())
        || transfer.client_transfer_id != evidence.client_withdrawal_id
        || evidence.amount < Decimal::ZERO
        || evidence.transaction_fee < Decimal::ZERO
        || evidence.provider_withdrawal_id.trim().is_empty()
        || evidence.source_url.trim().is_empty()
        || evidence.checked_at_ms < transfer.submission_attempted_at_ms
        || transfer
            .last_checked_at_ms
            .is_some_and(|time| evidence.checked_at_ms < time)
        || transfer
            .withdrawal_cost
            .as_ref()
            .is_some_and(|cost| evidence.checked_at_ms < cost.observed_at_ms)
        || transfer
            .provider_transfer_id
            .as_ref()
            .is_some_and(|id| id != &evidence.provider_withdrawal_id)
        || transfer
            .transaction_id
            .as_ref()
            .zip(evidence.transaction_id.as_ref())
            .is_some_and(|(old, new)| old != new)
    {
        return Err("提币回执的身份、时间或费用与当前资金步骤不一致，未覆盖原记录".into());
    }
    if let Some(old) = transfer
        .withdrawal_cost
        .as_ref()
        .filter(|cost| cost.confirmed)
    {
        if evidence.status != exchange::WithdrawalStatus::Completed
            || old.fee_exact.parse::<Decimal>().ok() != Some(evidence.transaction_fee)
            || old.reported_amount_exact.parse::<Decimal>().ok() != Some(evidence.amount)
        {
            return Err("已确认提币回执发生回退或费用变化，需核对原记录".into());
        }
    }
    Ok(())
}

fn active_leg_direction(
    run: &OnchainReplenishmentRun,
) -> Result<shared_types::OnchainTransferDirection, String> {
    let transfer = run
        .transfers
        .last()
        .ok_or_else(|| "replenishment transfer claim is missing".to_owned())?;
    run.plan
        .legs
        .get(transfer.leg_index as usize)
        .map(|leg| leg.direction)
        .ok_or_else(|| "replenishment active leg is missing".to_owned())
}

fn storage_problem(path: Option<&Path>) -> Option<String> {
    let path = path?;
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(path)?;
        Ok(())
    })();
    result.err().map(|error| error.to_string())
}

fn decode_entry(line: &str) -> Result<LogEntry, &'static str> {
    if !line.ends_with('\n') {
        return Err("未完整写入");
    }
    let entry = serde_json::from_str::<LogEntry>(line).map_err(|_| "格式损坏")?;
    if entry.schema_version != SCHEMA_VERSION || entry.plan.is_some() == entry.run.is_some() {
        return Err("版本或记录类型不受支持");
    }
    Ok(entry)
}

fn append_jsonl(path: &Path, entry: &LogEntry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut row = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    row.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&row)?;
    file.flush()?;
    file.sync_data()
}

#[cfg(test)]
#[path = "onchain_replenishment_plan_store/recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
mod tests {
    use shared_types::{
        OnchainComparisonDirection, OnchainReplenishmentDestination,
        OnchainReplenishmentDestinationStatus, OnchainReplenishmentLeg,
        OnchainReplenishmentLegEconomics, OnchainReplenishmentNetworkEvidence,
        OnchainReplenishmentPlanStatus, OnchainTransferDirection, OnchainTransferStatus,
    };

    use super::*;

    fn plan(id: &str, built_at_ms: i64, valid_until_ms: i64) -> OnchainReplenishmentPlanResponse {
        OnchainReplenishmentPlanResponse {
            plan_id: id.to_owned(),
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            status: OnchainReplenishmentPlanStatus::ReadyForAuthorization,
            legs: vec![OnchainReplenishmentLeg {
                direction: OnchainTransferDirection::WithdrawToChain,
                venue: "binance".to_owned(),
                asset: "USDC".to_owned(),
                chain: "solana".to_owned(),
                asset_address: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_owned()),
                asset_decimals: Some(6),
                source_address: None,
                transfer_amount: 10.0,
                transfer_amount_exact: Some("10".to_owned()),
                economics: OnchainReplenishmentLegEconomics {
                    estimated_cost_usd: Some(0.1),
                    estimated_network_cost_usd: None,
                    reconciled_cost_usd: None,
                    fee_amount: Some(0.1),
                    fee_amount_exact: Some("0.1".to_owned()),
                    source_debit_upper_bound: Some(10.1),
                    source_debit_upper_bound_exact: Some("10.1".to_owned()),
                },
                network_evidence: OnchainReplenishmentNetworkEvidence {
                    network: Some("SOL".to_owned()),
                    minimum_amount: Some(1.0),
                    minimum_amount_exact: Some("1".to_owned()),
                    amount_step: Some("0.01".to_owned()),
                    credit_confirmations: Some(1),
                    unlock_confirmations: Some(2),
                    transfer_status: OnchainTransferStatus::Ready,
                    evidence_source: Some("official".to_owned()),
                    evidence_observed_at_ms: Some(10),
                },
                destination: OnchainReplenishmentDestination {
                    address: Some("wallet-1".to_owned()),
                    tag: None,
                    status: OnchainReplenishmentDestinationStatus::Verified,
                    source: Some("official".to_owned()),
                    observed_at_ms: Some(10),
                    problem: None,
                },
                blocker: None,
            }],
            transfer_cost_usd: Some(1.0),
            post_transfer_net_profit_usd: Some(2.0),
            built_at_ms,
            valid_until_ms,
            requires_live_authorization: true,
            submit_ready: true,
            blockers: Vec::new(),
        }
    }

    pub(super) fn deposit_plan(
        id: &str,
        built_at_ms: i64,
        valid_until_ms: i64,
    ) -> OnchainReplenishmentPlanResponse {
        let mut plan = plan(id, built_at_ms, valid_until_ms);
        plan.direction = OnchainComparisonDirection::BuyCexSellOnchain;
        let leg = &mut plan.legs[0];
        leg.direction = OnchainTransferDirection::DepositToCex;
        leg.source_address = Some("wallet-source".to_owned());
        leg.destination.address = Some("binance-deposit-address".to_owned());
        plan
    }

    fn two_leg_plan(
        id: &str,
        built_at_ms: i64,
        valid_until_ms: i64,
    ) -> OnchainReplenishmentPlanResponse {
        let mut plan = plan(id, built_at_ms, valid_until_ms);
        let mut second = plan.legs[0].clone();
        second.venue = "bitget".to_owned();
        second.destination.address = Some("wallet-2".to_owned());
        plan.legs.push(second);
        plan
    }

    #[test]
    fn authorization_is_idempotent_and_expires_after_restart() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("replenishment.jsonl");
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
        store
            .insert(plan("fresh", 10, 100), 10)
            .expect("plan persistence");
        let first = store
            .authorize("fresh", "idem-1", "operator", 20)
            .expect("authorization");
        let replayed = store
            .authorize("fresh", "idem-1", "operator", 21)
            .expect("authorization replay");
        assert!(!first.replayed);
        assert!(replayed.replayed);
        assert_eq!(first.run.run_id, replayed.run.run_id);

        let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 90_000);
        let runs = restored.runs(20, 90_000);
        assert_eq!(runs.rows.len(), 1);
        assert_eq!(
            runs.rows[0].status,
            OnchainReplenishmentRunStatus::AuthorizationExpired
        );
    }

    #[test]
    fn authorization_rejects_a_plan_that_is_not_submit_ready() {
        let store = OnchainReplenishmentPlanStore::load_path(None, 10);
        let mut response = plan("locked", 10, 100);
        response.submit_ready = false;
        store.insert(response, 10).expect("plan persistence");

        let result = store.authorize("locked", "idem-locked", "operator", 20);

        assert!(matches!(
            result,
            Err(ReplenishmentAuthorizeError::NotReady(_))
        ));
        assert!(store.runs(10, 20).rows.is_empty());
    }

    #[test]
    fn submit_claim_survives_restart_without_duplicate_transfer_identity() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("replenishment.jsonl");
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
        let response = plan("fresh", 10, 100);
        store
            .insert(response.clone(), 10)
            .expect("plan persistence");
        let authorized = store
            .authorize("fresh", "idem-submit", "operator", 20)
            .expect("authorization");
        let first = store
            .claim_submission(&authorized.run.run_id, "operator", response.clone(), 0, &store.submission_snapshot(), 30)
            .expect("submit claim");
        assert!(!first.replayed);
        assert_eq!(first.run.transfers.len(), 1);

        let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 40);
        let replay = restored
            .claim_submission(&authorized.run.run_id, "operator", response, 0, &restored.submission_snapshot(), 40)
            .expect("submit replay");

        assert!(replay.replayed);
        assert_eq!(replay.run.status, OnchainReplenishmentRunStatus::Submitting);
        assert_eq!(replay.run.transfers.len(), 1);
        assert_eq!(
            replay.run.transfers[0].client_transfer_id,
            first.run.transfers[0].client_transfer_id
        );
    }

    #[test]
    fn replenishment_locked_credit_survives_restart_without_advancing_next_leg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replenishment.jsonl");
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
        let mut response = deposit_plan("locked", 10, 200_000);
        response
            .legs
            .push(plan("second", 10, 200_000).legs.remove(0));
        store.insert(response.clone(), 10).unwrap();
        let run_id = store
            .authorize("locked", "locked-idem", "operator", 20)
            .unwrap()
            .run
            .run_id;
        store
            .claim_submission(&run_id, "operator", response.clone(), 0, &store.submission_snapshot(), 30)
            .unwrap();
        store
            .record_chain_submission_intent(&run_id, "tx-1".to_owned(), "rpc".to_owned(), 31)
            .unwrap();
        store
            .record_source_status(
                &run_id,
                OnchainReplenishmentTransferStatus::SourceCompleted,
                "tx-1".to_owned(),
                Some("tx-1".to_owned()),
                Some(1),
                "rpc".to_owned(),
                None,
                40,
            )
            .unwrap();
        let locked = store
            .record_destination_credit(
                &run_id,
                Decimal::TEN,
                Some(false),
                Some(12),
                "binance-history".to_owned(),
                50,
            )
            .unwrap();
        assert_eq!(
            locked.status,
            OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        );
        assert_eq!(
            locked.transfers[0].credited_amount_exact.as_deref(),
            Some("10")
        );
        assert_eq!(locked.transfers[0].withdrawal_unlocked, Some(false));
        assert_eq!(locked.transfers.len(), 1);
        drop(store);

        let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 60);
        let run = restored.run(&run_id, 60).unwrap();
        assert_eq!(run.transfers[0].withdrawal_unlocked, Some(false));
        let replay = restored
            .claim_submission(&run_id, "operator", response, 1, &restored.submission_snapshot(), 60)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.run.transfers.len(), 1);
        assert_eq!(
            replay.run.status,
            OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        );
        let unlocked = restored
            .record_destination_credit(
                &run_id,
                Decimal::TEN,
                Some(true),
                Some(20),
                "binance-history".to_owned(),
                70,
            )
            .unwrap();
        assert_eq!(
            unlocked.status,
            OnchainReplenishmentRunStatus::ReadyForNextTransfer
        );
        assert_eq!(unlocked.transfers[0].withdrawal_unlocked, Some(true));
        assert_eq!(unlocked.transfers.len(), 1);
        assert!(restored
            .record_destination_credit(
                &run_id,
                Decimal::TEN,
                Some(false),
                Some(12),
                "stale-history".to_owned(),
                80
            )
            .is_err());
        assert_eq!(
            restored.run(&run_id, 80).unwrap().transfers[0].withdrawal_unlocked,
            Some(true)
        );
    }

    #[test]
    fn replenishment_credit_review_retains_transaction_after_restart_without_resubmission() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replenishment.jsonl");
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
        let response = deposit_plan("credit-review", 10, 200_000);
        store.insert(response.clone(), 10).unwrap();
        let run_id = store
            .authorize("credit-review", "review-idem", "operator", 20)
            .unwrap()
            .run
            .run_id;
        store
            .claim_submission(&run_id, "operator", response.clone(), 0, &store.submission_snapshot(), 30)
            .unwrap();
        let problem = "交易已确认，但原生 SOL 未到账，仅收到 WSOL";
        let paused = store
            .record_source_status(
                &run_id,
                OnchainReplenishmentTransferStatus::Paused,
                "tx-review".to_owned(),
                Some("tx-review".to_owned()),
                None,
                "rpc".to_owned(),
                Some(problem.to_owned()),
                40,
            )
            .unwrap();
        assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
        assert!(paused.next_action.contains("不要重复转账"));
        drop(store);
        let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 50);
        let replay = restored
            .claim_submission(&run_id, "operator", response, 0, &restored.submission_snapshot(), 60)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.run.status, OnchainReplenishmentRunStatus::Paused);
        assert_eq!(replay.run.transfers.len(), 1);
        let transfer = &replay.run.transfers[0];
        assert_eq!(transfer.status, OnchainReplenishmentTransferStatus::Paused);
        assert_eq!(transfer.transaction_id.as_deref(), Some("tx-review"));
        assert_eq!(transfer.evidence_source.as_deref(), Some("rpc"));
        assert_eq!(transfer.problem.as_deref(), Some(problem));
        assert_eq!(replay.run.problem.as_deref(), Some(problem));
    }

    #[test]
    fn replenishment_credit_shortfall_is_recorded_but_cannot_unlock_next_leg() {
        let store = OnchainReplenishmentPlanStore::load_path(None, 10);
        let response = deposit_plan("short", 10, 100);
        store.insert(response.clone(), 10).unwrap();
        let run_id = store
            .authorize("short", "short-idem", "operator", 20)
            .unwrap()
            .run
            .run_id;
        store
            .claim_submission(&run_id, "operator", response, 0, &store.submission_snapshot(), 30)
            .unwrap();
        assert!(store
            .record_destination_credit(
                &run_id,
                Decimal::TEN,
                Some(true),
                None,
                "history".to_owned(),
                31
            )
            .is_err());
        store
            .record_source_status(
                &run_id,
                OnchainReplenishmentTransferStatus::SourceCompleted,
                "tx-1".to_owned(),
                Some("tx-1".to_owned()),
                Some(1),
                "rpc".to_owned(),
                None,
                40,
            )
            .unwrap();
        let paused = store
            .record_destination_credit(
                &run_id,
                Decimal::new(99, 1),
                Some(true),
                Some(20),
                "history".to_owned(),
                50,
            )
            .unwrap();
        assert_eq!(paused.status, OnchainReplenishmentRunStatus::Paused);
        assert_eq!(
            paused.transfers[0].credited_amount_exact.as_deref(),
            Some("9.9")
        );
        assert!(paused.problem.unwrap().contains("低于计划"));
        assert!(store
            .record_destination_credit(
                &run_id,
                Decimal::TEN,
                Some(true),
                Some(20),
                "history".to_owned(),
                60
            )
            .is_err());
    }

    #[test]
    fn replenishment_legacy_credit_evidence_remains_unknown() {
        let row: OnchainReplenishmentTransferProgress = serde_json::from_value(serde_json::json!({
            "legIndex": 0, "clientTransferId": "old-transfer", "providerTransferId": "old-provider",
            "status": "destination_credited", "submissionAttemptedAtMs": 10, "lastCheckedAtMs": 20,
            "transactionId": "old-hash", "confirmations": 12, "evidenceSource": "history", "problem": null
        })).unwrap();
        assert_eq!(row.credited_amount_exact, None);
        assert_eq!(row.withdrawal_unlocked, None);
    }

    #[test]
    fn destination_credit_is_durable_only_after_exact_chain_evidence() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("replenishment.jsonl");
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
        let response = plan("credit", 10, 100);
        store
            .insert(response.clone(), 10)
            .expect("plan persistence");
        let authorized = store
            .authorize("credit", "idem-credit", "operator", 20)
            .expect("authorization");
        store
            .claim_submission(&authorized.run.run_id, "operator", response, 0, &store.submission_snapshot(), 30)
            .expect("submit claim");
        store
            .record_submission_ack(
                &authorized.run.run_id,
                "provider-1".to_owned(),
                None,
                31,
                "official-withdrawal-history".to_owned(),
                31,
            )
            .expect("submission ack");
        store
            .record_source_status(
                &authorized.run.run_id,
                OnchainReplenishmentTransferStatus::SourceCompleted,
                "provider-1".to_owned(),
                Some("transaction-1".to_owned()),
                Some(1),
                "official-withdrawal-history".to_owned(),
                None,
                40,
            )
            .expect("source completion");
        store
            .record_destination_credit(
                &authorized.run.run_id,
                Decimal::TEN,
                None,
                Some(2),
                "official-chain-rpc".to_owned(),
                50,
            )
            .expect("destination credit");

        let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 60);
        let run = restored
            .run(&authorized.run.run_id, 60)
            .expect("restored run");
        assert_eq!(run.status, OnchainReplenishmentRunStatus::Completed);
        assert_eq!(
            run.transfers[0].status,
            OnchainReplenishmentTransferStatus::DestinationCredited
        );
        assert_eq!(run.transfers[0].confirmations, Some(2));
    }

    #[test]
    fn verified_first_credit_unlocks_exactly_one_next_leg_after_authorization_window() {
        let store = OnchainReplenishmentPlanStore::load_path(None, 10);
        let response = two_leg_plan("multi", 10, 200_000);
        store
            .insert(response.clone(), 10)
            .expect("plan persistence");
        let authorized = store
            .authorize("multi", "idem-multi", "operator", 20)
            .expect("authorization");
        store
            .claim_submission(&authorized.run.run_id, "operator", response.clone(), 0, &store.submission_snapshot(), 30)
            .expect("first claim");
        store
            .record_submission_ack(
                &authorized.run.run_id,
                "provider-1".to_owned(),
                None,
                31,
                "official-withdrawal-history".to_owned(),
                31,
            )
            .expect("first ack");
        store
            .record_source_status(
                &authorized.run.run_id,
                OnchainReplenishmentTransferStatus::SourceCompleted,
                "provider-1".to_owned(),
                Some("transaction-1".to_owned()),
                Some(1),
                "official-withdrawal-history".to_owned(),
                None,
                40,
            )
            .expect("first source completion");
        let ready = store
            .record_destination_credit(
                &authorized.run.run_id,
                Decimal::TEN,
                None,
                Some(2),
                "official-chain-rpc".to_owned(),
                70_000,
            )
            .expect("first destination credit");
        assert_eq!(
            ready.status,
            OnchainReplenishmentRunStatus::ReadyForNextTransfer
        );

        let second = store
            .claim_submission(&authorized.run.run_id, "operator", response, 1, &store.submission_snapshot(), 70_001)
            .expect("second claim after initial authorization window");
        assert!(!second.replayed);
        assert_eq!(second.run.transfers.len(), 2);
        assert_eq!(second.run.transfers[1].leg_index, 1);
    }

    #[test]
    fn chain_deposit_restarts_without_duplicate_broadcast() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("replenishment.jsonl");
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
        let response = deposit_plan("deposit", 10, 100);
        store
            .insert(response.clone(), 10)
            .expect("plan persistence");
        let authorized = store
            .authorize("deposit", "idem-deposit", "operator", 20)
            .expect("authorization");
        store
            .claim_submission(&authorized.run.run_id, "operator", response, 0, &store.submission_snapshot(), 30)
            .expect("submit claim");
        store
            .record_chain_submission_intent(
                &authorized.run.run_id,
                "chain-transaction-1".to_owned(),
                "official-chain-rpc".to_owned(),
                31,
            )
            .expect("durable chain intent");
        store
            .record_source_status(
                &authorized.run.run_id,
                OnchainReplenishmentTransferStatus::SourceCompleted,
                "chain-transaction-1".to_owned(),
                Some("chain-transaction-1".to_owned()),
                Some(2),
                "official-chain-rpc".to_owned(),
                None,
                40,
            )
            .expect("chain finality");
        store
            .record_destination_credit(
                &authorized.run.run_id,
                Decimal::TEN,
                Some(true),
                Some(2),
                "official-deposit-history".to_owned(),
                50,
            )
            .expect("exchange credit");

        let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 60);
        let run = restored
            .run(&authorized.run.run_id, 60)
            .expect("restored run");
        let replay = restored
            .claim_submission(&run.run_id, "operator", run.plan.clone(), 0, &restored.submission_snapshot(), 60)
            .expect("completed replay");

        assert!(replay.replayed);
        assert_eq!(replay.run.status, OnchainReplenishmentRunStatus::Completed);
        assert_eq!(replay.run.transfers.len(), 1);
        assert_eq!(
            replay.run.transfers[0].transaction_id.as_deref(),
            Some("chain-transaction-1")
        );
    }

    #[test]
    fn exchange_deposit_failure_is_durable_and_terminal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("replenishment.jsonl");
        let store = OnchainReplenishmentPlanStore::load_path(Some(path.clone()), 10);
        let response = deposit_plan("failed-deposit", 10, 100);
        store
            .insert(response.clone(), 10)
            .expect("plan persistence");
        let authorized = store
            .authorize("failed-deposit", "idem-failed", "operator", 20)
            .expect("authorization");
        store
            .claim_submission(&authorized.run.run_id, "operator", response, 0, &store.submission_snapshot(), 30)
            .expect("submit claim");
        store
            .record_destination_failure(
                &authorized.run.run_id,
                Some(4),
                "official-deposit-history".to_owned(),
                "exchange reported rollback".to_owned(),
                40,
            )
            .expect("deposit failure");

        let restored = OnchainReplenishmentPlanStore::load_path(Some(path), 50);
        let run = restored
            .run(&authorized.run.run_id, 50)
            .expect("restored failed run");
        assert_eq!(run.status, OnchainReplenishmentRunStatus::Failed);
        assert_eq!(
            run.transfers[0].status,
            OnchainReplenishmentTransferStatus::Failed
        );
        assert_eq!(run.transfers[0].confirmations, Some(4));
        assert_eq!(run.problem.as_deref(), Some("exchange reported rollback"));
    }
}
