//! Settings 动作结果的成功/校验文案派生（纯函数，无副作用）。

use shared_types::{
    ActionMutationDiff, KillSwitchResponse, SecretStorageMode, TradingStatusResponse,
    VenueCredentialMaintenanceOperation, VenueCredentialMaintenanceResponse,
    VenueCredentialProbeStatus, VenueCredentialUpdateResponse, VenueCredentialValidationEvidence,
    VenueCredentialValidationStatus,
};

pub(in crate::panels::modules::settings) fn credential_success_message(
    response: &VenueCredentialUpdateResponse,
) -> String {
    let mut message = response.message.clone();
    if let Some(evidence) = response.validation_evidence.as_ref() {
        message.push_str(" · 验证 ");
        message.push_str(credential_validation_label(evidence.status));
        if let Some(read_summary) = credential_read_probe_summary(evidence) {
            message.push_str(" · ");
            message.push_str(&read_summary);
        }
    }
    message.push_str(" · Secret ");
    message.push_str(secret_storage_label(response.secret_storage.mode));
    if !response.secret_storage.encrypted {
        message.push_str(" 未加密");
    }
    message
}

pub(in crate::panels::modules::settings) fn credential_maintenance_success_message(
    response: &VenueCredentialMaintenanceResponse,
    idempotency_key: &str,
) -> String {
    let operation = match response.operation {
        VenueCredentialMaintenanceOperation::Clear => "已清空",
        VenueCredentialMaintenanceOperation::Migrate => "已迁移",
    };
    let mut message = format!("{operation} {}", response.affected_fields.join("/"));
    if !response.missing_fields.is_empty() {
        message.push_str(" · 缺失 ");
        message.push_str(&response.missing_fields.join("/"));
    }
    message.push_str(" · ");
    message.push_str(&response.message);
    if let Some(action_run_id) = response.action_run_id.as_deref() {
        message.push_str(" · Action ");
        message.push_str(action_run_id);
    }
    if let Some(request_id) = response.request_id.as_deref() {
        message.push_str(" · Request ");
        message.push_str(request_id);
    }
    message.push_str(" · Secret ");
    message.push_str(secret_storage_label(response.secret_storage.mode));
    message.push_str(" · Idempotency ");
    message.push_str(idempotency_key);
    message
}

fn secret_storage_label(mode: SecretStorageMode) -> &'static str {
    match mode {
        SecretStorageMode::EnvFileAtomic => ".env 原子写入",
        SecretStorageMode::Keychain => "系统 Keychain 加密",
        SecretStorageMode::RuntimeOnly => "仅进程内缓存",
    }
}

pub(in crate::panels::modules::settings) fn kill_switch_success_message(
    response: &KillSwitchResponse,
) -> String {
    let state = if response.summary.active {
        "Kill Switch 已开启"
    } else {
        "Kill Switch 已关闭"
    };
    let mut message = format!(
        "{state} · 原因 {} · 挂单 {}",
        response.summary.reason, response.summary.open_order_count
    );
    if let Some(action_run_id) = response.action_run_id.as_deref() {
        message.push_str(" · Action ");
        message.push_str(action_run_id);
    }
    if let Some(request_id) = response.request_id.as_deref() {
        message.push_str(" · Request ");
        message.push_str(request_id);
    }
    if let Some(idempotency_key) = response.idempotency_key.as_deref() {
        message.push_str(" · Idempotency ");
        message.push_str(idempotency_key);
    }
    message
}

pub(in crate::panels::modules::settings) fn risk_config_success_message(
    response: &TradingStatusResponse,
    fallback_idempotency_key: &str,
) -> String {
    let mut message = String::from("风控参数已生效");
    append_mutation_count(&mut message, response.mutation.as_ref());
    if let Some(action_run_id) = response.action_run_id.as_deref() {
        message.push_str(" · Action ");
        message.push_str(action_run_id);
    }
    if let Some(request_id) = response.request_id.as_deref() {
        message.push_str(" · Request ");
        message.push_str(request_id);
    }
    message.push_str(" · Idempotency ");
    message.push_str(
        response
            .idempotency_key
            .as_deref()
            .unwrap_or(fallback_idempotency_key),
    );
    message
}

fn append_mutation_count(message: &mut String, mutation: Option<&ActionMutationDiff>) {
    if let Some(mutation) = mutation.filter(|mutation| !mutation.is_empty()) {
        message.push_str(" · 变更 ");
        message.push_str(&mutation.changes.len().to_string());
        message.push_str(" 项");
    }
}

fn credential_read_probe_summary(evidence: &VenueCredentialValidationEvidence) -> Option<String> {
    let passed = credential_probe_labels(evidence, VenueCredentialProbeStatus::Ok);
    let failed = credential_probe_labels(evidence, VenueCredentialProbeStatus::Failed);
    let unknown = credential_probe_labels(evidence, VenueCredentialProbeStatus::Unknown);
    if passed.is_empty() && failed.is_empty() && unknown.is_empty() {
        return None;
    }
    let mut summary = String::from("私有读");
    let mut wrote = false;
    if !passed.is_empty() {
        summary.push_str("通过：");
        summary.push_str(&passed.join("/"));
        wrote = true;
    }
    if !failed.is_empty() {
        if wrote {
            summary.push('；');
        }
        summary.push_str("失败：");
        summary.push_str(&failed.join("/"));
        wrote = true;
    }
    if !unknown.is_empty() {
        if wrote {
            summary.push('；');
        }
        summary.push_str("未验证：");
        summary.push_str(&unknown.join("/"));
    }
    Some(summary)
}

fn credential_probe_labels(
    evidence: &VenueCredentialValidationEvidence,
    status: VenueCredentialProbeStatus,
) -> Vec<&'static str> {
    let mut labels = evidence
        .probes
        .iter()
        .filter(|probe| probe.status == status)
        .filter_map(|probe| credential_probe_label(&probe.kind))
        .collect::<Vec<_>>();
    labels.sort_unstable();
    labels.dedup();
    labels
}

fn credential_probe_label(kind: &str) -> Option<&'static str> {
    match kind {
        "account_mode_read" => Some("账户模式"),
        "balance_read" => Some("余额"),
        "open_orders_read" => Some("挂单"),
        "order_permission" => Some("订单权限"),
        "positions_read" => Some("持仓"),
        _ => None,
    }
}

fn credential_validation_label(status: VenueCredentialValidationStatus) -> &'static str {
    match status {
        VenueCredentialValidationStatus::ReadOnlyOk => "只读接口通过",
        VenueCredentialValidationStatus::LocalOnly => "本地格式通过",
        VenueCredentialValidationStatus::Unknown => "证据未知",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_switch_success_message_keeps_action_context() {
        let message = kill_switch_success_message(&shared_types::KillSwitchResponse {
            status: kill_switch_status(true),
            summary: shared_types::KillSwitchSummary {
                previous_active: false,
                active: true,
                open_order_count: 0,
                expected_open_order_count: Some(0),
                reason: "positions.kill_switch.enable".into(),
                checked_at_ms: 10,
            },
            action_run_id: Some("act-1".into()),
            request_id: Some("req-1".into()),
            idempotency_key: Some("idem-1".into()),
        });

        assert!(message.contains("Kill Switch 已开启"));
        assert!(message.contains("act-1"));
        assert!(message.contains("req-1"));
        assert!(message.contains("idem-1"));
    }

    #[test]
    fn risk_config_success_message_keeps_receipt_and_diff_count() {
        let mut status = kill_switch_status(false);
        status.action_run_id = Some("act-risk-1".into());
        status.request_id = Some("req-risk-1".into());
        status.idempotency_key = Some("idem-risk-1".into());
        status.mutation = Some(ActionMutationDiff {
            effective_at_ms: 10,
            changes: vec![shared_types::ActionMutationChange::MaxOpenOrders {
                before: 3,
                after: 5,
            }],
        });

        let message = risk_config_success_message(&status, "fallback");

        assert!(message.contains("变更 1 项"));
        assert!(message.contains("act-risk-1"));
        assert!(message.contains("req-risk-1"));
        assert!(message.contains("idem-risk-1"));
        assert!(!message.contains("fallback"));
    }

    fn kill_switch_status(active: bool) -> shared_types::TradingStatusResponse {
        shared_types::TradingStatusResponse {
            adapter: "mock".into(),
            environment: shared_types::ExecutionEnvironment::Paper,
            open_order_count: 0,
            risk: shared_types::TradingRiskStatus {
                live_trading_enabled: false,
                kill_switch_active: active,
                max_order_notional: 0.0,
                max_open_orders: 0,
                max_hedge_imbalance_pct: 0.0,
                liquidation_warn_pct: 0.0,
                liquidation_danger_pct: 0.0,
                allowed_exchanges: Vec::new(),
                allowed_symbols: Vec::new(),
                protected_positions: Vec::new(),
                auto_profit_close: Default::default(),
            },
            ws_channels: shared_types::TradingWsChannels {
                orders: "orders".into(),
                execution: "execution".into(),
                risk_alerts: "risk_alerts".into(),
            },
            action_run_id: None,
            request_id: None,
            idempotency_key: None,
            mutation: None,
        }
    }
}
