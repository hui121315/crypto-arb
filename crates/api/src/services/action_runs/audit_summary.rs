use super::*;

mod integrations;

use integrations::{
    automation_result_summary, trading_status_result_summary, webhook_result_summary,
};

pub(super) fn action_result_summary(run: &ActionRun) -> Option<serde_json::Value> {
    let result = run.result.as_ref()?;
    match run.kind {
        ActionRunKind::TradingRiskConfigUpdate | ActionRunKind::TradingAdapterSelect => {
            trading_status_result_summary(result)
        }
        ActionRunKind::TradingFeeSnapshotUpsert => fee_snapshot_result_summary(result),
        ActionRunKind::TradingOrderSubmit | ActionRunKind::TradingOrderCancel => {
            order_record_result_summary(result)
        }
        ActionRunKind::TradingOrderReconcile => order_reconcile_result_summary(result),
        ActionRunKind::TradingKillSwitch => kill_switch_result_summary(result),
        ActionRunKind::AutomationConfigUpdate
        | ActionRunKind::AutomationControl
        | ActionRunKind::AutomationLiveUnlock => automation_result_summary(result),
        ActionRunKind::HedgeConfirm => hedge_confirm_result_summary(result),
        ActionRunKind::WebhookConfigUpdate => webhook_result_summary(result),
        ActionRunKind::WebhookTest => Some(json!({
            "eventId": result.get("eventId"),
            "queued": result.get("queued"),
        })),
        ActionRunKind::MarketSubscriptionsUpdate => market_subscriptions_result_summary(result),
        ActionRunKind::GateCrossExModeUpdate => gate_crossex_result_summary(result),
        ActionRunKind::StockPlanBuild | ActionRunKind::StockPeerPlanBuild => Some(json!({
            "planId":result.get("planId"), "asset":result.pointer("/request/asset"),
            "phase":result.get("phase"), "observedAtMs":result.get("observedAtMs"),
        })),
        ActionRunKind::StockMonitorUpdate => Some(json!({
            "asset": result.get("asset"), "enabled": result.get("enabled"),
            "webhookEnabled": result.pointer("/alerts/enabled"),
            "observedAtMs": result.get("observedAtMs"),
        })),
        ActionRunKind::StockBatchUpdate => Some(json!({
            "enabled": result.pointer("/batch/request/enabled"),
            "assetCount": result.pointer("/batch/request/assets").and_then(serde_json::Value::as_array).map(Vec::len),
            "observedAtMs": result.get("observedAtMs"),
        })),
        ActionRunKind::OnchainComparisonConfigUpdate => Some(json!({
            "enabled": result.pointer("/config/enabled"),
            "chain": result.pointer("/config/chain"),
            "provider": result.pointer("/config/provider"),
            "observedAtMs": result.get("observedAtMs"),
        })),
        ActionRunKind::OnchainBatchAdd | ActionRunKind::OnchainBatchRemove => Some(json!({
            "itemCount": result.get("items").and_then(serde_json::Value::as_array).map(Vec::len),
            "observedAtMs": result.get("observedAtMs"),
        })),
        ActionRunKind::PortfolioClosePosition
        | ActionRunKind::PortfolioClosePair
        | ActionRunKind::PortfolioCloseAll
        | ActionRunKind::PortfolioCloseCompensation
        | ActionRunKind::PortfolioCloseManualTerminal => close_run_result_summary(result),
        ActionRunKind::VenueCredentialsUpdate
        | ActionRunKind::VenueCredentialsClear
        | ActionRunKind::VenueCredentialsMigrate
        | ActionRunKind::OnchainProviderCredentialsUpdate
        | ActionRunKind::OnchainProviderCredentialsClear => {
            venue_credentials_result_summary(result)
        }
    }
}

fn gate_crossex_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "runtimeState": result.get("runtimeState"),
        "selectedCount": result.get("selectedCount"),
        "liveCount": result.get("liveCount"),
        "config": result.get("config"),
    }))
}

fn market_subscriptions_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "updatedAtMs": result.get("updatedAtMs"),
        "venues": result.get("venues"),
    }))
}

fn fee_snapshot_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "venue": result.get("venue"),
        "provider": result.get("provider"),
        "symbol": result.get("symbol"),
        "product": result.get("product"),
        "source": result.get("source"),
        "validUntilMs": result.get("validUntilMs"),
        "evidenceId": result.pointer("/evidence/evidenceId"),
    }))
}

fn order_record_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "internalOrderId": result.pointer("/intent/id"),
        "clientOrderId": result.pointer("/intent/clientOrderId"),
        "exchange": result.pointer("/intent/exchange"),
        "symbol": result.pointer("/intent/symbol"),
        "mode": result.pointer("/intent/mode"),
        "state": result.get("state"),
        "exchangeOrderId": result.get("exchangeOrderId"),
    }))
}

fn order_reconcile_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    let diffs = result.as_array()?;
    Some(json!({
        "diffCount": diffs.len(),
        "kinds": diffs.iter().filter_map(|diff| diff.get("kind").cloned()).collect::<Vec<_>>(),
    }))
}

fn kill_switch_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "active": result.pointer("/summary/active"),
        "previousActive": result.pointer("/summary/previousActive"),
        "openOrderCount": result.pointer("/summary/openOrderCount"),
        "expectedOpenOrderCount": result.pointer("/summary/expectedOpenOrderCount"),
        "reason": result.pointer("/summary/reason"),
        "actionRunId": result.get("actionRunId"),
    }))
}

fn venue_credentials_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "venue": result.get("venue"),
        "operation": result.get("operation"),
        "configuredCount": result.get("configuredCount"),
        "fieldCount": result.get("fieldCount"),
        "affectedFields": result.get("affectedFields"),
        "missingFields": result.get("missingFields"),
        "secretStorage": {
            "mode": result.pointer("/secretStorage/mode"),
            "persistent": result.pointer("/secretStorage/persistent"),
            "encrypted": result.pointer("/secretStorage/encrypted"),
        },
        "actionRunId": result.get("actionRunId"),
    }))
}

fn hedge_confirm_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    let run = result.get("executionRun")?;
    Some(json!({
        "idempotencyKey": result.get("idempotencyKey"),
        "status": result.get("status"),
        "executionRun": {
            "runId": run.get("runId"),
            "ticketId": run.get("ticketId"),
            "opportunityId": run.get("opportunityId"),
            "state": run.get("state"),
            "statusReason": run.get("statusReason"),
            "netExposureUsd": run.get("netExposureUsd"),
            "longLeg": leg_summary(run.get("longLeg")),
            "shortLeg": leg_summary(run.get("shortLeg")),
            "finalityProblem": problem_summary(run.get("finalityProblem")),
            "valuationProblem": problem_summary(run.get("valuationProblem")),
            "unwindProblem": problem_summary(run.get("unwindProblem")),
            "costReconciliation": execution_cost_summary(run.get("costReconciliation")),
        }
    }))
}

fn close_run_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    let cost = result.get("costReconciliation")?;
    Some(json!({
        "closeRunId": result.get("id"),
        "status": result.get("status"),
        "costReconciliation": {
            "fundingUsd": cost.get("fundingUsd"),
            "manualHandlingUsd": cost.get("manualHandlingUsd"),
            "totalActualCostUsd": cost.get("totalActualCostUsd"),
            "evidenceOrderIds": cost.get("evidenceOrderIds"),
            "evidenceEventIds": cost.get("evidenceEventIds"),
            "fundingEventIds": cost.get("fundingEventIds"),
            "manualHandlingEventIds": cost.get("manualHandlingEventIds"),
            "missingFields": cost.get("missingFields"),
        }
    }))
}

fn execution_cost_summary(cost: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let cost = cost?;
    Some(json!({
        "actualOpenCostUsd": cost.get("actualOpenCostUsd"),
        "actualFundingUsd": cost.get("actualFundingUsd"),
        "fundingEventIds": cost.get("fundingEventIds"),
        "actualUnwindFeeUsd": cost.get("actualUnwindFeeUsd"),
        "actualUnwindSlippageUsd": cost.get("actualUnwindSlippageUsd"),
        "actualUnwindCostUsd": cost.get("actualUnwindCostUsd"),
        "unwindEventIds": cost.get("unwindEventIds"),
        "missingFields": cost.get("missingFields"),
        "actualCostUsd": cost.get("actualCostUsd"),
        "costDeltaUsd": cost.get("costDeltaUsd"),
    }))
}

fn leg_summary(leg: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let leg = leg?;
    Some(json!({
        "exchange": leg.get("exchange"),
        "symbol": leg.get("symbol"),
        "state": leg.get("state"),
        "orderIds": leg.get("orderIds"),
    }))
}

fn problem_summary(problem: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let problem = problem?;
    Some(json!({
        "code": problem.get("code"),
        "status": problem.get("status"),
        "retryAfterMs": problem.get("retryAfterMs"),
    }))
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn audit_detail_summarizes_trading_mutation_payloads() {
        let risk_run = test_run(
            ActionRunKind::TradingRiskConfigUpdate,
            json!({
                "adapter": "mock",
                "environment": "paper",
                "openOrderCount": 2,
                "risk": {
                    "liveTradingEnabled": false,
                    "killSwitchActive": true,
                    "maxOrderNotional": 1000.0,
                    "maxOpenOrders": 5,
                    "allowedExchanges": ["mock"],
                    "allowedSymbols": ["BTCUSDT"],
                }
            }),
        );
        let fee_run = test_run(
            ActionRunKind::TradingFeeSnapshotUpsert,
            json!({
                "venue": "binance",
                "symbol": "BTCUSDT",
                "product": "perp",
                "source": "official_schedule",
                "validUntilMs": 2000,
                "evidence": { "evidenceId": "fee-binance-perp" },
            }),
        );
        let reconcile_run = test_run(
            ActionRunKind::TradingOrderReconcile,
            json!([
                { "kind": "remote_missing", "exchangeOrderId": "ex-1" },
                { "kind": "state_mismatch", "exchangeOrderId": "ex-2" }
            ]),
        );

        let risk_summary = summary(&risk_run);
        let fee_summary = summary(&fee_run);
        let reconcile_summary = summary(&reconcile_run);

        assert_eq!(risk_summary["risk"]["allowedSymbols"][0], "BTCUSDT");
        assert_eq!(fee_summary["evidenceId"], "fee-binance-perp");
        assert_eq!(reconcile_summary["diffCount"], 2);
        assert_eq!(reconcile_summary["kinds"][1], "state_mismatch");
    }

    #[test]
    fn credential_maintenance_summary_keeps_safe_operation_evidence() {
        let run = test_run(
            ActionRunKind::VenueCredentialsClear,
            json!({
                "venue": "okx",
                "operation": "clear",
                "affectedFields": ["api_key", "api_secret"],
                "missingFields": ["api_key", "api_secret"],
                "secretStorage": {
                    "mode": "keychain",
                    "persistent": true,
                    "encrypted": true,
                    "path": "service:private"
                },
                "actionRunId": "act-clear"
            }),
        );

        let summary = summary(&run);

        assert_eq!(summary["operation"], "clear");
        assert_eq!(summary["affectedFields"][1], "api_secret");
        assert_eq!(summary["secretStorage"]["encrypted"], true);
        assert!(summary.get("path").is_none());
    }

    fn summary(run: &ActionRun) -> serde_json::Value {
        match action_result_summary(run) {
            Some(summary) => summary,
            None => panic!("missing action result summary for {:?}", run.kind),
        }
    }

    fn test_run(kind: ActionRunKind, result: serde_json::Value) -> ActionRun {
        ActionRun {
            id: "act-test".to_owned(),
            kind,
            status: ActionRunStatus::Succeeded,
            actor: "system".to_owned(),
            target: Some("target".to_owned()),
            request_id: Some("req-test".to_owned()),
            idempotency_key: Some("idem-test".to_owned()),
            message: "done".to_owned(),
            problem: None,
            result: Some(result),
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        }
    }
}
