use super::*;
use crate::api::rest::{with_mutation_timeout, ApiError};
use crate::panels::shared::operation_journal::{validate_setting_response, OperationJournal};
use shared_types::{ActionRunKind, ActionRunStatus};

mod storage;

#[derive(Clone, Copy)]
pub(super) struct BatchRuntime {
    selected: RwSignal<Vec<String>>,
    budget: RwSignal<String>,
    keyed: RwSignal<bool>,
    interval: RwSignal<u32>,
    pending: RwSignal<bool>,
    problem: RwSignal<Option<String>>,
    storage_problem: RwSignal<Option<String>>,
    initialized: StoredValue<bool>,
    applied: StoredValue<Option<StockBatchRequest>>,
    revision: RwSignal<String>,
    pub(super) journal: OperationJournal,
    reading: RwSignal<bool>,
    scope: StoredValue<String>,
}

impl BatchRuntime {
    pub(super) fn new(market: RwSignal<LoadState<StockMarketSnapshot>>) -> Self {
        let journal = OperationJournal::new("stocks-batch");
        let runtime = Self {
            selected: RwSignal::new(vec![]), budget: RwSignal::new("100".into()),
            keyed: RwSignal::new(false), interval: RwSignal::new(15),
            pending: RwSignal::new(false), problem: RwSignal::new(None),
            storage_problem: RwSignal::new(None), initialized: StoredValue::new(false),
            applied: StoredValue::new(None), revision: RwSignal::new(String::new()), journal, reading: RwSignal::new(false),
            scope: StoredValue::new(String::new()),
        };
        Effect::new(move |_| {
            journal.connection.track();
            runtime.scope.set_value(journal.draft_storage_key());
            runtime.initialized.set_value(false);
            runtime.applied.set_value(None);
            runtime.revision.set(String::new());
            runtime.reading.set(false);
            runtime.problem.set(None);
            runtime.storage_problem.set(None);
            market.set(LoadState::Loading);
            match storage::load(&runtime.scope.get_value()) {
                Ok(Some(draft)) => {
                    runtime.initialized.set_value(true);
                    runtime.applied.set_value(draft.applied);
                    runtime.revision.set(draft.revision);
                    runtime.set_draft(&draft.request);
                }
                Ok(None) => runtime.set_draft(&default_draft()),
                Err(error) => {
                    runtime.set_draft(&default_draft());
                    runtime.storage_problem.set(Some(error));
                }
            }
        });
        Effect::new(move |_| {
            runtime.selected.track(); runtime.budget.track();
            runtime.keyed.track(); runtime.interval.track();
            runtime.persist();
        });
        Effect::new(move |_| runtime.pending.set(journal.locked() || runtime.reading.get()
            || runtime.storage_problem.with(Option::is_some)
            || market.with(|value| !matches!(value, LoadState::Ready(snapshot) if !snapshot.batch.revision.is_empty()))));
        runtime
    }

    fn draft(self, enabled: bool) -> StockBatchRequest {
        StockBatchRequest { enabled, assets: self.selected.get_untracked(),
            budget_usdc: self.budget.get_untracked(), keyed: self.keyed.get_untracked(),
            interval_secs: self.interval.get_untracked() }
    }

    fn set_draft(self, request: &StockBatchRequest) {
        self.selected.set(request.assets.clone()); self.budget.set(request.budget_usdc.clone());
        self.keyed.set(request.keyed); self.interval.set(request.interval_secs);
    }

    fn dirty(self) -> bool {
        self.initialized.get_value() && self.applied.with_value(|saved| saved.as_ref()
            .is_none_or(|saved| self.draft(saved.enabled) != *saved))
    }

    fn persist(self) -> bool {
        if !self.initialized.get_value() || self.storage_problem.get_untracked().is_some() {
            return self.storage_problem.get_untracked().is_none();
        }
        match storage::save(&self.scope.get_value(), &storage::Draft {
            version: 1, request: self.draft(false), applied: self.applied.get_value(),
            revision: self.revision.get_untracked(),
        }) {
            Ok(()) => true,
            Err(error) => { self.storage_problem.set(Some(error)); false }
        }
    }
}

fn default_draft() -> StockBatchRequest {
    StockBatchRequest { enabled: false, assets: vec![], budget_usdc: "100".into(), keyed: false, interval_secs: 15 }
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct BatchData {
    pub selected: RwSignal<Vec<String>>,
    pub budget: RwSignal<String>,
    pub keyed: RwSignal<bool>,
    pub interval: RwSignal<u32>,
    pub pending: RwSignal<bool>,
    pub problem: RwSignal<Option<String>>,
    pub storage_problem: RwSignal<Option<String>>,
    pub journal: OperationJournal,
    pub recheck: Callback<()>,
    pub refresh: Callback<()>,
    pub reset_draft: Callback<()>,
    pub conflict: Memo<bool>,
    pub reconcile: Callback<bool>,
    pub apply: Callback<bool>,
}

pub(super) fn use_batch(runtime: BatchRuntime, market: RwSignal<LoadState<StockMarketSnapshot>>,
    catalog: RwSignal<LoadState<StockCatalog>>) -> BatchData {
    let journal = runtime.journal;
    let configuration = Memo::new(move |_| market.with(|s| s.value()
        .map(|s| (s.batch.request.clone(), s.batch.revision.clone()))));
    let conflict = Memo::new(move |_| market.with(|s| s.value().is_some_and(|s|
        !s.batch.revision.is_empty() && runtime.revision.get() != s.batch.revision)));
    Effect::new(move |_| {
        let Some((config, revision)) = configuration.get() else { return; };
        let awaiting_receipt = journal.pending.with(Option::is_some);
        if !runtime.initialized.get_value() {
            if let Some(config) = &config {
                runtime.set_draft(config);
            } else if let Some(assets) = catalog.with(|c| c.value().map(|c| c.rows.iter()
                .filter(|s| identity::backpack_issuer(s).is_ok()).take(STOCK_BATCH_LIMIT)
                .map(|s| s.asset.clone()).collect::<Vec<_>>())) {
                runtime.selected.set(assets);
            } else { return; }
            runtime.initialized.set_value(true);
            runtime.revision.set(revision);
        } else if !runtime.dirty() && !awaiting_receipt {
            if let Some(config) = &config {
                runtime.set_draft(config);
            }
            runtime.revision.set(revision);
        }
        runtime.applied.set_value(config);
        runtime.persist();
    });
    let refresh = Callback::new(move |_| spawn_local(read_current(runtime, market)));
    Effect::new(move |_| { journal.connection.track(); refresh.run(()); });
    let recheck = journal.recheck(Callback::new(move |run: shared_types::ActionRun| {
        runtime.problem.set((run.status != ActionRunStatus::Succeeded)
            .then(|| format!("上次保存未成功：{}", run.problem.map_or(run.message, |p| p.message))));
        // Receipts describe history; read current quotes/config separately.
        refresh.run(());
    }));
    let reset_draft = Callback::new(move |_| {
        match storage::remove(&runtime.scope.get_value()) {
            Ok(()) => {
                runtime.storage_problem.set(None);
                let saved = market.with_untracked(|s| s.value().and_then(|s| s.batch.request.clone()));
                runtime.initialized.set_value(saved.is_some());
                runtime.set_draft(saved.as_ref().unwrap_or(&default_draft()));
                runtime.applied.set_value(saved);
                runtime.revision.set(market.with_untracked(|s| s.value().map(|s|s.batch.revision.clone()).unwrap_or_default()));
                runtime.persist();
            }
            Err(error) => runtime.storage_problem.set(Some(error)),
        }
    });
    let reconcile = Callback::new(move |keep_draft: bool| {
        if runtime.pending.get_untracked() { return; }
        let Some(current) = market.with_untracked(|s|s.value().map(|s|s.batch.clone())) else { return; };
        if !keep_draft { runtime.set_draft(current.request.as_ref().unwrap_or(&default_draft())); }
        runtime.applied.set_value(current.request);
        runtime.revision.set(current.revision);
        runtime.initialized.set_value(true);
        runtime.problem.set(None);
        runtime.persist();
    });
    let apply = Callback::new(move |enabled| {
        if runtime.pending.get_untracked() || journal.locked() || !runtime.persist() { return; }
        if enabled && conflict.get_untracked() { return; }
        let request = if enabled { runtime.draft(true) } else {
            let Some(mut saved) = market.with_untracked(|s| s.value().and_then(|s| s.batch.request.clone())) else { return; };
            saved.enabled = false; saved
        };
        let expected_revision = if enabled { runtime.revision.get_untracked() } else {
            market.with_untracked(|s|s.value().map(|s|s.batch.revision.clone()).unwrap_or_default())
        };
        let Some(attempt) = journal.begin(ActionRunKind::StockBatchUpdate, "stocks-batch".into()) else { return; };
        let epoch = journal.epoch.get_untracked();
        let client = journal.client();
        runtime.problem.set(None);
        spawn_local(async move {
            let update = StockBatchUpdateRequest { request: request.clone(), expected_revision };
            let result = client.batch_stock(&update, &attempt.context).await.and_then(|snapshot| {
                validate_setting_response(&attempt, &snapshot)?;
                if snapshot.batch.request.as_ref() != Some(&request) || snapshot.batch.revision.is_empty() {
                    return Err(ApiError::client("STOCK_BATCH_RECEIPT_MISMATCH", "原参数处理结果不匹配，需核对原操作"));
                }
                Ok(snapshot)
            });
            if !journal.current(epoch) { return; }
            match result {
                Ok(snapshot) => { runtime.revision.set(snapshot.batch.revision); journal.resolve(&attempt); }
                Err(error) => { runtime.problem.set(Some(error.problem.message.clone())); journal.failed(&attempt, &error); }
            }
            journal.busy.set(false);
            read_current(runtime, market).await;
        });
    });
    BatchData { selected: runtime.selected, budget: runtime.budget, keyed: runtime.keyed,
        interval: runtime.interval, pending: runtime.pending, problem: runtime.problem,
        storage_problem: runtime.storage_problem, journal, recheck, refresh, reset_draft, conflict, reconcile, apply }
}

async fn read_current(runtime: BatchRuntime, market: RwSignal<LoadState<StockMarketSnapshot>>) {
    if runtime.reading.get_untracked() { return; }
    runtime.reading.set(true);
    let epoch = runtime.journal.epoch.get_untracked();
    let client = runtime.journal.client();
    let result = with_mutation_timeout("读取股票当前监控", client.stock_market_snapshot()).await;
    if !runtime.journal.current(epoch) { return; }
    match result {
        Ok(snapshot) => super::apply_snapshot(market, snapshot),
        Err(error) => { market.try_update(|s| s.apply_result(Err(error.problem))); }
    }
    runtime.reading.set(false);
}

#[cfg(test)]
impl BatchData {
    pub fn fixture() -> Self {
        Self { selected: RwSignal::new(vec!["MU.US".into()]), budget: RwSignal::new("100".into()),
            keyed: RwSignal::new(false), interval: RwSignal::new(15), pending: RwSignal::new(false),
            problem: RwSignal::new(None), storage_problem: RwSignal::new(None),
            journal: OperationJournal::fixture("stocks-batch"),
            conflict: Memo::new(|_|false), reconcile: Callback::new(|_| {}),
            recheck: Callback::new(|_| {}), refresh: Callback::new(|_| {}),
            reset_draft: Callback::new(|_| {}), apply: Callback::new(|_| {}) }
    }
}
