use serde_json::Value;
use shared_types::{problem::codes, ActionRun, ActionRunKind, ActionRunStatus, ApiProblem,
    KillSwitchResponse, MarketSubscriptionsResponse, TradingStatusResponse, WebhookRuntimeStatus};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// 从 durable audit JSONL 恢复 `ActionRun` 的幂等边界。
///
/// 仅恢复白名单内不含凭证的配置回执；其他动作仍不恢复任意 response payload，
/// 缺少回执不得重新执行外部副作用。
pub(crate) fn replay_action_runs(path: Option<&str>) -> io::Result<Vec<ActionRun>> {
    let Some(raw_path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(Vec::new());
    };
    let path = Path::new(raw_path);
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut latest = BTreeMap::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: Value = serde_json::from_str(line).map_err(|error| {
            invalid_data(format!(
                "audit log line {} is not valid JSON: {error}",
                index + 1
            ))
        })?;
        let Some(snapshot) = entry
            .get("detail")
            .and_then(Value::as_object)
            .and_then(|detail| detail.get("actionRun"))
        else {
            continue;
        };
        let mut run: ActionRun = serde_json::from_value(snapshot.clone()).map_err(|error| {
            invalid_data(format!(
                "audit log line {} has malformed actionRun snapshot: {error}",
                index + 1
            ))
        })?;
        if run.id.trim().is_empty() {
            return Err(invalid_data(format!(
                "audit log line {} has an empty actionRun identity",
                index + 1
            )));
        }
        validate_correlation(&entry, &run, index + 1)?;
        run.result = configuration_receipt(&run);
        let replace = latest
            .get(&run.id)
            .is_none_or(|current: &ActionRun| run.updated_at_ms >= current.updated_at_ms);
        if replace {
            latest.insert(run.id.clone(), run);
        }
    }
    Ok(latest.into_values().map(recover_interrupted_run).collect())
}

/// Typed round-trip strips unknown fields and only retains correlated, non-secret config receipts.
pub(crate) fn configuration_receipt(run: &ActionRun) -> Option<Value> {
    if run.status != ActionRunStatus::Succeeded {
        return None;
    }
    let payload = run.result.as_ref()?.clone();
    match run.kind {
        ActionRunKind::TradingRiskConfigUpdate | ActionRunKind::TradingAdapterSelect => {
            let status: TradingStatusResponse = serde_json::from_value(payload).ok()?;
            matching_status_receipt(run, &status).then(|| serde_json::to_value(status).ok())?
        }
        ActionRunKind::TradingKillSwitch => {
            let response: KillSwitchResponse = serde_json::from_value(payload).ok()?;
            if !matching_status_receipt(run, &response.status)
                || response.action_run_id.as_deref() != Some(run.id.as_str())
                || response.request_id != run.request_id
                || response.idempotency_key != run.idempotency_key {
                return None;
            }
            serde_json::to_value(response).ok()
        }
        ActionRunKind::WebhookConfigUpdate => {
            let mut status: WebhookRuntimeStatus = serde_json::from_value(payload).ok()?;
            if run.target.as_deref() != Some("webhook-delivery") || status.updated_at_ms <= 0
                || status.configuration_problem.is_some() {
                return None;
            }
            // Recovery proves a saved configuration, not historical delivery or its destination.
            status.config.url = if status.config.url_configured {
                "已配置（地址已隐藏）".into()
            } else {
                String::new()
            };
            status.recent_deliveries.clear();
            serde_json::to_value(status).ok()
        }
        ActionRunKind::MarketSubscriptionsUpdate => {
            let mut response: MarketSubscriptionsResponse = serde_json::from_value(payload).ok()?;
            let target = run.target.as_deref()?;
            if response.updated_at_ms <= 0
                || response.venues.iter().filter(|row| row.venue == target).count() != 1
            {
                return None;
            }
            // Live connection evidence is read afresh after resolving the original operation.
            response.runtime.clear();
            serde_json::to_value(response).ok()
        }
        ActionRunKind::StockPlanBuild => {
            let response: shared_types::stocks::StockPlanBuildReceipt = serde_json::from_value(payload).ok()?;
            if !response.valid_for(run.target.as_deref()?) { return None; }
            serde_json::to_value(response).ok()
        }
        ActionRunKind::StockPeerPlanBuild => {
            let response: shared_types::stocks::StockPeerPlanBuildReceipt = serde_json::from_value(payload).ok()?;
            if !response.valid_for(run.target.as_deref()?) { return None; }
            serde_json::to_value(response).ok()
        }
        ActionRunKind::StockMonitorUpdate => {
            let response: shared_types::stocks::StockMonitorReceipt = serde_json::from_value(payload).ok()?;
            if !response.valid_for(run.target.as_deref()?) { return None; }
            serde_json::to_value(response).ok()
        }
        ActionRunKind::StockBatchUpdate => {
            let response: shared_types::stocks::StockMarketSnapshot = serde_json::from_value(payload).ok()?;
            if run.target.as_deref() != Some("stocks-batch") || response.observed_at_ms <= 0
                || response.batch.request.is_none() {
                return None;
            }
            // Persist configuration only; quotes, RFQ and account/settlement data are not receipts.
            serde_json::to_value(shared_types::stocks::StockMarketSnapshot {
                observed_at_ms: response.observed_at_ms,
                batch: shared_types::stocks::StockBatchStatus {
                    request: response.batch.request,
                    revision: response.batch.revision,
                    ..Default::default()
                },
                ..Default::default()
            }).ok()
        }
        _ => None,
    }
}

fn matching_status_receipt(run: &ActionRun, status: &TradingStatusResponse) -> bool {
    status.action_run_id.as_deref() == Some(run.id.as_str())
        && status.request_id == run.request_id
        && status.idempotency_key == run.idempotency_key
}

fn validate_correlation(entry: &Value, run: &ActionRun, line_number: usize) -> io::Result<()> {
    validate_optional_identity(entry, "actionRunId", Some(run.id.as_str()), line_number)?;
    validate_optional_identity(entry, "requestId", run.request_id.as_deref(), line_number)?;
    validate_optional_identity(
        entry,
        "idempotencyKey",
        run.idempotency_key.as_deref(),
        line_number,
    )
}

fn validate_optional_identity(
    entry: &Value,
    field: &str,
    expected: Option<&str>,
    line_number: usize,
) -> io::Result<()> {
    let Some(actual) = entry.get(field) else {
        return Ok(());
    };
    let actual = actual.as_str().ok_or_else(|| {
        invalid_data(format!(
            "audit log line {line_number} has a non-string {field} correlation"
        ))
    })?;
    if Some(actual) == expected {
        return Ok(());
    }
    Err(invalid_data(format!(
        "audit log line {line_number} has mismatched {field} correlation"
    )))
}

fn recover_interrupted_run(mut run: ActionRun) -> ActionRun {
    if run.status != ActionRunStatus::Accepted {
        return run;
    }
    run.status = ActionRunStatus::Failed;
    run.message =
        "action run was interrupted by a previous process; reconcile before retry".to_owned();
    let mut problem = ApiProblem::new(
        codes::ACTION_RUN_REPLAY_UNAVAILABLE,
        "action run was accepted before restart and its terminal outcome is unknown",
    )
    .with_status(409)
    .with_request_id(run.request_id.clone());
    problem.details = Some(serde_json::json!({
        "reason": "restart_in_flight",
        "originalStatus": "accepted",
        "actionRunId": run.id,
        "idempotencyKey": run.idempotency_key,
    }));
    run.problem = Some(problem);
    run
}

fn invalid_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;
    use serde_json::json;
    use shared_types::{ActionMutationChange, ActionMutationDiff, ActionRunKind};
    use tempfile::tempdir;

    #[test]
    fn replay_turns_interrupted_accepted_run_into_fail_closed_terminal() {
        let directory = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let path = directory.path().join("audit.jsonl");
        let run = ActionRun {
            id: "act-restart".to_owned(),
            kind: ActionRunKind::TradingOrderSubmit,
            status: ActionRunStatus::Accepted,
            actor: "api-token:operator:0123456789abcdef".to_owned(),
            target: Some("client-1".to_owned()),
            request_id: Some("req-1".to_owned()),
            idempotency_key: Some("client-1".to_owned()),
            message: "accepted".to_owned(),
            problem: None,
            result: Some(json!({ "sensitive": "not-replayed" })),
            mutation: Some(ActionMutationDiff {
                effective_at_ms: 2,
                changes: vec![ActionMutationChange::LiveTradingEnabled {
                    before: false,
                    after: true,
                }],
            }),
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        let line = json!({ "detail": { "actionRun": run } });
        std::fs::write(&path, format!("{line}\n"))
            .unwrap_or_else(|error| panic!("write failed: {error}"));

        let runs =
            replay_action_runs(path.to_str()).unwrap_or_else(|error| panic!("replay: {error}"));

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, ActionRunStatus::Failed);
        assert_eq!(
            runs[0]
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(codes::ACTION_RUN_REPLAY_UNAVAILABLE)
        );
        assert!(runs[0].result.is_none());
        assert!(runs[0].mutation.as_ref().is_some_and(|mutation| {
            matches!(
                mutation.changes.as_slice(),
                [ActionMutationChange::LiveTradingEnabled {
                    before: false,
                    after: true
                }]
            )
        }));
    }

    #[test]
    fn replay_rejects_malformed_action_snapshot() {
        let directory = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let path = directory.path().join("audit.jsonl");
        std::fs::write(&path, "{\"detail\":{\"actionRun\":{\"id\":1}}}\n")
            .unwrap_or_else(|error| panic!("write failed: {error}"));

        let error = replay_action_runs(path.to_str()).expect_err("malformed snapshot must fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn replay_rejects_top_level_request_or_action_identity_drift() {
        let directory = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let path = directory.path().join("audit.jsonl");
        let run = ActionRun {
            id: "act-correlated".to_owned(),
            kind: ActionRunKind::TradingOrderSubmit,
            status: ActionRunStatus::Succeeded,
            actor: "api-token:operator:0123456789abcdef".to_owned(),
            target: Some("client-1".to_owned()),
            request_id: Some("req-correlated".to_owned()),
            idempotency_key: Some("idem-correlated".to_owned()),
            message: "submitted".to_owned(),
            problem: None,
            result: None,
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        let line = json!({
            "actionRunId": "act-other",
            "requestId": "req-correlated",
            "idempotencyKey": "idem-correlated",
            "detail": { "actionRun": run }
        });
        std::fs::write(&path, format!("{line}\n"))
            .unwrap_or_else(|error| panic!("write failed: {error}"));

        let error = replay_action_runs(path.to_str())
            .expect_err("mismatched top-level correlation must fail closed");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("mismatched actionRunId"));
    }

    #[test]
    fn replay_accepts_matching_top_level_correlation() {
        let directory = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let path = directory.path().join("audit.jsonl");
        let run = ActionRun {
            id: "act-correlated".to_owned(),
            kind: ActionRunKind::TradingOrderSubmit,
            status: ActionRunStatus::Succeeded,
            actor: "api-token:operator:0123456789abcdef".to_owned(),
            target: Some("client-1".to_owned()),
            request_id: Some("req-correlated".to_owned()),
            idempotency_key: Some("idem-correlated".to_owned()),
            message: "submitted".to_owned(),
            problem: None,
            result: None,
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        let line = json!({
            "actionRunId": "act-correlated",
            "requestId": "req-correlated",
            "idempotencyKey": "idem-correlated",
            "detail": { "actionRun": run }
        });
        std::fs::write(&path, format!("{line}\n"))
            .unwrap_or_else(|error| panic!("write failed: {error}"));

        let runs = replay_action_runs(path.to_str())
            .unwrap_or_else(|error| panic!("matching correlation failed: {error}"));

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "act-correlated");
        assert_eq!(runs[0].request_id.as_deref(), Some("req-correlated"));
    }

    #[test]
    fn replay_keeps_non_idempotent_action_run_for_audit_visibility() {
        let directory = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let path = directory.path().join("audit.jsonl");
        let run = ActionRun {
            id: "act-reconcile".to_owned(),
            kind: ActionRunKind::TradingOrderReconcile,
            status: ActionRunStatus::Succeeded,
            actor: "api-token:operator:0123456789abcdef".to_owned(),
            target: Some("open-orders".to_owned()),
            request_id: Some("req-reconcile".to_owned()),
            idempotency_key: None,
            message: "reconciled".to_owned(),
            problem: None,
            result: None,
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        let line = json!({ "detail": { "actionRun": run } });
        std::fs::write(&path, format!("{line}\n"))
            .unwrap_or_else(|error| panic!("write failed: {error}"));

        let runs =
            replay_action_runs(path.to_str()).unwrap_or_else(|error| panic!("replay: {error}"));

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "act-reconcile");
        assert!(runs[0].idempotency_key.is_none());
        assert_eq!(runs[0].status, ActionRunStatus::Succeeded);
    }
}
