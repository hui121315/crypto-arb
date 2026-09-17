//! 凭证字段、保存期验证与运行态可用性的状态文案。

use super::*;
use shared_types::credential_matrix::{
    CredentialProbeLink, CredentialProbeMatrix, CredentialReadiness,
};
use shared_types::{
    VenueCredentialFieldSource, VenueCredentialValidationEvidence, VenueCredentialValidationStatus,
};

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn credential_field_label(
    configured: bool,
) -> &'static str {
    if configured {
        "字段已填写"
    } else {
        "字段未填写"
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn credential_field_source_label(
    source: VenueCredentialFieldSource,
) -> &'static str {
    match source {
        VenueCredentialFieldSource::Missing => "缺失",
        VenueCredentialFieldSource::Environment => "环境变量",
        VenueCredentialFieldSource::EnvFile => ".env",
        VenueCredentialFieldSource::Keychain => "Keychain",
        VenueCredentialFieldSource::Runtime => "进程内",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn secret_storage_health_label(
    health: SecretStorageHealth,
) -> &'static str {
    match health {
        SecretStorageHealth::Ready => "可用",
        SecretStorageHealth::Degraded => "降级",
        SecretStorageHealth::Unavailable => "不可用",
        SecretStorageHealth::Unknown => "状态未知",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn secret_storage_health_class(
    health: SecretStorageHealth,
) -> &'static str {
    match health {
        SecretStorageHealth::Ready => "status-pill ready",
        SecretStorageHealth::Degraded | SecretStorageHealth::Unknown => "status-pill pending",
        SecretStorageHealth::Unavailable => "status-pill blocked",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn operation_status_label(
    status: VenueOperationStatus,
) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "正常",
        VenueOperationStatus::Warn => "降级",
        VenueOperationStatus::Blocked => "阻断",
        VenueOperationStatus::Unknown => "待验证",
        VenueOperationStatus::Unsupported => "不支持",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn operation_status_class(
    status: VenueOperationStatus,
) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "status-pill ready",
        VenueOperationStatus::Warn | VenueOperationStatus::Unknown => "status-pill pending",
        VenueOperationStatus::Blocked => "status-pill blocked",
        VenueOperationStatus::Unsupported => "status-pill",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn trading_runtime_summary(
    venue_id: &str,
    evidence: &TradingRuntimeEvidence,
) -> String {
    let ready = trading_runtime_ready_count(evidence);
    let attention = trading_runtime_attention_count(evidence);
    let total = trading_runtime_rows(evidence).len();
    format!("{venue_id} · 当前运行态 {ready}/{total} 正常 · {attention}/{total} 待处理")
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn trading_runtime_status_label(
    evidence: &TradingRuntimeEvidence,
) -> &'static str {
    if trading_runtime_ready_count(evidence) == trading_runtime_rows(evidence).len() {
        return "当前可用";
    }
    if trading_runtime_rows(evidence)
        .into_iter()
        .flatten()
        .any(has_explicit_unavailability)
    {
        return "当前不可用";
    }
    if trading_runtime_rows(evidence)
        .into_iter()
        .flatten()
        .any(|row| row.status == VenueOperationStatus::Warn)
    {
        return "当前状态降级";
    }
    "当前状态未验证"
}

fn has_explicit_unavailability(row: &VenueOperationHealth) -> bool {
    matches!(
        row.status,
        VenueOperationStatus::Blocked | VenueOperationStatus::Unsupported
    ) || row.supported == Some(false)
        || row.configured == Some(false)
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn trading_runtime_status_class(
    evidence: &TradingRuntimeEvidence,
) -> &'static str {
    match trading_runtime_status_label(evidence) {
        "当前可用" => "status-pill ready",
        "当前不可用" => "status-pill blocked",
        _ => "status-pill pending",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn selected_summary(
    row: Option<&VenueCredentialStatus>,
) -> String {
    let Some(row) = row else {
        return "等待规格".into();
    };
    let required_total = row.fields.iter().filter(|field| field.required).count();
    let required_configured = row
        .fields
        .iter()
        .filter(|field| field.required && field.configured)
        .count();
    let optional_total = row.fields.len().saturating_sub(required_total);
    let optional_configured = row
        .fields
        .iter()
        .filter(|field| !field.required && field.configured)
        .count();
    let configured = required_configured;
    let write_support = if row.live_write {
        "静态写侧声明"
    } else {
        "静态未声明写侧"
    };
    let validation = credential_validation_summary(row.validation_evidence.as_ref());
    let missing = if row.missing_fields.is_empty() {
        "字段完整".to_owned()
    } else {
        format!("缺 {} 项", row.missing_fields.len())
    };
    if optional_total == 0 {
        format!(
            "{configured}/{} 字段已填写 / {missing} / {validation} / 当前状态待运行态证据 / {write_support} / {}",
            row.fields.len(),
            row.note
        )
    } else {
        format!(
            "{configured}/{required_total} 必填字段已填写 / {optional_configured}/{optional_total} 可选字段已填写 / {missing} / {validation} / 当前状态待运行态证据 / {write_support} / {}",
            row.note
        )
    }
}

fn credential_validation_summary(evidence: Option<&VenueCredentialValidationEvidence>) -> String {
    let Some(evidence) = evidence else {
        return "未验证".to_owned();
    };
    let status = validation_status_label(evidence.status);
    match evidence.readiness() {
        CredentialReadiness::LiveReady => format!("{status} / 已验证（保存期）"),
        CredentialReadiness::Blocked => {
            format!(
                "{status} / 未验证（权限阻断）: {}",
                blocking_link_labels(evidence)
            )
        }
        CredentialReadiness::Incomplete => {
            format!(
                "{status} / 未验证（缺少探针）: {}",
                blocking_link_labels(evidence)
            )
        }
    }
}

fn blocking_link_labels(evidence: &VenueCredentialValidationEvidence) -> String {
    let labels = evidence
        .blocking_links()
        .into_iter()
        .map(credential_link_label)
        .collect::<Vec<_>>();
    if labels.is_empty() {
        "-".to_owned()
    } else {
        labels.join("/")
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn credential_link_label(
    link: CredentialProbeLink,
) -> &'static str {
    match link {
        CredentialProbeLink::BalanceRead => "余额",
        CredentialProbeLink::PositionsRead => "持仓",
        CredentialProbeLink::OpenOrdersRead => "挂单",
        CredentialProbeLink::OrderPermission => "订单权限",
        CredentialProbeLink::AccountModeRead => "账户模式",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn validation_status_label(
    status: VenueCredentialValidationStatus,
) -> &'static str {
    match status {
        VenueCredentialValidationStatus::ReadOnlyOk => "只读验证",
        VenueCredentialValidationStatus::LocalOnly => "本地格式检查",
        VenueCredentialValidationStatus::Unknown => "验证证据未知",
    }
}
