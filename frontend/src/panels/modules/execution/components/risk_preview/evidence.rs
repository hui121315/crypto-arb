use super::*;
use crate::panels::modules::market_evidence::structured_problem_context_label;

#[path = "evidence/order_plan.rs"]
mod order_plan;

pub(super) use order_plan::{order_plan_detail, order_plan_summary};

pub(super) fn fee_evidence_summary(preview: &ExecutionPreview) -> String {
    let count = preview.fee_evidence.len();
    if count == 0 {
        return "等待费率数据依据".into();
    }
    let missing = preview
        .fee_evidence
        .iter()
        .filter(|line| {
            line.health.contains("缺")
                || line.health.contains("未验证")
                || line.health.contains("问题")
        })
        .count();
    if missing == 0 {
        format!("{count} 条已带数据依据")
    } else {
        format!("{count} 条 · {missing} 条需复核")
    }
}

pub(super) fn fee_evidence_detail(preview: &ExecutionPreview) -> String {
    if preview.fee_evidence.is_empty() {
        return "等待后端 HedgeTicket 返回 fee snapshots".into();
    }
    preview
        .fee_evidence
        .iter()
        .map(|line| {
            format!(
                "{} · {} · {} · {}",
                line.label, line.rate, line.source, line.health
            )
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

pub(super) fn guard_detail(guard: &ExecutionGuard) -> String {
    let Some(outcome) = guard.preflight_outcome.as_ref() else {
        return guard.detail.clone();
    };
    let mut parts = vec![
        guard.detail.clone(),
        preflight_status_label(outcome.status).into(),
    ];
    if !outcome.scope.venues.is_empty() {
        parts.push(format!("范围 {}", outcome.scope.venues.join(",")));
    }
    if !outcome.scope.symbols.is_empty() {
        parts.push(format!("标的 {}", outcome.scope.symbols.join(",")));
    }
    if !outcome.scope.account_modes.is_empty() {
        parts.push(format!("账户 {}", outcome.scope.account_modes.join(",")));
    }
    if !outcome.scope.operations.is_empty() {
        parts.push(format!(
            "操作 {}",
            preflight_operations_summary(&outcome.scope.operations)
        ));
    }
    if !outcome.observed_venues.is_empty() {
        parts.push(format!("读到 {}", outcome.observed_venues.join(",")));
    }
    if outcome.checked_at_ms > 0 {
        parts.push(format!("检查 {}", outcome.checked_at_ms));
    }
    if !outcome.balance_rows.is_empty() {
        parts.push(format!(
            "余额 {}",
            balance_rows_summary(&outcome.balance_rows)
        ));
    }
    if !outcome.row_health.is_empty() {
        parts.push(format!(
            "余额健康 {}",
            account_data_health_summary(&outcome.row_health)
        ));
    }
    if !outcome.field_quality.is_empty() {
        parts.push(format!(
            "字段 {}",
            account_field_quality_summary(&outcome.field_quality)
        ));
    }
    if !outcome.problems.is_empty() {
        parts.push(format!(
            "问题 {}",
            preflight_problems_summary(&outcome.problems)
        ));
    }
    if let Some(source) = outcome.source.as_deref() {
        parts.push(format!("来源 {source}"));
    }
    if let Some(freshness_ms) = outcome.freshness_ms {
        parts.push(format!("新鲜度 {freshness_ms}ms"));
    }
    if let Some(retry_after_ms) = outcome.retry_after_ms {
        parts.push(format!("重试 {retry_after_ms}ms"));
    }
    if let Some(request_id) = outcome.request_id.as_deref() {
        parts.push(format!("请求 {}", short_id(request_id)));
    }
    if let Some(error) = outcome.error.as_deref() {
        parts.push(format!("错误 {error}"));
    }
    parts.join(" · ")
}

pub(super) fn preflight_operations_summary(operations: &[HedgePreflightOperation]) -> String {
    operations
        .iter()
        .map(|operation| preflight_operation_label(*operation))
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn preflight_operation_label(operation: HedgePreflightOperation) -> &'static str {
    match operation {
        HedgePreflightOperation::MarginBalance => "保证金余额",
        HedgePreflightOperation::PrivateRead => "私有读取",
        HedgePreflightOperation::Positions => "持仓读取",
        HedgePreflightOperation::OpenOrders => "挂单读取",
        HedgePreflightOperation::Capability => "订单能力",
        HedgePreflightOperation::AccountMode => "账户模式",
        HedgePreflightOperation::OrderWrite => "下单权限",
        HedgePreflightOperation::PrivateWs => "私有WS",
        HedgePreflightOperation::OrderFinality => "订单最终结果",
        HedgePreflightOperation::Orderbook => "订单簿",
    }
}

pub(super) fn account_field_quality_summary(rows: &[AccountFieldQuality]) -> String {
    let mut parts = rows
        .iter()
        .take(4)
        .map(account_field_quality_line)
        .collect::<Vec<_>>();
    if rows.len() > parts.len() {
        parts.push(format!("+{} 条", rows.len() - parts.len()));
    }
    parts.join(" / ")
}

pub(super) fn account_field_quality_line(row: &AccountFieldQuality) -> String {
    format!(
        "{} {} {} · {}",
        account_field_subject(row),
        row.field,
        account_quality_status_label(row.status),
        row.source,
    )
}

pub(super) fn account_field_subject(row: &AccountFieldQuality) -> String {
    let venue = row.subject.venue.as_deref().unwrap_or("账户");
    match row.subject.kind {
        AccountFieldSubjectKind::Account => venue.to_owned(),
        AccountFieldSubjectKind::Balance => match row.subject.currency.as_deref() {
            Some(currency) => format!("{venue} {currency}"),
            None => venue.to_owned(),
        },
        AccountFieldSubjectKind::Position => match row.subject.symbol.as_deref() {
            Some(symbol) => format!("{venue} {symbol}"),
            None => venue.to_owned(),
        },
        AccountFieldSubjectKind::OpenOrder => match (
            row.subject.symbol.as_deref(),
            row.subject.order_id.as_deref(),
        ) {
            (Some(symbol), Some(order_id)) => format!("{venue} {symbol} {order_id}"),
            (Some(symbol), None) => format!("{venue} {symbol}"),
            _ => venue.to_owned(),
        },
    }
}

pub(super) fn account_quality_status_label(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "OK",
        AccountFieldQualityStatus::Estimated => "EST",
        AccountFieldQualityStatus::Unknown => "UNKNOWN",
        AccountFieldQualityStatus::Invalid => "INVALID",
        AccountFieldQualityStatus::Missing => "MISSING",
    }
}

pub(super) fn preflight_problems_summary(problems: &[ApiProblem]) -> String {
    let mut parts = problems
        .iter()
        .take(3)
        .map(preflight_problem_line)
        .collect::<Vec<_>>();
    if problems.len() > parts.len() {
        parts.push(format!("+{} 条", problems.len() - parts.len()));
    }
    parts.join(" / ")
}

pub(super) fn preflight_problem_line(problem: &ApiProblem) -> String {
    let mut parts = vec![problem.code.clone(), problem.message.clone()];
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("req {}", short_id(request_id)));
    }
    if let Some(context) = structured_problem_context_label(problem) {
        parts.push(context);
    }
    parts.join(" ")
}
