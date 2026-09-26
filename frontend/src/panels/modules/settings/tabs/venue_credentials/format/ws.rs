//! 交易 WS 能力与提交门禁的显示文案。

use super::*;

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn ws_chip(
    label: &'static str,
    operation: ExchangeWsOperation,
    show_submit_gate: bool,
) -> impl IntoView {
    let class_name = format!("ws-cap {}", status_class(operation.status));
    let status = ws_status_label(&operation);
    let submit_gate = show_submit_gate.then(|| ws_submit_gate_label(&operation));
    let transport = ws_transport_label(&operation, show_submit_gate);
    let name = operation.operation.unwrap_or_else(|| "-".to_string());
    let note = operation.note;
    let evidence = operation
        .evidence
        .map(|evidence| {
            let parser = evidence.parser_test.unwrap_or_else(|| "-".to_owned());
            format!(
                "{} · {} · {} · {} · {}",
                release_status_label(evidence.release_status),
                authenticated_runtime_evidence_label(
                    evidence.requires_authenticated_runtime_evidence,
                    evidence.authenticated_runtime_evidence,
                ),
                evidence.checked_at,
                evidence.auth_kind,
                parser
            )
        })
        .unwrap_or_else(|| "数据依据未登记".to_owned());
    view! {
        <div class=class_name>
            <span>{label}</span>
            <strong>{status}</strong>
            <em>{operation.product} " / " {name}</em>
            <em>{transport}</em>
            <em>{note}</em>
            <em>{evidence}</em>
            {submit_gate.map(|gate| view! { <em>{gate}</em> })}
        </div>
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn ws_transport_label(
    operation: &ExchangeWsOperation,
    write_operation: bool,
) -> &'static str {
    if !write_operation {
        return "WS 实时流；REST 仅冷启动、断线补洞与历史查询";
    }
    if operation.is_live_submittable() {
        return "WS 提交主路径；受理确认 不确定时仅按 client id 对账，禁止 REST 重放";
    }
    if operation.evidence.as_ref().is_some_and(|evidence| {
        evidence.release_status == ExchangeWsReleaseStatus::ProductionReady
            && evidence.requires_authenticated_runtime_evidence
            && !evidence.authenticated_runtime_evidence
    }) {
        return "REST 单次提交；官方 WS 已发布但等待认证运行数据依据";
    }
    "REST 单次提交；WS 写路径未获生产授权"
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn ws_status_label(
    operation: &ExchangeWsOperation,
) -> &'static str {
    if operation.evidence.as_ref().is_some_and(|evidence| {
        evidence.requires_authenticated_runtime_evidence && !evidence.authenticated_runtime_evidence
    }) {
        "缺认证运行数据依据"
    } else if operation
        .evidence
        .as_ref()
        .is_some_and(|evidence| evidence.release_status == ExchangeWsReleaseStatus::BetaUnavailable)
    {
        "Beta 不可用"
    } else {
        status_label(operation.status)
    }
}

fn release_status_label(status: ExchangeWsReleaseStatus) -> &'static str {
    match status {
        ExchangeWsReleaseStatus::Unknown => "发布状态未知",
        ExchangeWsReleaseStatus::ProductionReady => "官方 schema 已发布",
        ExchangeWsReleaseStatus::BetaUnavailable => "Beta 禁止生产",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn authenticated_runtime_evidence_label(
    required: bool,
    authenticated: bool,
) -> &'static str {
    match (required, authenticated) {
        (true, true) => "认证运行数据依据已验证",
        (true, false) => "认证运行数据依据缺失",
        (false, _) => "无额外认证运行数据依据执行条件",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn ws_submit_gate_label(
    operation: &ExchangeWsOperation,
) -> &'static str {
    if operation.is_live_submittable() {
        "live writer 允许提交"
    } else {
        "live writer 禁止提交"
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn status_label(
    status: ExchangeWsSupportStatus,
) -> &'static str {
    match status {
        ExchangeWsSupportStatus::Ready => "静态实现",
        ExchangeWsSupportStatus::RequiresPermission => "需权限",
        ExchangeWsSupportStatus::SchemaPending => "待核准",
        ExchangeWsSupportStatus::RestOnly => "REST",
    }
}

pub(in crate::panels::modules::settings::tabs::venue_credentials) fn status_class(
    status: ExchangeWsSupportStatus,
) -> &'static str {
    match status {
        ExchangeWsSupportStatus::Ready => "ready",
        ExchangeWsSupportStatus::RequiresPermission => "permission",
        ExchangeWsSupportStatus::SchemaPending => "pending",
        ExchangeWsSupportStatus::RestOnly => "rest",
    }
}
