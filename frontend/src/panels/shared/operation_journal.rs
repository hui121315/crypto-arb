use crate::api::rest::{with_mutation_timeout, ApiClient, ApiError, MutationRequestContext};
use leptos::{prelude::*, task::spawn_local};
use serde::{Deserialize, Serialize};
use shared_types::{
    ActionRun, ActionRunKind, ActionRunStatus, ResourceStatus, TradingStatusResponse,
    VenueCredentialMaintenanceResponse, VenueCredentialUpdateResponse,
};

mod storage;

// Only correlation identifiers, never credential fields, fingerprints or bodies.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingOperation {
    version: u8,
    pub kind: ActionRunKind,
    pub target: String,
    pub context: MutationRequestContext,
    pub run_id: Option<String>,
}

impl PendingOperation {
    fn valid(&self, domain: &str) -> bool {
        let kind_valid = match domain {
            "credentials" => matches!(
                self.kind,
                ActionRunKind::VenueCredentialsUpdate
                    | ActionRunKind::VenueCredentialsClear
                    | ActionRunKind::VenueCredentialsMigrate
            ),
            "environment" => self.kind == ActionRunKind::TradingAdapterSelect,
            "webhook" => {
                self.kind == ActionRunKind::WebhookConfigUpdate && self.target == "webhook-delivery"
            }
            "webhook-test" => {
                self.kind == ActionRunKind::WebhookTest && self.target == "webhook-test"
            }
            "market" => self.kind == ActionRunKind::MarketSubscriptionsUpdate,
            "crossex" => self.kind == ActionRunKind::GateCrossExModeUpdate && self.target == "gate_crossex",
            "stocks-batch" => self.kind == ActionRunKind::StockBatchUpdate && self.target == "stocks-batch",
            "stocks-monitor" => self.kind == ActionRunKind::StockMonitorUpdate,
            "stocks-plan" => self.kind == ActionRunKind::StockPlanBuild,
            "stocks-peer-plan" => self.kind == ActionRunKind::StockPeerPlanBuild,
            "automation" => matches!(self.kind, ActionRunKind::AutomationConfigUpdate | ActionRunKind::AutomationControl)
                && self.target == "automated-arbitrage",
            "onchain-config" => match self.kind {
                ActionRunKind::OnchainComparisonConfigUpdate | ActionRunKind::OnchainBatchAdd =>
                    self.target == "onchain-cex-comparison",
                ActionRunKind::OnchainBatchRemove => true,
                _ => false,
            },
            "risk" => matches!(
                (self.kind, self.target.as_str()),
                (ActionRunKind::TradingRiskConfigUpdate, "risk-config")
                    | (
                        ActionRunKind::TradingKillSwitch,
                        "kill-switch:on" | "kill-switch:off"
                    )
            ),
            "position-close" => matches!(self.kind,
                ActionRunKind::PortfolioClosePosition | ActionRunKind::PortfolioClosePair
                    | ActionRunKind::PortfolioCloseAll),
            "position-remedy" => matches!(self.kind,
                ActionRunKind::PortfolioCloseCompensation | ActionRunKind::PortfolioCloseManualTerminal),
            "position-remedy-cancel" => self.kind == ActionRunKind::TradingOrderCancel,
            _ => false,
        };
        let id = |v: &str| {
            !v.is_empty()
                && v.len() <= 256
                && v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b':' | b'.' | b'_' | b'-'))
        };
        self.version == 1
            && kind_valid
            && (id(&self.target) || (domain == "position-close"
                && !self.target.is_empty() && self.target.len() <= 256
                && self.target.bytes().all(|c| c.is_ascii_alphanumeric()
                    || matches!(c, b':' | b'.' | b'_' | b'-' | b'/'))))
            && id(self.context.request_id())
            && (if matches!(domain, "position-close" | "position-remedy" | "position-remedy-cancel") {
                let prefix = match self.kind {
                    ActionRunKind::PortfolioClosePosition => "positions-single:",
                    ActionRunKind::PortfolioClosePair => "positions-pair:",
                    ActionRunKind::PortfolioCloseCompensation => "positions-compensation:",
                    ActionRunKind::PortfolioCloseManualTerminal => "positions-manual-terminal:",
                    ActionRunKind::TradingOrderCancel => "positions-compensation-cancel:",
                    _ => "positions-all:",
                };
                self.context.idempotency_key().is_some_and(|key|
                    key.starts_with(prefix) && key.len() <= 2048
                        && !key.chars().any(char::is_control))
            } else { self.context.idempotency_key()
                == Some(
                    format!(
                        "settings-{}:{}",
                        self.kind.as_str(),
                        self.context.request_id()
                    )
                    .as_str(),
                ) })
            && self.run_id.as_deref().is_none_or(id)
    }

    fn same_request(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.target == other.target
            && self.context == other.context
            && self
                .run_id
                .as_ref()
                .is_none_or(|id| other.run_id.as_ref() == Some(id))
    }

    fn matches(&self, run: &ActionRun) -> bool {
        run.kind == self.kind
            && run.target.as_deref() == Some(self.target.as_str())
            && run.request_id.as_deref() == Some(self.context.request_id())
            && run.idempotency_key.as_deref() == self.context.idempotency_key()
            && self.run_id.as_ref().is_none_or(|id| id == &run.id)
            && !run.id.is_empty()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OperationJournal {
    pub pending: RwSignal<Option<PendingOperation>>,
    pub busy: RwSignal<bool>,
    pub problem: RwSignal<Option<String>>,
    pub epoch: RwSignal<u64>,
    pub connection: RwSignal<u64>,
    domain: &'static str,
    scope: StoredValue<String>,
    base: RwSignal<String>,
    token: RwSignal<String>,
}

impl OperationJournal {
    #[cfg(test)]
    pub(crate) fn fixture(domain: &'static str) -> Self {
        Self { pending: RwSignal::new(None), busy: RwSignal::new(false), problem: RwSignal::new(None),
            epoch: RwSignal::new(0), connection: RwSignal::new(0), domain,
            scope: StoredValue::new(storage::key(domain, "", "")),
            base: RwSignal::new(String::new()), token: RwSignal::new(String::new()) }
    }

    pub(crate) fn new(domain: &'static str) -> Self {
        let app = expect_context::<crate::state::AppContext>();
        let journal = Self {
            pending: RwSignal::new(None),
            busy: RwSignal::new(false),
            problem: RwSignal::new(None),
            epoch: RwSignal::new(0),
            connection: RwSignal::new(0),
            domain,
            scope: StoredValue::new(storage::key(
                domain,
                &app.api_base.get_untracked(),
                &app.api_auth_token.get_untracked(),
            )),
            base: app.api_base,
            token: app.api_auth_token,
        };
        journal.restore();
        Effect::new(move |_| {
            app.api_base.track();
            app.api_auth_token.track();
            let next = journal.current_key();
            if journal.scope.get_value() == next {
                return;
            }
            journal.scope.set_value(next);
            journal.epoch.update(|epoch| *epoch = epoch.wrapping_add(1));
            journal
                .connection
                .update(|epoch| *epoch = epoch.wrapping_add(1));
            journal.pending.set(None);
            journal.busy.set(false);
            journal.restore();
        });
        journal
    }

    fn current_key(self) -> String {
        storage::key(
            self.domain,
            &self.base.get_untracked(),
            &self.token.get_untracked(),
        )
    }
    pub(crate) fn client(self) -> ApiClient {
        ApiClient::with_base_and_auth(&self.base.get_untracked(), &self.token.get_untracked())
    }
    pub(crate) fn draft_storage_key(self) -> String {
        format!("{}.draft", self.current_key())
    }
    pub(crate) fn locked(self) -> bool {
        self.busy.get() || self.pending.with(Option::is_some) || self.problem.with(Option::is_some)
    }
    pub(crate) fn current(self, epoch: u64) -> bool {
        self.epoch.try_get_untracked() == Some(epoch)
            && self.scope.get_value() == self.current_key()
    }
    pub(crate) fn restore(self) {
        match storage::load(&self.scope.get_value(), self.domain) {
            Ok(attempt) => {
                self.pending.set(attempt);
                self.problem.set(None);
            }
            Err(problem) => self.problem.set(Some(problem)),
        }
    }
    pub(crate) fn restored_state(
        self,
        kinds: &[ActionRunKind],
    ) -> crate::state::action_state::ActionState {
        use crate::state::action_state::ActionState;
        self.pending
            .get_untracked()
            .filter(|attempt| kinds.contains(&attempt.kind))
            .map_or(ActionState::Idle, |attempt| {
                ActionState::failed(
                    "操作结果待核对",
                    shared_types::ApiProblem::new(
                        "SETTINGS_RESULT_UNKNOWN",
                        "已找回原请求编号，请查询原处理结果确认结果",
                    ),
                )
                .with_evidence(
                    attempt
                        .context
                        .evidence()
                        .with_action_run_id(attempt.run_id),
                )
            })
    }
    pub(crate) fn begin(self, kind: ActionRunKind, target: String) -> Option<PendingOperation> {
        self.begin_with_context(kind, target, MutationRequestContext::new_idempotent_attempt(
            format!("settings-{}", kind.as_str()),
        ))
    }
    pub(crate) fn begin_with_context(
        self, kind: ActionRunKind, target: String, context: MutationRequestContext,
    ) -> Option<PendingOperation> {
        if self.busy.get_untracked()
            || self.pending.with_untracked(Option::is_some)
            || self.problem.with_untracked(Option::is_some)
            || self.scope.get_value() != self.current_key()
        {
            return None;
        }
        let attempt = PendingOperation {
            version: 1,
            kind,
            target,
            context,
            run_id: None,
        };
        if !attempt.valid(self.domain) {
            self.problem
                .set(Some("操作身份不完整，未发送请求。".into()));
            return None;
        }
        if let Err(error) = storage::persist(&self.scope.get_value(), self.domain, &attempt, true) {
            self.restore();
            self.problem.set(Some(error));
            return None;
        }
        self.epoch.update(|value| *value = value.wrapping_add(1));
        self.pending.set(Some(attempt.clone()));
        self.busy.set(true);
        self.problem.set(None);
        Some(attempt)
    }
    pub(crate) fn remember_run(self, attempt: &PendingOperation, run_id: String) {
        let mut known = attempt.clone();
        known.run_id = Some(run_id);
        if let Err(error) = storage::persist(&self.scope.get_value(), self.domain, &known, false) {
            self.problem.set(Some(error));
        }
        self.pending.set(Some(known));
    }
    pub(crate) fn resolve(self, attempt: &PendingOperation) -> bool {
        match storage::resolve(&self.scope.get_value(), self.domain, attempt) {
            Ok(()) => {
                self.pending.set(None);
                self.problem.set(None);
                true
            }
            Err(error) => {
                self.problem.set(Some(error));
                false
            }
        }
    }
    pub(crate) fn failed(self, attempt: &PendingOperation, error: &ApiError) {
        if matches!(error.problem.status, Some(400..=499))
            && error.problem.status != Some(408)
            && error.problem.code != shared_types::problem::codes::ACTION_RUN_IN_FLIGHT
        {
            self.resolve(attempt);
        }
        self.busy.set(false);
    }
    pub(crate) fn recheck(self, terminal: Callback<ActionRun>) -> Callback<()> {
        Callback::new(move |()| {
            if self.busy.get_untracked() || self.scope.get_value() != self.current_key() {
                return;
            }
            let Some(attempt) = self.pending.get_untracked() else {
                self.restore();
                return;
            };
            let epoch = self.epoch.get_untracked();
            let client = self.client();
            self.busy.set(true);
            self.problem.set(None);
            spawn_local(async move {
                let result = with_mutation_timeout("查询原操作", lookup(&client, &attempt)).await;
                if !self.current(epoch) {
                    return;
                }
                match result {
                    Ok(run) => {
                        let mut known = attempt;
                        known.run_id = Some(run.id.clone());
                        if let Err(error) =
                            storage::persist(&self.scope.get_value(), self.domain, &known, false)
                        {
                            self.problem.set(Some(error));
                        }
                        self.pending.set(Some(known.clone()));
                        if run.status == ActionRunStatus::Accepted {
                            if self.problem.get_untracked().is_none() {
                                self.problem
                                    .set(Some("后端已受理，尚未完成；请稍后核对。".into()));
                            }
                        } else if run.kind == ActionRunKind::WebhookTest
                            && run.status == ActionRunStatus::Succeeded
                        {
                            // Enqueue is terminal for the action, not for the delivery.
                            terminal.run(run);
                        } else if self.resolve(&known) {
                            terminal.run(run);
                        }
                    }
                    Err(error) => self
                        .problem
                        .set(Some(format!("{} · code {}", error, error.problem.code))),
                }
                self.busy.set(false);
            });
        })
    }
}

async fn lookup(client: &ApiClient, attempt: &PendingOperation) -> Result<ActionRun, ApiError> {
    let run = lookup_operation(client, attempt).await?;
    if run.status == ActionRunStatus::Succeeded {
        if run.problem.is_some() {
            return Err(mismatch());
        }
        let result = run.result.clone().ok_or_else(|| {
            ApiError::client(
                "SETTINGS_RECEIPT_MISSING",
                "成功处理结果缺少结果，不能确认已完成",
            )
        })?;
        validate_result(attempt, result, Some(&run.id))?;
    }
    Ok(run)
}

// Identity lookup only; each funds workflow must validate its typed result and finality.
pub(crate) async fn lookup_operation(client: &ApiClient, attempt: &PendingOperation) -> Result<ActionRun, ApiError> {
    let run = if let Some(id) = &attempt.run_id {
        client.action_run(id).await?
    } else {
        let envelope = client.action_runs_envelope().await?;
        if !matches!(
            envelope.status,
            ResourceStatus::Ready | ResourceStatus::Partial
        ) {
            return Err(ApiError::client(
                "SETTINGS_RECEIPT_UNAVAILABLE",
                "账本暂不可核对，保留原操作",
            ));
        }
        let mut matches = envelope
            .data
            .unwrap_or_default()
            .into_iter()
            .filter(|run| attempt.matches(run));
        let run = matches.next().ok_or_else(|| {
            ApiError::client(
                "SETTINGS_RECEIPT_NOT_FOUND",
                "账本未找到原操作，不代表未执行；请勿重复提交",
            )
        })?;
        if matches.next().is_some() {
            return Err(ApiError::client(
                "SETTINGS_RECEIPT_AMBIGUOUS",
                "原操作存在多个处理结果，结果待核对",
            ));
        }
        run
    };
    if !attempt.matches(&run) {
        return Err(mismatch());
    }
    Ok(run)
}

pub(crate) fn validate_setting_response<T: Serialize>(
    attempt: &PendingOperation,
    response: &T,
) -> Result<(), ApiError> {
    validate_result(
        attempt,
        serde_json::to_value(response).map_err(|_| mismatch())?,
        None,
    )
}

fn validate_result(
    attempt: &PendingOperation,
    result: serde_json::Value,
    run_id: Option<&str>,
) -> Result<(), ApiError> {
    let (target, request, run, extra_valid) = match attempt.kind {
        ActionRunKind::VenueCredentialsUpdate => {
            let r: VenueCredentialUpdateResponse =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            (
                r.venue,
                r.request_id,
                r.action_run_id,
                r.configured_count <= r.field_count,
            )
        }
        ActionRunKind::VenueCredentialsClear | ActionRunKind::VenueCredentialsMigrate => {
            let r: VenueCredentialMaintenanceResponse =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            let expected = if attempt.kind == ActionRunKind::VenueCredentialsClear {
                shared_types::VenueCredentialMaintenanceOperation::Clear
            } else {
                shared_types::VenueCredentialMaintenanceOperation::Migrate
            };
            (
                r.venue,
                r.request_id,
                r.action_run_id,
                r.operation == expected,
            )
        }
        ActionRunKind::WebhookTest => {
            let r: shared_types::WebhookTestResponse =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            let valid = r.queued
                && !r.action_run_id.is_empty()
                && r.event_id == format!("evt-webhook-test-{}", r.action_run_id)
                && r.idempotency_key.as_deref() == attempt.context.idempotency_key();
            (
                "webhook-test".to_owned(),
                r.request_id,
                Some(r.action_run_id),
                valid,
            )
        }
        ActionRunKind::WebhookConfigUpdate => {
            let r: shared_types::WebhookRuntimeStatus =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            return if attempt.target == "webhook-delivery" && r.updated_at_ms > 0 {
                Ok(())
            } else {
                Err(mismatch())
            };
        }
        ActionRunKind::MarketSubscriptionsUpdate => {
            let r: shared_types::MarketSubscriptionsResponse =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            return if r.updated_at_ms > 0
                && r.venues
                    .iter()
                    .filter(|row| row.venue == attempt.target)
                    .count()
                    == 1
            {
                Ok(())
            } else {
                Err(mismatch())
            };
        }
        ActionRunKind::GateCrossExModeUpdate => {
            let r: shared_types::GateCrossExModeSnapshot =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            let selected = r.config.selected_routes.iter().collect::<std::collections::BTreeSet<_>>();
            return if attempt.target == "gate_crossex"
                && r.observed_at_ms > 0
                && r.config.min_gross_spread_pct.is_finite()
                && (0.0..=100.0).contains(&r.config.min_gross_spread_pct)
                && selected.len() == r.config.selected_routes.len()
                && selected.len() <= shared_types::GATE_CROSSEX_SELECTED_ROUTE_LIMIT
                && selected.len() == r.selected_count
                && selected.iter().all(|route| !route.is_empty() && route.trim() == route.as_str())
                && (r.config.mode == shared_types::GateCrossExMode::Disabled)
                    == (r.runtime_state == shared_types::GateCrossExRuntimeState::Disabled)
            {
                Ok(())
            } else {
                Err(mismatch())
            };
        }
        ActionRunKind::StockPlanBuild => {
            let receipt: shared_types::stocks::StockPlanBuildReceipt =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            return if receipt.valid_for(&attempt.target) { Ok(()) } else { Err(mismatch()) };
        }
        ActionRunKind::StockPeerPlanBuild => {
            let receipt: shared_types::stocks::StockPeerPlanBuildReceipt =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            return if receipt.valid_for(&attempt.target) { Ok(()) } else { Err(mismatch()) };
        }
        ActionRunKind::StockMonitorUpdate => {
            let receipt: shared_types::stocks::StockMonitorReceipt =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            return if receipt.valid_for(&attempt.target) { Ok(()) } else { Err(mismatch()) };
        }
        ActionRunKind::StockBatchUpdate => {
            let snapshot: shared_types::stocks::StockMarketSnapshot =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            return if attempt.target == "stocks-batch" && snapshot.observed_at_ms > 0
                && snapshot.batch.request.is_some() { Ok(()) } else { Err(mismatch()) };
        }
        ActionRunKind::OnchainComparisonConfigUpdate => {
            let snapshot = serde_json::from_value(result).map_err(|_| mismatch())?;
            validate_onchain_snapshot(&snapshot)?;
            return if attempt.target == "onchain-cex-comparison" { Ok(()) } else { Err(mismatch()) };
        }
        ActionRunKind::AutomationConfigUpdate | ActionRunKind::AutomationControl => {
            let snapshot = serde_json::from_value(result).map_err(|_| mismatch())?;
            validate_automation_snapshot(&snapshot)?;
            return if attempt.target == "automated-arbitrage" { Ok(()) } else { Err(mismatch()) };
        }
        ActionRunKind::OnchainBatchAdd | ActionRunKind::OnchainBatchRemove => {
            let batch: shared_types::OnchainBatchSnapshot =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            let ids = batch.items.iter().map(|item| &item.item_id).collect::<std::collections::BTreeSet<_>>();
            let valid = batch.observed_at_ms > 0 && batch.max_items > 0
                && batch.items.len() <= batch.max_items && ids.len() == batch.items.len()
                && batch.items.iter().all(|item| !item.item_id.trim().is_empty()
                    && item.observed_at_ms > 0 && valid_onchain_config(&item.config));
            let target_valid = if attempt.kind == ActionRunKind::OnchainBatchAdd {
                attempt.target == "onchain-cex-comparison" && !batch.items.is_empty()
            } else { !ids.iter().any(|id| id.as_str() == attempt.target) };
            return if valid && target_valid { Ok(()) } else { Err(mismatch()) };
        }
        ActionRunKind::TradingKillSwitch => {
            let r: shared_types::KillSwitchResponse =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            let target = if r.summary.active {
                "kill-switch:on"
            } else {
                "kill-switch:off"
            };
            let valid = r.summary.active == r.status.risk.kill_switch_active
                && r.idempotency_key.as_deref() == attempt.context.idempotency_key();
            (target.to_owned(), r.request_id, r.action_run_id, valid)
        }
        ActionRunKind::TradingAdapterSelect | ActionRunKind::TradingRiskConfigUpdate => {
            let r: TradingStatusResponse =
                serde_json::from_value(result).map_err(|_| mismatch())?;
            let key_valid = r
                .idempotency_key
                .as_deref()
                .is_none_or(|key| Some(key) == attempt.context.idempotency_key());
            let target = if attempt.kind == ActionRunKind::TradingRiskConfigUpdate {
                "risk-config".to_owned()
            } else {
                r.adapter
            };
            (target, r.request_id, r.action_run_id, key_valid)
        }
        _ => return Err(mismatch()),
    };
    if !extra_valid
        || target != attempt.target
        || request.as_deref() != Some(attempt.context.request_id())
        || run_id.is_some_and(|id| run.as_deref() != Some(id))
    {
        return Err(mismatch());
    }
    Ok(())
}

fn mismatch() -> ApiError {
    ApiError::client(
        "SETTINGS_RECEIPT_MISMATCH",
        "处理结果身份或结果不匹配，仍需核对原操作",
    )
}

pub(crate) fn validate_onchain_snapshot(snapshot: &shared_types::OnchainComparisonSnapshot) -> Result<(), ApiError> {
    if snapshot.observed_at_ms > 0 && valid_onchain_config(&snapshot.config) {
        Ok(())
    } else { Err(mismatch()) }
}

pub(crate) fn validate_automation_snapshot(snapshot: &shared_types::AutomationRuntimeStatus) -> Result<(), ApiError> {
    let config = &snapshot.config;
    if snapshot.updated_at_ms > 0
        && [config.capital_usd, config.leverage, config.min_one_cycle_net_bps, config.min_depth_usd]
            .into_iter().all(|value| value.is_finite() && value > 0.0)
        && (1..=8).contains(&config.max_concurrent_runs)
        && (shared_types::MIN_AUTOMATION_ENTRY_COOLDOWN_SECS..=86_400).contains(&config.cooldown_secs) {
        Ok(())
    } else { Err(mismatch()) }
}

fn valid_onchain_config(config: &shared_types::OnchainComparisonConfig) -> bool {
    let amount = |value: &str| !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit())
        && value.parse::<u128>().is_ok_and(|n| n > 0);
    [&config.chain, &config.provider, &config.cex_venue, &config.cex_symbol,
        &config.base_token, &config.quote_token, &config.base_mint, &config.quote_mint]
        .iter().all(|value| !value.trim().is_empty())
        && amount(&config.base_amount_raw) && amount(&config.quote_amount_raw)
        && config.max_age_ms > 0
        && [config.cex_taker_fee_bps, config.gas_usd, config.slippage_bps, config.min_liquidity_usd]
            .into_iter().all(|value| value.is_finite() && value >= 0.0)
}

pub(crate) fn settings_recovery_panel(
    journal: OperationJournal,
    recheck: Callback<()>,
) -> impl IntoView {
    view! {
        <Show when=move || journal.pending.with(Option::is_some) || journal.problem.with(Option::is_some)>
            <div class="provider-credentials-feedback provider-credentials-recovery has-action" role="alert" aria-label="设置操作待核对">
                <span class="provider-credentials-recovery-copy">
                    <strong>{move || journal.pending.with(|pending| pending.as_ref().map_or_else(|| "恢复记录不可用，已暂停修改".into(), |attempt| {
                        let label = match attempt.kind { ActionRunKind::VenueCredentialsUpdate => "保存凭证", ActionRunKind::VenueCredentialsClear => "清除凭证", ActionRunKind::VenueCredentialsMigrate => "迁移凭证", ActionRunKind::WebhookConfigUpdate => "更新 Webhook 配置", ActionRunKind::MarketSubscriptionsUpdate => "更新行情订阅", ActionRunKind::GateCrossExModeUpdate => "保存 CrossEx 配置", ActionRunKind::StockBatchUpdate => "保存股票批量监控", ActionRunKind::AutomationConfigUpdate => "保存自动化配置", ActionRunKind::AutomationControl => "更新自动化启停", ActionRunKind::OnchainComparisonConfigUpdate => "保存链上配置", ActionRunKind::OnchainBatchAdd => "加入批量监控", ActionRunKind::OnchainBatchRemove => "移除批量市场", ActionRunKind::TradingRiskConfigUpdate => "保存风控", ActionRunKind::TradingKillSwitch => "切换总闸", _ => "切换执行环境" };
                        let label = if attempt.kind == ActionRunKind::StockMonitorUpdate { "保存股票单股监控" } else { label };
                        let label = if attempt.kind == ActionRunKind::StockPlanBuild { "构建股票计划" } else { label };
                        let label = if attempt.kind == ActionRunKind::StockPeerPlanBuild { "构建股票双边计划" } else { label };
                        format!("{} · {label}{}", attempt.target, if journal.busy.get() { "处理中" } else { "结果待核对" })
                    }))}</strong>
                    <span>"核对只查询原处理结果，不会重新提交。"</span>
                    {move || journal.problem.get().map(|message| view! { <span>{message}</span> })}
                </span>
                <button type="button" class="row-action" disabled=move || journal.busy.get() on:click=move |_| recheck.run(())>
                    {move || if journal.busy.get() { "核对中…" } else if journal.domain == "environment" { "核对上次切换" } else { "核对上次操作" }}
                </button>
            </div>
        </Show>
    }
}
