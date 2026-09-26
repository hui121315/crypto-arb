use super::audit_context::action_event_context;
use super::audit_summary::action_result_summary;
use super::*;
use axum::http::StatusCode;
use shared_types::problem::codes;

pub(super) fn record_audit(run: &ActionRun, outcome: &'static str) -> Result<(), AppError> {
    let event = AuditEvent::now(
        run.actor.as_str(),
        audit_action(run.kind),
        run.target.as_deref().unwrap_or("global"),
        outcome,
        audit_detail(run, outcome)?,
    )
    .with_context(action_event_context(run))
    .with_correlation(action_correlation(run));
    audit::record_durable(&event).map_err(|reason| audit_durability_error(run, outcome, reason))
}

fn action_correlation(run: &ActionRun) -> audit::AuditCorrelation {
    let summary = action_result_summary(run);
    let mut order_ids = Vec::new();
    let mut run_ids = Vec::new();
    if let Some(summary) = summary.as_ref() {
        match run.kind {
            ActionRunKind::TradingOrderSubmit | ActionRunKind::TradingOrderCancel => {
                collect_string(summary, "/internalOrderId", &mut order_ids);
                collect_string(summary, "/clientOrderId", &mut order_ids);
                collect_string(summary, "/exchangeOrderId", &mut order_ids);
            }
            ActionRunKind::HedgeConfirm => {
                collect_strings(summary, "/executionRun/longLeg/orderIds", &mut order_ids);
                collect_strings(summary, "/executionRun/shortLeg/orderIds", &mut order_ids);
                collect_string(summary, "/executionRun/runId", &mut run_ids);
            }
            ActionRunKind::PortfolioClosePosition
            | ActionRunKind::PortfolioClosePair
            | ActionRunKind::PortfolioCloseAll
            | ActionRunKind::PortfolioCloseCompensation
            | ActionRunKind::PortfolioCloseManualTerminal => {
                collect_strings(
                    summary,
                    "/costReconciliation/evidenceOrderIds",
                    &mut order_ids,
                );
                collect_string(summary, "/closeRunId", &mut run_ids);
            }
            ActionRunKind::TradingRiskConfigUpdate
            | ActionRunKind::TradingAdapterSelect
            | ActionRunKind::TradingKillSwitch
            | ActionRunKind::TradingFeeSnapshotUpsert
            | ActionRunKind::TradingOrderReconcile
            | ActionRunKind::AutomationConfigUpdate
            | ActionRunKind::AutomationControl
            | ActionRunKind::AutomationLiveUnlock
            | ActionRunKind::WebhookConfigUpdate
            | ActionRunKind::WebhookTest
            | ActionRunKind::MarketSubscriptionsUpdate
            | ActionRunKind::GateCrossExModeUpdate
            | ActionRunKind::StockBatchUpdate
            | ActionRunKind::StockMonitorUpdate
            | ActionRunKind::StockPlanBuild
            | ActionRunKind::StockPeerPlanBuild
            | ActionRunKind::OnchainComparisonConfigUpdate
            | ActionRunKind::OnchainBatchAdd
            | ActionRunKind::OnchainBatchRemove
            | ActionRunKind::VenueCredentialsUpdate
            | ActionRunKind::VenueCredentialsClear
            | ActionRunKind::VenueCredentialsMigrate
            | ActionRunKind::OnchainProviderCredentialsUpdate
            | ActionRunKind::OnchainProviderCredentialsClear => {}
        }
    }

    audit::AuditCorrelation::request(run.request_id.clone())
        .with_action_run_id(run.id.clone())
        .with_idempotency_key(run.idempotency_key.clone())
        .with_order_ids(order_ids)
        .with_run_ids(run_ids)
}

fn collect_string(value: &serde_json::Value, pointer: &str, target: &mut Vec<String>) {
    if let Some(value) = value.pointer(pointer).and_then(serde_json::Value::as_str) {
        target.push(value.to_owned());
    }
}

fn collect_strings(value: &serde_json::Value, pointer: &str, target: &mut Vec<String>) {
    let Some(values) = value.pointer(pointer).and_then(serde_json::Value::as_array) else {
        return;
    };
    target.extend(
        values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
}

fn audit_detail(run: &ActionRun, outcome: &'static str) -> Result<serde_json::Value, AppError> {
    let snapshot = durable_snapshot(run, outcome)?;
    Ok(json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
        "status": to_json_value(&run.status),
        "message": run.message,
        "problem": run.problem.as_ref().map(problem_detail),
        "mutation": run.mutation,
        "resultSummary": action_result_summary(run),
        "actionRun": snapshot,
    }))
}

fn durable_snapshot(run: &ActionRun, outcome: &'static str) -> Result<serde_json::Value, AppError> {
    let mut snapshot = run.clone();
    // Arbitrary responses remain process-local; only typed, credential-free config receipts persist.
    snapshot.result = audit::configuration_receipt(run);
    serde_json::to_value(snapshot).map_err(|error| {
        audit_durability_error(
            run,
            outcome,
            format!("action_run_snapshot_encode_failed: {error}"),
        )
    })
}

fn audit_durability_error(
    run: &ActionRun,
    outcome: &'static str,
    reason: impl Into<String>,
) -> AppError {
    AppError::domain(
        StatusCode::SERVICE_UNAVAILABLE,
        codes::AUDIT_STORAGE_WRITE_FAILED,
        "high-risk action audit event was not durably acknowledged",
    )
    .with_details(json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
        "action": audit_action(run.kind),
        "outcome": outcome,
        "reason": reason.into(),
    }))
}

fn problem_detail(problem: &ApiProblem) -> serde_json::Value {
    json!({
        "code": problem.code,
        "status": problem.status,
        "retryAfterMs": problem.retry_after_ms,
    })
}

pub(super) fn to_json_value<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value)
        .unwrap_or_else(|_| serde_json::Value::String("encode_failed".into()))
}

pub(super) fn audit_outcome(run: &ActionRun) -> &'static str {
    match run.status {
        ActionRunStatus::Accepted => "accepted",
        ActionRunStatus::Succeeded => "success",
        ActionRunStatus::Failed
            if run
                .problem
                .as_ref()
                .and_then(|problem| problem.status)
                .is_some_and(|status| status >= 500) =>
        {
            "error"
        }
        ActionRunStatus::Failed => "denied",
    }
}

pub(super) const fn audit_action(kind: ActionRunKind) -> &'static str {
    match kind {
        ActionRunKind::TradingRiskConfigUpdate => "trading.risk_config.update",
        ActionRunKind::TradingAdapterSelect => "trading.adapter.select",
        ActionRunKind::TradingKillSwitch => "trading.kill_switch.set",
        ActionRunKind::TradingFeeSnapshotUpsert => "trading.fee_snapshot.upsert",
        ActionRunKind::TradingOrderSubmit => "trading.order.submit",
        ActionRunKind::TradingOrderCancel => "trading.order.cancel",
        ActionRunKind::TradingOrderReconcile => "trading.order.reconcile",
        ActionRunKind::AutomationConfigUpdate => "automation.config.update",
        ActionRunKind::AutomationControl => "automation.control",
        ActionRunKind::AutomationLiveUnlock => "automation.live_unlock",
        ActionRunKind::HedgeConfirm => "hedge.confirm",
        ActionRunKind::WebhookConfigUpdate => "webhook.config.update",
        ActionRunKind::WebhookTest => "webhook.test.enqueue",
        ActionRunKind::MarketSubscriptionsUpdate => "market_subscriptions.config.update",
        ActionRunKind::GateCrossExModeUpdate => "gate_crossex.mode.update",
        ActionRunKind::StockBatchUpdate => "backpack_stock.batch.update",
        ActionRunKind::StockMonitorUpdate => "backpack_stock.monitor.update",
        ActionRunKind::StockPlanBuild => "backpack_stock.plan.build",
        ActionRunKind::StockPeerPlanBuild => "stock_peer.plan.build",
        ActionRunKind::OnchainComparisonConfigUpdate => "onchain_comparison.config.update",
        ActionRunKind::OnchainBatchAdd => "onchain_comparison.batch.add",
        ActionRunKind::OnchainBatchRemove => "onchain_comparison.batch.remove",
        ActionRunKind::VenueCredentialsUpdate => "venue_credentials.update",
        ActionRunKind::VenueCredentialsClear => "venue_credentials.clear",
        ActionRunKind::VenueCredentialsMigrate => "venue_credentials.migrate",
        ActionRunKind::OnchainProviderCredentialsUpdate => "onchain.provider_credentials.update",
        ActionRunKind::OnchainProviderCredentialsClear => "onchain.provider_credentials.clear",
        ActionRunKind::PortfolioClosePosition => "portfolio.position.close",
        ActionRunKind::PortfolioClosePair => "portfolio.position.close_pair",
        ActionRunKind::PortfolioCloseAll => "portfolio.positions.close_all",
        ActionRunKind::PortfolioCloseCompensation => "portfolio.close_run.compensate",
        ActionRunKind::PortfolioCloseManualTerminal => "portfolio.close_run.manual_terminal",
    }
}

#[cfg(test)]
mod tests;
