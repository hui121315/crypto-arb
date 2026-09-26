use super::{apply_batch_snapshot, SnapshotState};
use crate::api::rest::{with_mutation_timeout, ApiError};
use crate::panels::shared::operation_journal::{
    validate_onchain_snapshot, validate_setting_response, OperationJournal,
};
use leptos::{prelude::*, task::spawn_local};
use shared_types::{ActionRunKind, ActionRunStatus, OnchainComparisonConfigPatch};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::onchain) struct ConfigurationRuntime {
    pub journal: OperationJournal,
    pub needs_current: RwSignal<bool>,
    pub recheck: Callback<()>,
    pub read_current: Callback<()>,
    pub update: Callback<OnchainComparisonConfigPatch>,
    pub add: Callback<OnchainComparisonConfigPatch>,
    pub remove: Callback<String>,
}

impl ConfigurationRuntime {
    pub(in crate::panels::modules::onchain) fn awaiting_confirmation(self) -> bool {
        self.journal.locked() || self.needs_current.get() || self.journal.connection.get() != 0
    }

    pub(super) fn new(
        snapshots: SnapshotState,
        journal: OperationJournal,
        needs_current: RwSignal<bool>,
        problem: RwSignal<Option<String>>,
    ) -> Self {
        let read_current = Callback::new(move |()| {
            if journal.connection.get_untracked() != 0 || journal.pending.with_untracked(Option::is_some) {
                return;
            }
            let Some(read_epoch) = snapshots.begin_recovery_read() else { return; };
            let epoch = journal.epoch.get_untracked();
            needs_current.set(true);
            let client = journal.client();
            spawn_local(async move {
                let result = with_mutation_timeout("读取当前链上配置", client.onchain_comparison()).await
                    .and_then(|snapshot| { validate_onchain_snapshot(&snapshot)?; Ok(snapshot) });
                if !journal.current(epoch) || !snapshots.finish_action(read_epoch) { return; }
                match result {
                    Ok(snapshot) => {
                        snapshots.apply_saved(snapshot);
                        needs_current.set(false);
                        problem.set(None);
                    }
                    Err(error) => problem.set(Some(format!("原操作已核对，但当前配置读取失败：{error}"))),
                }
            });
        });
        let query_receipt = journal.recheck(Callback::new(move |run: shared_types::ActionRun| {
            if run.status == ActionRunStatus::Succeeded {
                // A receipt proves the old write, not the current configuration.
                needs_current.set(true);
                read_current.run(());
            } else {
                problem.set(Some(format!("上次操作未成功：{}", run.problem.map_or(run.message, |p| p.message))));
            }
        }));
        let recheck = Callback::new(move |()| {
            if journal.connection.get_untracked() == 0 { query_receipt.run(()); }
        });
        let requests = ConfigurationRequests { snapshots, journal, problem };
        Self {
            journal, needs_current, recheck, read_current,
            update: Callback::new(move |patch| requests.submit(ConfigurationWrite::Update(patch))),
            add: Callback::new(move |patch| requests.submit(ConfigurationWrite::Add(patch))),
            remove: Callback::new(move |id: String| requests.submit(ConfigurationWrite::Remove(id.trim().to_owned()))),
        }
    }
}

enum ConfigurationWrite {
    Update(OnchainComparisonConfigPatch),
    Add(OnchainComparisonConfigPatch),
    Remove(String),
}

#[derive(Clone, Copy)]
struct ConfigurationRequests {
    snapshots: SnapshotState,
    journal: OperationJournal,
    problem: RwSignal<Option<String>>,
}

impl ConfigurationRequests {
    fn submit(self, write: ConfigurationWrite) {
        let Some(snapshot_epoch) = self.snapshots.begin_action() else { return; };
        let (kind, target) = match &write {
            ConfigurationWrite::Update(_) => (ActionRunKind::OnchainComparisonConfigUpdate, "onchain-cex-comparison".to_owned()),
            ConfigurationWrite::Add(_) => (ActionRunKind::OnchainBatchAdd, "onchain-cex-comparison".to_owned()),
            ConfigurationWrite::Remove(id) => (ActionRunKind::OnchainBatchRemove, id.clone()),
        };
        let Some(attempt) = self.journal.begin(kind, target) else {
            self.snapshots.finish_action(snapshot_epoch);
            return;
        };
        let epoch = self.journal.epoch.get_untracked();
        let client = self.journal.client();
        self.problem.set(None);
        spawn_local(async move {
            let result = with_mutation_timeout("更新链上监控配置", async {
                match write {
                    ConfigurationWrite::Update(patch) => {
                        let snapshot = client.update_onchain_comparison_with_context(&patch, &attempt.context).await?;
                        validate_setting_response(&attempt, &snapshot)?;
                        validate_submitted_amounts(&patch, &snapshot.config)?;
                        Ok((Some(snapshot), None))
                    }
                    ConfigurationWrite::Add(patch) => {
                        let batch = client.add_onchain_batch_with_context(&patch, &attempt.context).await?;
                        validate_setting_response(&attempt, &batch)?;
                        let expected = OnchainComparisonConfigPatch { enabled: Some(true), ..patch };
                        if !batch.items.iter().any(|item| validate_submitted_amounts(&expected, &item.config).is_ok()) {
                            return Err(mismatch());
                        }
                        Ok((None, Some(batch)))
                    }
                    ConfigurationWrite::Remove(id) => {
                        let batch = client.remove_onchain_batch_with_context(id, &attempt.context).await?;
                        validate_setting_response(&attempt, &batch)?;
                        Ok((None, Some(batch)))
                    }
                }
            }).await;
            if !self.journal.current(epoch) || !self.snapshots.finish_action(snapshot_epoch) { return; }
            match result {
                Ok((snapshot, batch)) => {
                    if let Some(snapshot) = snapshot { self.snapshots.apply_saved(snapshot); }
                    if let Some(batch) = batch { apply_batch_snapshot(self.snapshots.raw_state(), batch); }
                    self.journal.resolve(&attempt);
                    self.journal.busy.set(false);
                }
                Err(error) => {
                    self.journal.failed(&attempt, &error);
                    let label = if self.journal.locked() { "操作结果待核对" } else { "操作未成功" };
                    self.problem.set(Some(format!("{label}：{error}")));
                }
            }
        });
    }
}

fn mismatch() -> ApiError {
    ApiError::client("SETTINGS_RECEIPT_MISMATCH", "返回市场或金额与本次修改不一致，请核对原操作")
}

fn validate_submitted_amounts(patch: &OnchainComparisonConfigPatch, config: &shared_types::OnchainComparisonConfig) -> Result<(), ApiError> {
    let text_matches = [(&patch.chain, &config.chain), (&patch.cex_venue, &config.cex_venue),
        (&patch.cex_symbol, &config.cex_symbol)].into_iter()
        .all(|(expected, actual)| expected.as_deref().is_none_or(|v| v.trim().eq_ignore_ascii_case(actual)));
    let amounts_match = [(&patch.base_amount_raw, &config.base_amount_raw), (&patch.quote_amount_raw, &config.quote_amount_raw)]
        .into_iter().all(|(expected, actual)| expected.as_ref().is_none_or(|v| v == actual));
    let contracts_match = [(&patch.base_mint, &config.base_mint), (&patch.quote_mint, &config.quote_mint)]
        .into_iter().all(|(expected, actual)| expected.as_deref().is_none_or(|v|
            super::same_token_address(&config.chain, v.trim(), actual)));
    if text_matches && amounts_match && contracts_match
        && patch.enabled.is_none_or(|v| v == config.enabled)
        && patch.source.as_ref().and_then(|source| source.provider.as_deref()).is_none_or(|v| v.trim().eq_ignore_ascii_case(&config.provider)) {
        Ok(())
    } else { Err(mismatch()) }
}
