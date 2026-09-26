use super::{apply_automation_action, ConfirmedAt, Requests};
use crate::api::rest::{with_mutation_timeout, ApiError};
use crate::panels::shared::operation_journal::{
    validate_automation_snapshot, validate_setting_response, OperationJournal,
};
use leptos::{prelude::*, task::spawn_local};
use shared_types::{
    ActionRunKind, ActionRunStatus, AutomatedArbitrageConfig, AutomatedArbitrageConfigPatch,
    AutomationControlAction, AutomationControlRequest,
};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::automation) struct AutomationMutations {
    pub journal: OperationJournal,
    pub needs_current: RwSignal<bool>,
    pub recheck: Callback<()>,
    pub read_current: Callback<()>,
    pub update: Callback<AutomatedArbitrageConfigPatch>,
    pub control: Callback<AutomationControlRequest>,
}

impl AutomationMutations {
    pub(super) fn new(
        requests: Requests,
        journal: OperationJournal,
        needs_current: RwSignal<bool>,
        saved: RwSignal<u64>,
    ) -> Self {
        let read_current = Callback::new(move |()| {
            if journal.connection.get_untracked() != 0 || journal.pending.with_untracked(Option::is_some)
                || requests.reading.get_untracked() { return; }
            requests.invalidate_reads();
            requests.reading.set(true);
            needs_current.set(true);
            let revision = requests.revision.get_untracked();
            let epoch = journal.epoch.get_untracked();
            let started = ConfirmedAt::now();
            let client = journal.client();
            spawn_local(async move {
                let result = with_mutation_timeout("读取当前自动化配置", client.automation_status()).await
                    .and_then(|next| { validate_automation_snapshot(&next)?; Ok(next) });
                if !journal.current(epoch) || requests.revision.get_untracked() != revision { return; }
                requests.reading.set(false);
                match result {
                    Ok(next) if !started.expired() => {
                        requests.status.set(crate::state::load_state::LoadState::Ready(next));
                        requests.last_received.set(Some(started));
                        requests.source.set("核对后读取");
                        requests.notice.set(Some("原操作已核对，已载入当前自动化配置".into()));
                        saved.update(|version| *version += 1);
                        needs_current.set(false);
                    }
                    result => requests.notice.set(Some(format!("原操作已核对，当前配置仍待确认：{}",
                        result.err().map_or_else(|| "读取耗时过长，请重试".into(), |error| error.to_string())))),
                }
            });
        });
        let query = journal.recheck(Callback::new(move |run: shared_types::ActionRun| {
            // The receipt proves a historical write; never hydrate from its old payload.
            requests.notice.set(Some(if run.status == ActionRunStatus::Succeeded {
                "原操作已完成，正在读取当前配置".into()
            } else { format!("原操作未成功：{}；正在读取当前状态", run.message) }));
            read_current.run(());
        }));
        let recheck = Callback::new(move |()| {
            if journal.connection.get_untracked() == 0 { query.run(()); }
        });
        let writes = Writes { requests, journal, saved };
        Self { journal, needs_current, recheck, read_current,
            update: Callback::new(move |patch| writes.submit(Write::Config(patch))),
            control: Callback::new(move |request| writes.submit(Write::Control(request))),
        }
    }
}

enum Write {
    Config(AutomatedArbitrageConfigPatch),
    Control(AutomationControlRequest),
}

#[derive(Clone, Copy)]
struct Writes {
    requests: Requests,
    journal: OperationJournal,
    saved: RwSignal<u64>,
}

impl Writes {
    fn submit(self, write: Write) {
        if self.requests.blocked.get_untracked() { return; }
        let (kind, message) = match &write {
            Write::Config(_) => (ActionRunKind::AutomationConfigUpdate, "自动化配置已保存"),
            Write::Control(_) => (ActionRunKind::AutomationControl, "自动化控制已确认"),
        };
        let Some(attempt) = self.journal.begin(kind, "automated-arbitrage".into()) else { return; };
        self.requests.invalidate_reads();
        self.requests.notice.set(Some("正在更新自动化…".into()));
        let epoch = self.journal.epoch.get_untracked();
        let client = self.journal.client();
        let saving_draft = matches!(&write, Write::Config(patch) if patch.capital_usd.is_some());
        spawn_local(async move {
            let result = with_mutation_timeout("更新自动化", async {
                let response = match &write {
                    Write::Config(patch) => client.update_automation_config_with_context(patch, &attempt.context).await?,
                    Write::Control(request) => client.control_automation_with_context(request, &attempt.context).await?,
                };
                validate_setting_response(&attempt, &response)?;
                validate_write(&write, &response.config)?;
                Ok(response)
            }).await;
            if !self.journal.current(epoch) { return; }
            match result {
                Ok(response) => {
                    self.requests.status.update(|state| apply_automation_action(state, Ok(response)));
                    self.requests.last_received.set(Some(ConfirmedAt::now()));
                    self.requests.source.set("操作结果");
                    if self.journal.resolve(&attempt) {
                        self.requests.notice.set(Some(message.into()));
                        if saving_draft { self.saved.update(|version| *version += 1); }
                    }
                    self.journal.busy.set(false);
                }
                Err(error) => {
                    self.journal.failed(&attempt, &error);
                    let label = if self.journal.locked() { "操作结果待核对" } else { "操作未成功" };
                    self.requests.notice.set(Some(format!("{label}：{error}")));
                }
            }
        });
    }
}

fn validate_write(write: &Write, current: &AutomatedArbitrageConfig) -> Result<(), ApiError> {
    let matches = match write {
        Write::Control(request) => match request.action {
            AutomationControlAction::Pause => current.paused,
            AutomationControlAction::Resume => current.enabled && !current.paused,
            AutomationControlAction::EmergencyStop => !current.enabled && current.paused,
        },
        Write::Config(patch) => {
            let expected = serde_json::to_value(patch).map_err(|_| mismatch())?;
            let actual = serde_json::to_value(current).map_err(|_| mismatch())?;
            expected.as_object().is_some_and(|fields| fields.iter().all(|(key, value)| {
                if value.is_null() { return true; }
                if key == "canonicalSymbols" {
                    let normalized = patch.canonical_symbols.as_ref().into_iter().flatten()
                        .map(|v| v.trim().to_ascii_uppercase()).filter(|v| !v.is_empty())
                        .collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>();
                    return normalized == current.canonical_symbols;
                }
                actual.get(key) == Some(value)
            }))
        }
    };
    if matches { Ok(()) } else { Err(mismatch()) }
}

fn mismatch() -> ApiError {
    ApiError::client("AUTOMATION_RECEIPT_MISMATCH", "返回配置与本次操作不一致，请核对原请求")
}
