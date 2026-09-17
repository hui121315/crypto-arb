use common::config::AppConfig;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use shared_types::{
    OnchainCexOrderPlan, OnchainComparisonConfig, OnchainExecutionBuildResponse,
    OnchainExecutionRunStatus, OnchainExecutionSubmitResponse, OrderRecord,
};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u8 = 1;
mod cost_claims;
mod wallet_claims;
use cost_claims::CostClaims;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OnchainExecutionStage {
    Prepared,
    CexActionSubmitting,
    QuoteConversionFilled,
    PrimaryCexFilled,
    ChainBroadcasting,
    AwaitingChainFinality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OnchainCexActionKind {
    Primary,
    QuoteConversion,
    Compensation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OnchainQuoteConversionAttempt {
    pub(crate) plan: OnchainCexOrderPlan,
    pub(crate) record: OrderRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingOnchainExecution {
    pub(crate) response: OnchainExecutionSubmitResponse,
    pub(crate) config: OnchainComparisonConfig,
    pub(crate) build: OnchainExecutionBuildResponse,
    pub(crate) stage: OnchainExecutionStage,
    pub(crate) primary_plan: OnchainCexOrderPlan,
    pub(crate) primary_record: Option<OrderRecord>,
    pub(crate) quote_conversion_plan: Option<shared_types::OnchainQuoteConversionOrderPlan>,
    #[serde(default)]
    pub(crate) quote_conversion_records: Vec<OrderRecord>,
    #[serde(default)]
    pub(crate) quote_conversion_attempts: Vec<OnchainQuoteConversionAttempt>,
    pub(crate) transaction_id: Option<String>,
    #[serde(default)]
    pub(crate) active_cex_order: Option<OnchainCexOrderPlan>,
    #[serde(default)]
    pub(crate) active_cex_kind: Option<OnchainCexActionKind>,
    #[serde(default)]
    pub(crate) active_cex_record: Option<OrderRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cross_chain_cost_claim: Option<CrossChainCostClaim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response: Option<OnchainExecutionSubmitResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint: Option<Box<PendingOnchainExecution>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrossChainCostClaim {
    pub(crate) run_id: String,
    pub(crate) build_id: String,
    pub(crate) approval_costs: Vec<shared_types::OnchainExecutionApprovalCost>,
    pub(crate) replenishment_costs: Vec<shared_types::OnchainExecutionReplenishmentCost>,
}

#[derive(Debug)]
pub(crate) struct OnchainExecutionRunStore {
    path: Option<PathBuf>,
    append_lock: Mutex<()>,
    startup_problem: Option<String>,
    persistence_problem: Mutex<Option<String>>,
    checkpoints: Mutex<BTreeMap<String, PendingOnchainExecution>>,
    cost_claims: Mutex<CostClaims>,
    wallet_claims: std::sync::Arc<super::onchain_wallet_claims::WalletClaims>,
}

pub(crate) struct OnchainExecutionRunReplay {
    pub(crate) store: OnchainExecutionRunStore,
    pub(crate) runs: Vec<OnchainExecutionSubmitResponse>,
    pub(crate) pending: Vec<PendingOnchainExecution>,
}

impl OnchainExecutionRunStore {
    pub(crate) fn load(config: &AppConfig) -> OnchainExecutionRunReplay {
        let path = config
            .storage
            .onchain_execution_run_ledger_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| config.storage.resolve_runtime_path(path));
        Self::load_path(path)
    }

    fn load_path(path: Option<PathBuf>) -> OnchainExecutionRunReplay {
        let (runs, mut pending, recovery_problem, cost_claims) = replay(path.as_deref());
        let startup_problem = recovery_problem
            .or_else(|| {
                runs.iter()
                    .any(|r| {
                        matches!(
                            r.status,
                            OnchainExecutionRunStatus::Executing
                                | OnchainExecutionRunStatus::AwaitingChainFinality
                                | OnchainExecutionRunStatus::FinalityUnresolved
                        ) && !pending.iter().any(|p| p.response.run_id == r.run_id)
                    })
                    .then(|| "进行中的执行日志缺少钱包检查点，无法恢复资金占用".into())
            })
            .or_else(|| storage_problem(path.as_deref()));
        if startup_problem.is_some() {
            pending.clear();
        }
        let replay = OnchainExecutionRunReplay {
            store: Self {
                path,
                append_lock: Mutex::new(()),
                startup_problem,
                persistence_problem: Mutex::new(None),
                cost_claims: Mutex::new(cost_claims),
                wallet_claims: Default::default(),
                checkpoints: Mutex::new(
                    pending
                        .iter()
                        .map(|row| (row.response.run_id.clone(), row.clone()))
                        .collect(),
                ),
            },
            runs,
            pending,
        };
        replay.store.restore_wallet_claims();
        replay
    }

    pub(crate) fn append_run(
        &self,
        response: &OnchainExecutionSubmitResponse,
    ) -> Result<(), String> {
        self.append(&LogEntry {
            cross_chain_cost_claim: None,
            schema_version: SCHEMA_VERSION,
            response: Some(response.clone()),
            checkpoint: None,
        })
    }

    pub(crate) fn check_replenishment_available(
        &self,
        costs: &[shared_types::OnchainExecutionReplenishmentCost],
    ) -> Result<(), String> {
        self.readiness()?;
        self.cost_claims.lock().available(costs, None)
    }

    pub(crate) fn check_approval_available(&self, costs: &[shared_types::OnchainExecutionApprovalCost]) -> Result<(), String> {
        self.readiness()?;
        self.cost_claims.lock().approval_available(costs, None)
    }

    pub(crate) fn approval_cost_owner(&self, id: &str) -> Option<String> {
        self.cost_claims.lock().approval_owner(id)
    }

    pub(crate) fn append_pending(
        &self,
        checkpoint: &PendingOnchainExecution,
    ) -> Result<(), String> {
        self.append(&LogEntry {
            cross_chain_cost_claim: None,
            schema_version: SCHEMA_VERSION,
            response: None,
            checkpoint: Some(Box::new(checkpoint.clone())),
        })
    }

    pub(crate) fn readiness(&self) -> Result<(), String> {
        match (&self.path, &self.startup_problem) {
            (None, _) => Err("链上执行恢复日志未配置，已阻止真实提交".to_owned()),
            (_, Some(problem)) => Err(format!("链上执行恢复日志不可写：{problem}")),
            (Some(_), None) => Ok(()),
        }?;
        self.persistence_problem
            .lock()
            .as_ref()
            .map_or(Ok(()), |problem| {
                Err(format!(
                    "链上执行记录写入失败，已停止新增资金动作；请保留日志并核对后重启：{problem}"
                ))
            })
    }

    pub(crate) fn append_cex_intent(
        &self,
        run_id: &str,
        plan: &OnchainCexOrderPlan,
        kind: OnchainCexActionKind,
    ) -> Result<(), String> {
        let _guard = self.append_lock.lock();
        self.readiness()?;
        let mut checkpoint = self
            .checkpoints
            .lock()
            .get(run_id)
            .cloned()
            .ok_or("CEX 资金动作缺少已保存的执行计划，不能提交")?;
        if unresolved_cex(&checkpoint) {
            return Err("上一笔 CEX 订单终态未确认，不能重复下单或补偿".into());
        }
        checkpoint.stage = OnchainExecutionStage::CexActionSubmitting;
        checkpoint.active_cex_order = Some(plan.clone());
        checkpoint.active_cex_kind = Some(kind);
        checkpoint.active_cex_record = None;
        checkpoint.response.updated_at_ms = common::time::now_ms();
        checkpoint.response.message =
            "CEX 资金动作准备提交，已保留客户端订单号；结果未知时不可重复下单".into();
        self.append_unlocked(&LogEntry {
            cross_chain_cost_claim: None,
            schema_version: SCHEMA_VERSION,
            response: None,
            checkpoint: Some(Box::new(checkpoint)),
        })
    }

    pub(crate) fn unresolved_cex_action(&self, run_id: &str) -> bool {
        self.checkpoints
            .lock()
            .get(run_id)
            .is_some_and(unresolved_cex)
    }

    pub(crate) fn append_cex_observation(
        &self,
        run_id: &str,
        record: &OrderRecord,
    ) -> Result<(), String> {
        let _guard = self.append_lock.lock();
        self.readiness()?;
        let mut checkpoint = self
            .checkpoints
            .lock()
            .get(run_id)
            .cloned()
            .ok_or("CEX 订单缺少提交检查点")?;
        let plan = checkpoint
            .active_cex_order
            .as_ref()
            .ok_or("CEX 订单缺少客户端订单计划")?;
        if record.intent.client_order_id != plan.client_order_id
            || !shared_types::venue_names_equal(&record.intent.exchange, &plan.venue)
            || record.intent.side != plan.side
        {
            return Err("CEX 成交记录不属于待确认订单".into());
        }
        checkpoint.active_cex_record = Some(record.clone());
        match checkpoint.active_cex_kind {
            Some(OnchainCexActionKind::Primary) => checkpoint.primary_record = Some(record.clone()),
            Some(OnchainCexActionKind::QuoteConversion) => {
                let attempt = OnchainQuoteConversionAttempt {
                    plan: plan.clone(),
                    record: record.clone(),
                };
                if let Some(existing) = checkpoint
                    .quote_conversion_attempts
                    .iter_mut()
                    .find(|row| row.plan.client_order_id == plan.client_order_id)
                {
                    *existing = attempt;
                } else {
                    checkpoint.quote_conversion_attempts.push(attempt);
                }
                checkpoint.quote_conversion_records = checkpoint
                    .quote_conversion_attempts
                    .iter()
                    .map(|row| row.record.clone())
                    .collect();
            }
            _ => {}
        }
        checkpoint.response.updated_at_ms = common::time::now_ms();
        self.append_unlocked(&LogEntry {
            cross_chain_cost_claim: None,
            schema_version: SCHEMA_VERSION,
            response: None,
            checkpoint: Some(Box::new(checkpoint)),
        })
    }

    fn append(&self, entry: &LogEntry) -> Result<(), String> {
        let _guard = self.append_lock.lock();
        self.append_unlocked(entry)
    }

    fn append_unlocked(&self, entry: &LogEntry) -> Result<(), String> {
        use super::onchain_wallet_claims::{Module, Owner};

        self.readiness()?;
        validate_entry(entry)?;
        if let Some(claim) = &entry.cross_chain_cost_claim {
            if self.cost_claims.lock().validate_cross_chain(claim)? { return Ok(()); }
            return self.wallet_claims.persist_unclaimed(Module::Execution, || {
                let path = self.path.as_deref().ok_or("执行恢复日志未配置")?;
                if let Err(error) = append_jsonl(path, entry) {
                    *self.persistence_problem.lock() = Some(error.to_string());
                    return self.readiness();
                }
                self.cost_claims.lock().record_cross_chain(claim);
                Ok(())
            });
        }
        let response = entry
            .response
            .as_ref()
            .or_else(|| entry.checkpoint.as_ref().map(|c| &c.response))
            .ok_or("执行记录缺失")?;
        self.cost_claims.lock().validate(response)?;
        let hold = self.wallet_hold(entry)?;
        self.wallet_claims.commit(
            Owner::new(Module::Execution, &response.run_id),
            hold,
            response.updated_at_ms,
            || {
                let path = self.path.as_deref().ok_or("链上执行恢复日志未配置")?;
                if let Err(error) = append_jsonl(path, entry) {
                    tracing::warn!(path = %path.display(), %error, "failed to persist on-chain execution state");
                    *self.persistence_problem.lock() = Some(error.to_string());
                    return self.readiness();
                }
                // Ownership and prepared execution share the same fsynced journal row.
                self.cost_claims.lock().record(response);
                if let Some(checkpoint) = &entry.checkpoint {
                    self.checkpoints
                        .lock()
                        .insert(checkpoint.response.run_id.clone(), *checkpoint.clone());
                }
                if let Some(response) = &entry.response {
                    if !matches!(
                        response.status,
                        OnchainExecutionRunStatus::Executing
                            | OnchainExecutionRunStatus::AwaitingChainFinality
                            | OnchainExecutionRunStatus::FinalityUnresolved
                    ) {
                        self.checkpoints.lock().remove(&response.run_id);
                    }
                }
                Ok(())
            },
        )
    }

    pub(crate) fn claim_cross_chain_costs(&self, claim: CrossChainCostClaim) -> Result<(), String> {
        self.append(&LogEntry { schema_version: SCHEMA_VERSION, response: None, checkpoint: None, cross_chain_cost_claim: Some(claim) })
    }
}

fn storage_problem(path: Option<&Path>) -> Option<String> {
    let path = path?;
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .sync_all()?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    result.err().map(|error| error.to_string())
}

fn replay(
    path: Option<&Path>,
) -> (
    Vec<OnchainExecutionSubmitResponse>,
    Vec<PendingOnchainExecution>,
    Option<String>,
    CostClaims,
) {
    let Some(path) = path else {
        return (Vec::new(), Vec::new(), None, CostClaims::default());
    };
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Vec::new(), Vec::new(), None, CostClaims::default());
        }
        Err(error) => {
            return (
                Vec::new(),
                Vec::new(),
                Some(format!("无法读取恢复日志：{error}")),
                CostClaims::default(),
            )
        }
    };
    let mut state = ReplayState::default();
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
            break Some(format!(
                "恢复日志第 {line_number} 行未完整写入；不能跳过或继续追加"
            ));
        }
        let entry = match serde_json::from_str::<LogEntry>(line.trim()) {
            Ok(entry) if entry.schema_version == SCHEMA_VERSION => entry,
            Ok(_) => break Some(format!("恢复日志第 {line_number} 行版本不支持")),
            Err(_) => break Some(format!("恢复日志第 {line_number} 行损坏")),
        };
        if let Err(error) = validate_entry(&entry) {
            break Some(format!("恢复日志第 {line_number} 行无效：{error}"));
        }
        if let Err(error) = state.apply(entry) {
            break Some(format!("恢复日志第 {line_number} 行无效：{error}"));
        }
    };
    let (runs, pending, claims) = state.finish();
    (runs, pending, problem, claims)
}

#[derive(Default)]
struct ReplayState {
    runs: BTreeMap<String, OnchainExecutionSubmitResponse>,
    pending: BTreeMap<String, PendingOnchainExecution>,
    cost_claims: CostClaims,
}

impl ReplayState {
    fn apply(&mut self, entry: LogEntry) -> Result<(), String> {
        if let Some(claim) = &entry.cross_chain_cost_claim {
            self.cost_claims.validate_cross_chain(claim)?;
            self.cost_claims.record_cross_chain(claim);
            return Ok(());
        }
        let response = entry
            .response
            .as_ref()
            .or_else(|| entry.checkpoint.as_ref().map(|c| &c.response))
            .ok_or("执行记录缺失")?;
        self.cost_claims.validate(response)?;
        self.cost_claims.record(response);
        match (entry.response, entry.checkpoint) {
            (Some(response), None) => self.apply_run(response),
            (None, Some(checkpoint)) => self.apply_pending(*checkpoint),
            _ => return Err("记录必须包含一个运行结果或检查点".into()),
        }
        Ok(())
    }

    fn apply_run(&mut self, response: OnchainExecutionSubmitResponse) {
        if !matches!(
            response.status,
            OnchainExecutionRunStatus::Executing
                | OnchainExecutionRunStatus::AwaitingChainFinality
                | OnchainExecutionRunStatus::FinalityUnresolved
        ) {
            self.pending.remove(&response.run_id);
        }
        self.runs.insert(response.run_id.clone(), response);
    }

    fn apply_pending(&mut self, checkpoint: PendingOnchainExecution) {
        let run_id = checkpoint.response.run_id.clone();
        self.runs
            .insert(run_id.clone(), checkpoint.response.clone());
        self.pending.insert(run_id, checkpoint);
    }

    fn finish(
        mut self,
    ) -> (
        Vec<OnchainExecutionSubmitResponse>,
        Vec<PendingOnchainExecution>,
        CostClaims,
    ) {
        self.pending.retain(|run_id, _| {
            self.runs.get(run_id).is_some_and(|run| {
                matches!(
                    run.status,
                    OnchainExecutionRunStatus::Executing
                        | OnchainExecutionRunStatus::AwaitingChainFinality
                        | OnchainExecutionRunStatus::FinalityUnresolved
                )
            })
        });
        (
            self.runs.into_values().collect(),
            self.pending.into_values().collect(),
            self.cost_claims,
        )
    }
}

fn append_jsonl(path: &Path, entry: &LogEntry) -> std::io::Result<()> {
    let mut row = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    row.push(b'\n');
    // A missing journal after startup is a fault, not permission to recreate an empty one.
    let mut file = OpenOptions::new().append(true).open(path)?;
    file.write_all(&row)?;
    file.sync_all()
}

fn unresolved_cex(checkpoint: &PendingOnchainExecution) -> bool {
    checkpoint.stage == OnchainExecutionStage::CexActionSubmitting
        && checkpoint.active_cex_record.as_ref().is_none_or(|record| {
            !matches!(
                record.state,
                shared_types::LiveOrderState::Filled
                    | shared_types::LiveOrderState::Cancelled
                    | shared_types::LiveOrderState::Rejected
                    | shared_types::LiveOrderState::Failed
            )
        })
}

fn validate_entry(entry: &LogEntry) -> Result<(), String> {
    if entry.cross_chain_cost_claim.is_some() {
        if entry.response.is_some() || entry.checkpoint.is_some() { return Err("费用归属记录不可混入其他执行状态".into()); }
        return Ok(());
    }
    let response = match (&entry.response, &entry.checkpoint) {
        (Some(response), None) => response,
        (None, Some(checkpoint)) => {
            if checkpoint.response.build_id != checkpoint.build.build_id
                || checkpoint.response.replenishment_costs != checkpoint.build.replenishment_costs
                || checkpoint.response.approval_costs != checkpoint.build.approval_costs
                || checkpoint.transaction_id != checkpoint.response.chain_transaction_id
                || (checkpoint.stage == OnchainExecutionStage::CexActionSubmitting
                    && checkpoint
                        .active_cex_order
                        .as_ref()
                        .is_none_or(|order| order.client_order_id.trim().is_empty()))
                || (checkpoint.stage == OnchainExecutionStage::Prepared
                    && (checkpoint.primary_record.is_some()
                        || checkpoint.transaction_id.is_some()
                        || checkpoint.active_cex_order.is_some()))
            {
                return Err("执行检查点的计划、交易哈希或待提交订单不一致".into());
            }
            for cost in &checkpoint.build.approval_costs {
                crate::services::onchain_comparison::approval_allocation::matches_execution(
                    cost, &checkpoint.config, checkpoint.build.direction, &checkpoint.build.chain_transaction,
                )?;
            }
            &checkpoint.response
        }
        _ => return Err("记录必须包含一个运行结果或检查点".into()),
    };
    if response.run_id.trim().is_empty() || response.build_id.trim().is_empty() {
        return Err("运行或计划标识缺失".into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_checkpoint() -> PendingOnchainExecution {
    tests::checkpoint()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        ExecutionMode, InstrumentAssetClass, InstrumentListingStatus, InstrumentMetadataSource,
        LiveOrderState, MarginMode, OnchainComparisonDirection, OnchainUnsignedTransaction,
        OrderIntent, OrderSide, OrderSizingPlan, OrderSource, OrderType, OrderUpdateSource,
        StrategyKind, TimeInForce, VenueInstrument, VenueOrderIdentity,
    };

    #[test]
    fn pending_checkpoint_survives_restart_and_terminal_run_clears_it() {
        let path = temp_path();
        let replay = OnchainExecutionRunStore::load_path(Some(path.clone()));
        let checkpoint = checkpoint();
        serde_json::from_value::<OnchainExecutionSubmitResponse>(
            serde_json::to_value(&checkpoint.response).expect("response should encode"),
        )
        .expect("response should decode");
        serde_json::from_value::<OnchainComparisonConfig>(
            serde_json::to_value(&checkpoint.config).expect("config should encode"),
        )
        .expect("config should decode");
        serde_json::from_value::<OnchainExecutionBuildResponse>(
            serde_json::to_value(&checkpoint.build).expect("build should encode"),
        )
        .expect("build should decode");
        let primary_record = serde_json::from_value::<Option<OrderRecord>>(
            serde_json::to_value(&checkpoint.primary_record).expect("record should encode"),
        )
        .expect("record should decode");
        assert!(primary_record.is_some());
        replay.store.append_pending(&checkpoint).unwrap();
        let raw = std::fs::read_to_string(&path).expect("checkpoint log should be readable");
        serde_json::from_str::<LogEntry>(raw.trim()).expect("checkpoint row should decode");

        let restored = OnchainExecutionRunStore::load_path(Some(path.clone()));
        assert_eq!(restored.runs.len(), 1);
        assert_eq!(restored.pending.len(), 1);
        assert_eq!(
            restored.pending[0].transaction_id.as_deref(),
            Some("chain-tx")
        );

        let mut terminal = checkpoint.response;
        terminal.status = OnchainExecutionRunStatus::Completed;
        terminal.updated_at_ms = 3;
        restored.store.append_run(&terminal).unwrap();
        let completed = OnchainExecutionRunStore::load_path(Some(path.clone()));
        assert_eq!(completed.runs, vec![terminal]);
        assert!(completed.pending.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn live_submission_requires_a_writable_recovery_log() {
        assert!(OnchainExecutionRunStore::load_path(None)
            .store
            .readiness()
            .is_err());

        let blocked_parent = temp_path();
        std::fs::write(&blocked_parent, b"not a directory").expect("fixture should be writable");
        let blocked = OnchainExecutionRunStore::load_path(Some(blocked_parent.join("runs.jsonl")));
        assert!(blocked.store.readiness().is_err());
        let _ = std::fs::remove_file(blocked_parent);
    }

    #[test]
    fn execution_recovery_rejects_damage_without_skipping_or_appending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        let checkpoint = checkpoint();
        let good = serde_json::to_string(&LogEntry {
            cross_chain_cost_claim: None,
            schema_version: 1,
            response: None,
            checkpoint: Some(Box::new(checkpoint.clone())),
        })
        .unwrap();
        let wrong_build = serde_json::to_string(&LogEntry {
            cross_chain_cost_claim: None,
            schema_version: 1,
            response: None,
            checkpoint: Some(Box::new(PendingOnchainExecution {
                build: build(cex_plan()),
                response: OnchainExecutionSubmitResponse {
                    build_id: "wrong".into(),
                    ..checkpoint.response.clone()
                },
                ..checkpoint.clone()
            })),
        })
        .unwrap();
        for tail in [
            "{broken}\n".to_owned(),
            good.clone(),
            "\n".into(),
            "{\"schemaVersion\":2}\n".into(),
            "{\"schemaVersion\":1}\n".into(),
            format!("{wrong_build}\n"),
        ] {
            let bytes = format!("{good}\n{tail}");
            std::fs::write(&path, &bytes).unwrap();
            let restored = OnchainExecutionRunStore::load_path(Some(path.clone()));
            assert_eq!(restored.runs.len(), 1);
            assert!(restored.pending.is_empty());
            assert!(restored.store.readiness().unwrap_err().contains("第 2 行"));
            assert!(restored.store.append_pending(&checkpoint).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn execution_recovery_write_failure_latches_and_preserves_the_order_intent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        let saved = dir.path().join("saved.jsonl");
        let replay = OnchainExecutionRunStore::load_path(Some(path.clone()));
        let checkpoint = checkpoint();
        let mut conversion = cex_plan();
        conversion.client_order_id = "quote-order-2".into();
        assert!(replay
            .store
            .append_cex_intent(
                "missing",
                &conversion,
                OnchainCexActionKind::QuoteConversion
            )
            .is_err());
        assert!(std::fs::read(&path).unwrap().is_empty());
        replay.store.append_pending(&checkpoint).unwrap();
        replay
            .store
            .append_cex_intent("run-1", &conversion, OnchainCexActionKind::QuoteConversion)
            .unwrap();
        let restored = OnchainExecutionRunStore::load_path(Some(path.clone()));
        assert_eq!(
            restored.pending[0].stage,
            OnchainExecutionStage::CexActionSubmitting
        );
        assert_eq!(
            restored.pending[0]
                .active_cex_order
                .as_ref()
                .unwrap()
                .client_order_id,
            "quote-order-2"
        );
        assert_eq!(
            restored.pending[0].transaction_id,
            checkpoint.transaction_id
        );
        let bytes = std::fs::read(&path).unwrap();
        std::fs::rename(&path, &saved).unwrap();
        assert!(replay.store.append_pending(&checkpoint).is_err());
        assert!(!path.exists(), "a lost journal must not be recreated");
        std::fs::rename(&saved, &path).unwrap();
        assert!(replay.store.append_run(&checkpoint.response).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let restarted = OnchainExecutionRunStore::load_path(Some(path));
        assert!(restarted.store.readiness().is_ok());
        assert_eq!(restarted.pending.len(), 1);
        assert_eq!(
            restarted.pending[0]
                .active_cex_order
                .as_ref()
                .unwrap()
                .client_order_id,
            "quote-order-2"
        );
    }

    #[test]
    fn execution_recovery_retains_partial_conversion_receipts_and_blocks_unknown_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        let replay = OnchainExecutionRunStore::load_path(Some(path.clone()));
        let checkpoint = checkpoint();
        replay.store.append_pending(&checkpoint).unwrap();
        let mut plan = cex_plan();
        plan.client_order_id = "quote-1".into();
        replay
            .store
            .append_cex_intent("run-1", &plan, OnchainCexActionKind::QuoteConversion)
            .unwrap();
        assert!(replay.store.unresolved_cex_action("run-1"));
        let mut record = order_record();
        record.intent.client_order_id = plan.client_order_id.clone();
        record.state = LiveOrderState::PartiallyFilled;
        record.filled_quantity = Some(0.25);
        replay
            .store
            .append_cex_observation("run-1", &record)
            .unwrap();
        let mut next = plan.clone();
        next.client_order_id = "quote-2".into();
        assert!(replay
            .store
            .append_cex_intent("run-1", &next, OnchainCexActionKind::QuoteConversion)
            .is_err());
        let mut unresolved = checkpoint.response.clone();
        unresolved.status = OnchainExecutionRunStatus::FinalityUnresolved;
        replay.store.append_run(&unresolved).unwrap();
        let restored = OnchainExecutionRunStore::load_path(Some(path.clone()));
        assert_eq!(restored.pending.len(), 1);
        assert_eq!(
            restored.pending[0].quote_conversion_attempts[0]
                .record
                .filled_quantity,
            Some(0.25)
        );
        record.state = LiveOrderState::Cancelled;
        replay
            .store
            .append_cex_observation("run-1", &record)
            .unwrap();
        assert!(!replay.store.unresolved_cex_action("run-1"));
        replay
            .store
            .append_cex_intent("run-1", &next, OnchainCexActionKind::QuoteConversion)
            .unwrap();
        record.intent.client_order_id = next.client_order_id;
        record.state = LiveOrderState::Filled;
        record.filled_quantity = Some(0.75);
        replay
            .store
            .append_cex_observation("run-1", &record)
            .unwrap();
        let restored = OnchainExecutionRunStore::load_path(Some(path));
        assert_eq!(restored.pending[0].quote_conversion_attempts.len(), 2);
        assert_eq!(
            restored.pending[0].quote_conversion_attempts[0]
                .record
                .filled_quantity,
            Some(0.25)
        );
        assert_eq!(
            restored.pending[0].quote_conversion_attempts[1]
                .record
                .filled_quantity,
            Some(0.75)
        );
    }

    #[tokio::test]
    async fn execution_recovery_http_preserves_known_rows_and_blocks_funds_on_damage() {
        use axum::{
            body::{to_bytes, Body},
            http::{header, Request, StatusCode},
        };
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        let row = checkpoint();
        let bytes = format!(
            "{}\n{{\"schemaVersion\":1",
            serde_json::to_string(&LogEntry {
                cross_chain_cost_claim: None,
                schema_version: 1,
                response: None,
                checkpoint: Some(Box::new(row.clone()))
            })
            .unwrap()
        );
        std::fs::write(&path, &bytes).unwrap();
        let mut config = AppConfig::default();
        config.history.enabled = false;
        config.security.auth_token = Some("offline-recovery".into());
        config.storage.onchain_execution_run_ledger_path = Some(path.to_string_lossy().into());
        let state = crate::state::AppState::new(config).await.unwrap();
        let router = crate::app::build_router(state);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/onchain/execution/runs")
                    .header(header::AUTHORIZATION, "Bearer offline-recovery")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let snapshot: shared_types::OnchainExecutionRunsResponse =
            serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(snapshot.rows[0].cex_order_id, row.response.cex_order_id);
        assert!(snapshot.recovery_problem.unwrap().contains("第 2 行"));
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/onchain/execution/submit")
                    .header(header::AUTHORIZATION, "Bearer offline-recovery")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{\"buildId\":\"build-1\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(String::from_utf8_lossy(
            &to_bytes(response.into_body(), 128 * 1024).await.unwrap()
        )
        .contains("ONCHAIN_EXECUTION_RECOVERY_UNAVAILABLE"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
    }

    pub(super) fn checkpoint() -> PendingOnchainExecution {
        let plan = cex_plan();
        let response = OnchainExecutionSubmitResponse {
            run_id: "run-1".to_owned(),
            build_id: "build-1".to_owned(),
            status: OnchainExecutionRunStatus::AwaitingChainFinality,
            cex_order_id: Some("order-1".to_owned()),
            cex_order_state: Some(LiveOrderState::Filled),
            cex_filled_quantity: Some(1.0),
            chain_transaction_id: Some("chain-tx".to_owned()),
            compensation_order_id: None,
            legs: Vec::new(),
            recovery_actions: Vec::new(),
            replenishment_costs: Vec::new(),
            approval_costs: Vec::new(),
            estimated_net_profit_usd: 1.0,
            remaining_exposure_usd: 100.0,
            quantity_reconciled: false,
            accounting: None,
            message: "pending".to_owned(),
            problem: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        PendingOnchainExecution {
            response,
            config: OnchainComparisonConfig::default(),
            build: build(plan.clone()),
            stage: OnchainExecutionStage::AwaitingChainFinality,
            primary_plan: plan,
            primary_record: Some(order_record()),
            quote_conversion_plan: None,
            quote_conversion_records: Vec::new(),
            quote_conversion_attempts: Vec::new(),
            transaction_id: Some("chain-tx".to_owned()),
            active_cex_order: None,
            active_cex_kind: None,
            active_cex_record: None,
        }
    }

    fn build(cex_order: OnchainCexOrderPlan) -> OnchainExecutionBuildResponse {
        OnchainExecutionBuildResponse {
            build_id: "build-1".to_owned(),
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            provider: "jupiter_swap_v2".to_owned(),
            chain: "solana".to_owned(),
            wallet_address: "wallet".to_owned(),
            input_token: "USDC".to_owned(),
            output_token: "SOL".to_owned(),
            input_amount_raw: "100000000".to_owned(),
            output_amount_raw: "1000000000".to_owned(),
            chain_transaction: OnchainUnsignedTransaction::SolanaVersioned {
                transaction_base64: "unsigned".to_owned(),
                request_id: "request".to_owned(),
                router: "jupiter".to_owned(),
                mode: "fast".to_owned(),
                last_valid_block_height: None,
                expire_at_ms: None,
            },
            minimum_output_amount_raw: None,
            chain_input_adjustment: None,
            settlement_assets: None,
            cex_order,
            quote_conversion_order: None,
            quote_usd_valuation: None,
            replenishment_costs: Vec::new(),
            approval_costs: Vec::new(),
            estimated_net_profit_usd: 1.0,
            estimated_net_spread_bps: 100.0,
            quote_observed_at_ms: 1,
            cex_observed_at_ms: 1,
            built_at_ms: 1,
            valid_until_ms: 10,
            official_docs_url: "docs".to_owned(),
            build_ready: true,
            submit_ready: true,
            blockers: Vec::new(),
        }
    }

    fn cex_plan() -> OnchainCexOrderPlan {
        OnchainCexOrderPlan {
            venue: "kraken".to_owned(),
            native_symbol: "SOL/USD".to_owned(),
            client_order_id: "client".to_owned(),
            side: OrderSide::Sell,
            base_quantity: 1.0,
            reference_price: 100.0,
            estimated_quote_amount: 100.0,
            instrument_spec: VenueInstrument {
                venue: "kraken".to_owned(),
                native_symbol: "SOL/USD".to_owned(),
                canonical_symbol: "SOL".to_owned(),
                display_symbol: "SOL/USD".to_owned(),
                asset_class: InstrumentAssetClass::Crypto,
                product_type: Some("spot".to_owned()),
                quote_asset: Some("USD".to_owned()),
                settle_asset: None,
                margin_asset: None,
                contract_size: Some(1.0),
                execution_supported: true,
                price_tick: Some(0.01),
                qty_step: Some(0.001),
                min_qty: Some(0.001),
                min_notional: Some(1.0),
                listing_status: InstrumentListingStatus::Trading,
                funding_interval_ms: None,
                builder_dex: None,
                source: InstrumentMetadataSource::OfficialEndpoint,
                source_url: None,
                checked_at_ms: 1,
                schema_version: Some("test".to_owned()),
            },
            sizing_plan: OrderSizingPlan::default(),
        }
    }

    fn order_record() -> OrderRecord {
        let intent = OrderIntent {
            id: "order-1".to_owned(),
            source: OrderSource::Strategy,
            strategy: Some(StrategyKind::OnchainDepeg),
            mode: ExecutionMode::Live,
            exchange: "kraken".to_owned(),
            symbol: "SOL/USD".to_owned(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: None,
            slippage_tolerance_bps: Some(10.0),
            reduce_only: false,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        };
        OrderRecord {
            identity: VenueOrderIdentity::from_intent(&intent),
            intent,
            state: LiveOrderState::Filled,
            risk: None,
            last_update_source: OrderUpdateSource::Internal,
            exchange_order_id: Some("exchange-order".to_owned()),
            message: None,
            filled_quantity: Some(1.0),
            filled_price: Some(100.0),
            filled_fee: None,
            updated_at_ms: 2,
        }
    }

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-onchain-execution-{}-{}.jsonl",
            std::process::id(),
            common::time::now_ms()
        ))
    }
}
