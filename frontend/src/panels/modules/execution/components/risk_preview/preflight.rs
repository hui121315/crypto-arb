use super::*;
use shared_types::MarginPreflightOutcome;

pub(super) fn margin_preflight_summary(preview: &ExecutionPreview) -> String {
    if !preview.is_ready() {
        return "待保证金".into();
    }
    preflight_health_summary(&preview.risk.guards).map_or_else(
        || money(preview.used_capital_usd),
        |summary| format!("{} · {summary}", money(preview.used_capital_usd)),
    )
}

pub(super) fn preflight_health_summary(guards: &[ExecutionGuard]) -> Option<String> {
    let outcomes = guards
        .iter()
        .filter_map(|guard| guard.preflight_outcome.as_ref())
        .collect::<Vec<_>>();
    let total = outcomes.len();
    if total == 0 {
        return None;
    }
    let passed = outcomes
        .iter()
        .filter(|outcome| outcome.status == HedgePreflightStatus::Passed)
        .count();
    let blocked = outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.status,
                HedgePreflightStatus::Blocked | HedgePreflightStatus::Failed
            )
        })
        .count();
    if blocked == 0 {
        Some(format!("预检 {passed}/{total}"))
    } else {
        Some(format!("预检 {passed}/{total} · {blocked} 阻断"))
    }
}

pub(super) fn ticket_venue_availability_summary(preview: &ExecutionPreview) -> String {
    let Some(outcome) = live_ticket_preflight_outcome(preview) else {
        return if preview.execution_mode_label == "模拟" {
            "模拟：不要求实盘双腿运行态".into()
        } else {
            "缺实盘双腿运行态预检".into()
        };
    };
    let venues = ticket_venue_scope_label(&outcome.scope.venues);
    let observed = outcome.observed_venues.len();
    let expected = outcome.scope.venues.len();
    let availability = match outcome.status {
        HedgePreflightStatus::Passed => "可用",
        HedgePreflightStatus::Blocked | HedgePreflightStatus::Failed => "阻断",
        HedgePreflightStatus::Skipped => "待检查",
    };
    format!("双腿 {venues} · {availability} {observed}/{expected}")
}

pub(super) fn ticket_venue_availability_detail(preview: &ExecutionPreview) -> String {
    let Some(outcome) = live_ticket_preflight_outcome(preview) else {
        return "HedgeTicket 未返回实盘双腿运行态预检范围".into();
    };
    let mut parts = vec![
        format!(
            "双腿范围 {}",
            ticket_venue_scope_label(&outcome.scope.venues)
        ),
        format!(
            "已观测 {}",
            ticket_venue_scope_label(&outcome.observed_venues)
        ),
        format!("状态 {:?}", outcome.status),
    ];
    if outcome.checked_at_ms > 0 {
        parts.push(format!("checked {}", outcome.checked_at_ms));
    }
    if let Some(source) = outcome.source.as_deref() {
        parts.push(format!("source {source}"));
    }
    if let Some(freshness_ms) = outcome.freshness_ms {
        parts.push(format!("freshness {freshness_ms}ms"));
    }
    if let Some(request_id) = outcome.request_id.as_deref() {
        parts.push(format!("request {request_id}"));
    }
    if let Some(retry_after_ms) = outcome.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if !outcome.problems.is_empty() {
        let codes = outcome
            .problems
            .iter()
            .map(|problem| problem.code.as_str())
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!("problems {codes}"));
    }
    if !outcome.row_health.is_empty() {
        parts.push(format!(
            "health {}",
            account_data_health_summary(&outcome.row_health)
        ));
    }
    parts.join(" · ")
}

fn live_ticket_preflight_outcome(preview: &ExecutionPreview) -> Option<&MarginPreflightOutcome> {
    preview
        .risk
        .guards
        .iter()
        .find(|guard| guard.key == "live_operation_health")
        .and_then(|guard| guard.preflight_outcome.as_ref())
}

fn ticket_venue_scope_label(venues: &[String]) -> String {
    if venues.is_empty() {
        "缺 venue 范围".into()
    } else {
        venues.join(" / ")
    }
}

pub(super) fn account_preflight_detail(preview: &ExecutionPreview) -> String {
    let details = preview
        .risk
        .guards
        .iter()
        .filter(|guard| guard.preflight_outcome.is_some())
        .map(guard_detail)
        .collect::<Vec<_>>();
    if details.is_empty() {
        "等待后端返回账户/保证金预检证据".into()
    } else {
        details.join(" / ")
    }
}

pub(super) fn positions_evidence_summary(preview: &ExecutionPreview) -> String {
    if !preview.is_ready() {
        return "待持仓".into();
    }
    if preview.execution_mode_label == "模拟" {
        return "模拟无需实盘证据".into();
    }
    preview
        .liquidation
        .positions_evidence
        .as_ref()
        .map_or_else(|| "未返回持仓证据".into(), positions_evidence_line)
}

pub(super) fn positions_evidence_detail(preview: &ExecutionPreview) -> String {
    if preview.execution_mode_label == "模拟" {
        return "模拟模式不读取交易所私有持仓与强平价；切换实盘后必须配置对应凭证并通过运行态预检。"
            .into();
    }
    let Some(evidence) = preview.liquidation.positions_evidence.as_ref() else {
        return "等待后端返回持仓/强平证据".into();
    };
    let mut parts = vec![
        format!("状态 {}", list_status_label(evidence.status)),
        format!("source {}", evidence.source),
        format!("observed {}", evidence.observed_at_ms),
        format!("rows {}", evidence.row_count),
    ];
    if let Some(liq) = evidence.current_account_liq_distance_pct {
        parts.push(format!("当前强平 {}", pct(liq)));
    }
    if !evidence.operation_health.is_empty() {
        parts.push(format!(
            "运行态 {}",
            operation_health_detail(&evidence.operation_health)
        ));
    }
    if !evidence.field_quality.is_empty() {
        parts.push(format!(
            "字段 {}",
            account_field_quality_summary(&evidence.field_quality)
        ));
    }
    if !evidence.problems.is_empty() {
        parts.push(format!(
            "问题 {}",
            preflight_problems_summary(&evidence.problems)
        ));
    }
    if let Some(retry_after_ms) = evidence.retry_after_ms {
        parts.push(format!("重试 {retry_after_ms}ms"));
    }
    if let Some(request_id) = evidence.request_id.as_deref() {
        parts.push(format!("请求 {}", short_id(request_id)));
    }
    parts.join(" · ")
}

pub(super) fn positions_evidence_line(evidence: &HedgePreviewPositionsEvidence) -> String {
    let mut parts = vec![
        list_status_label(evidence.status).to_owned(),
        format!("{} 行", evidence.row_count),
    ];
    if let Some(summary) = operation_health_summary(&evidence.operation_health) {
        parts.push(summary);
    }
    if !evidence.field_quality.is_empty() {
        parts.push(format!("{} 字段缺证据", evidence.field_quality.len()));
    }
    if !evidence.problems.is_empty() {
        parts.push(format!("{} 问题", evidence.problems.len()));
    }
    parts.join(" · ")
}

pub(super) fn list_status_label(status: ListStatus) -> &'static str {
    match status {
        ListStatus::Fresh => "Fresh",
        ListStatus::Degraded => "Degraded",
    }
}

pub(super) fn operation_health_summary(rows: &[VenueOperationHealth]) -> Option<String> {
    let total = rows.len();
    if total == 0 {
        return None;
    }
    let ok = rows
        .iter()
        .filter(|row| row.status == VenueOperationStatus::Ok)
        .count();
    Some(format!("运行态 {ok}/{total}"))
}

pub(super) fn operation_health_detail(rows: &[VenueOperationHealth]) -> String {
    let mut parts = rows
        .iter()
        .take(4)
        .map(operation_health_line)
        .collect::<Vec<_>>();
    if rows.len() > parts.len() {
        parts.push(format!("+{} 条", rows.len() - parts.len()));
    }
    parts.join(" / ")
}

pub(super) fn operation_health_line(row: &VenueOperationHealth) -> String {
    let mut parts = vec![
        row.venue.clone(),
        row.operation.clone(),
        venue_operation_status_label(row.status).into(),
    ];
    if let Some(freshness_ms) = row.freshness_ms {
        parts.push(format!("{freshness_ms}ms"));
    }
    if let Some(retry_after_ms) = row.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    parts.join(" ")
}

pub(super) fn venue_operation_status_label(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "OK",
        VenueOperationStatus::Warn => "WARN",
        VenueOperationStatus::Blocked => "BLOCK",
        VenueOperationStatus::Unknown => "UNKNOWN",
        VenueOperationStatus::Unsupported => "UNSUPPORTED",
    }
}

pub(super) fn depth_health_lines(preview: &ExecutionPreview) -> Vec<String> {
    [
        ("多腿", preview.depth.long_depth_health.as_ref()),
        ("空腿", preview.depth.short_depth_health.as_ref()),
    ]
    .into_iter()
    .filter_map(|(label, health)| depth_health_line(label, health))
    .collect()
}

pub(super) fn depth_health_line(label: &str, health: Option<&MarketDataHealth>) -> Option<String> {
    health.map(|health| format!("{label} {}", market_health_label(health)))
}
