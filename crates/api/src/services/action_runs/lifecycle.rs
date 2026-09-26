use super::audit_log::record_audit;
use super::mutate::prune;
use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ActionRunStart {
    pub kind: ActionRunKind,
    pub actor: String,
    pub target: Option<String>,
    pub idempotency_key: Option<String>,
    pub message: String,
}

impl ActionRunStart {
    pub(crate) fn new(
        kind: ActionRunKind,
        headers: &HeaderMap,
        target: impl Into<Option<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            actor: audit::extract_actor(headers),
            target: target.into(),
            idempotency_key: None,
            message: message.into(),
        }
    }

    pub(crate) fn with_idempotency_key(mut self, key: Option<String>) -> Self {
        self.idempotency_key = key;
        self
    }
}

pub(crate) fn begin(state: &AppState, start: ActionRunStart) -> Result<ActionRun, AppError> {
    let now = common::time::now_ms();
    let run = build_run(format!("act-{}", Uuid::new_v4()), start, now);
    record_audit(&run, "accepted")?;
    state.action_runs().insert(run.id.clone(), run.clone());
    prune(state);
    Ok(run)
}

pub(crate) enum ActionRunBegin {
    Started(ActionRun),
    Replayed(ActionRun),
}

impl ActionRunBegin {
    pub(crate) fn run(&self) -> &ActionRun {
        match self {
            Self::Started(run) | Self::Replayed(run) => run,
        }
    }

    pub(crate) fn is_replayed(&self) -> bool {
        matches!(self, Self::Replayed(_))
    }
}

pub(crate) fn begin_idempotent(
    state: &AppState,
    mut start: ActionRunStart,
) -> Result<ActionRunBegin, AppError> {
    let Some(key) = clean_key(start.idempotency_key.as_deref()) else {
        return begin(state, start).map(ActionRunBegin::Started);
    };
    start.idempotency_key = Some(key.clone());
    let now = common::time::now_ms();
    let id = idempotent_run_id(start.kind, &key);
    let run = build_run(id.clone(), start, now);
    match state.action_runs().entry(id) {
        Entry::Occupied(entry) => Ok(ActionRunBegin::Replayed(entry.get().clone())),
        Entry::Vacant(entry) => {
            record_audit(&run, "accepted")?;
            entry.insert(run.clone());
            prune(state);
            Ok(ActionRunBegin::Started(run))
        }
    }
}

#[cfg(test)]
pub(super) fn find_by_idempotency_key(
    state: &AppState,
    kind: ActionRunKind,
    key: &str,
) -> Option<ActionRun> {
    let key = clean_key(Some(key))?;
    state
        .action_runs()
        .iter()
        .find(|entry| {
            entry.kind == kind
                && entry
                    .idempotency_key
                    .as_deref()
                    .is_some_and(|item| item == key)
        })
        .map(|entry| entry.value().clone())
}

fn build_run(id: String, start: ActionRunStart, now: i64) -> ActionRun {
    ActionRun {
        id,
        kind: start.kind,
        status: ActionRunStatus::Accepted,
        actor: start.actor,
        target: start.target,
        request_id: common::request_id::current(),
        idempotency_key: start.idempotency_key,
        message: start.message,
        problem: None,
        result: None,
        mutation: None,
        started_at_ms: now,
        updated_at_ms: now,
    }
}

fn clean_key(key: Option<&str>) -> Option<String> {
    key.map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
}

fn idempotent_run_id(kind: ActionRunKind, key: &str) -> String {
    format!("act-{}-{:016x}", action_slug(kind), stable_hash(key))
}

fn action_slug(kind: ActionRunKind) -> &'static str {
    match kind {
        ActionRunKind::TradingRiskConfigUpdate => "trading-risk-config-update",
        ActionRunKind::TradingAdapterSelect => "trading-adapter-select",
        ActionRunKind::TradingKillSwitch => "trading-kill-switch",
        ActionRunKind::TradingFeeSnapshotUpsert => "trading-fee-snapshot-upsert",
        ActionRunKind::TradingOrderSubmit => "trading-order-submit",
        ActionRunKind::TradingOrderCancel => "trading-order-cancel",
        ActionRunKind::TradingOrderReconcile => "trading-order-reconcile",
        ActionRunKind::AutomationConfigUpdate => "automation-config-update",
        ActionRunKind::AutomationControl => "automation-control",
        ActionRunKind::AutomationLiveUnlock => "automation-live-unlock",
        ActionRunKind::HedgeConfirm => "hedge-confirm",
        ActionRunKind::WebhookConfigUpdate => "webhook-config-update",
        ActionRunKind::WebhookTest => "webhook-test",
        ActionRunKind::MarketSubscriptionsUpdate => "market-subscriptions-update",
        ActionRunKind::GateCrossExModeUpdate => "gate-crossex-mode-update",
        ActionRunKind::StockBatchUpdate => "stock-batch-update",
        ActionRunKind::StockMonitorUpdate => "stock-monitor-update",
        ActionRunKind::StockPlanBuild => "stock-plan-build",
        ActionRunKind::StockPeerPlanBuild => "stock-peer-plan-build",
        ActionRunKind::OnchainComparisonConfigUpdate => "onchain-comparison-config-update",
        ActionRunKind::OnchainBatchAdd => "onchain-batch-add",
        ActionRunKind::OnchainBatchRemove => "onchain-batch-remove",
        ActionRunKind::VenueCredentialsUpdate => "venue-credentials-update",
        ActionRunKind::VenueCredentialsClear => "venue-credentials-clear",
        ActionRunKind::VenueCredentialsMigrate => "venue-credentials-migrate",
        ActionRunKind::OnchainProviderCredentialsUpdate => "onchain-provider-credentials-update",
        ActionRunKind::OnchainProviderCredentialsClear => "onchain-provider-credentials-clear",
        ActionRunKind::PortfolioClosePosition => "portfolio-close-position",
        ActionRunKind::PortfolioClosePair => "portfolio-close-pair",
        ActionRunKind::PortfolioCloseAll => "portfolio-close-all",
        ActionRunKind::PortfolioCloseCompensation => "portfolio-close-compensation",
        ActionRunKind::PortfolioCloseManualTerminal => "portfolio-close-manual-terminal",
    }
}

pub(crate) fn explicit_idempotency_key(headers: &HeaderMap) -> Option<String> {
    ["idempotency-key", "x-idempotency-key"]
        .into_iter()
        .find_map(|name| headers.get(name))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn stable_hash(key: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn recent(state: &AppState) -> Vec<ActionRun> {
    let mut runs: Vec<ActionRun> = state
        .action_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    runs.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.id.cmp(&left.id))
    });
    runs.truncate(RECENT_ACTION_RUNS);
    runs
}

pub(crate) fn recent_envelope(state: &AppState) -> shared_types::ActionRunEnvelope {
    let total = state.action_runs().len();
    let rows = recent(state);
    let coverage = shared_types::ResourceCoverage::new(total, rows.len());
    let (status, problems) = if coverage.truncated {
        (
            shared_types::ResourceStatus::Partial,
            vec![ApiProblem::new(
                "ACTION_RUN_HISTORY_BOUNDED",
                format!("showing {} of {total} retained action runs", rows.len()),
            )
            .with_source("action-run-registry")],
        )
    } else {
        (shared_types::ResourceStatus::Ready, Vec::new())
    };
    shared_types::ResourceEnvelope::with_data(
        rows,
        status,
        "action-run-registry",
        common::time::now_ms(),
        problems,
    )
    .with_coverage(coverage)
}

pub(crate) fn get(state: &AppState, id: &str) -> Option<ActionRun> {
    state
        .action_runs()
        .get(id.trim())
        .map(|entry| entry.value().clone())
}
