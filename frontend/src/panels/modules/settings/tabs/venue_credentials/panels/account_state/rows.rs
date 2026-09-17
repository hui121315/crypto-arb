use super::*;
use shared_types::{
    AccountBindingEvidence, AccountBindingStatus, AccountDataHealth, AccountEquityScope,
    AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubject, VenueAccountSummary,
};

pub(super) fn account_summary_row(row: &VenueAccountSummary) -> impl IntoView {
    let freshness = freshness_label(row.freshness_ms);
    let margin = format!(
        "Available ${:.2} · Withdrawable {} · IM ${:.2} · MM ${:.2}",
        row.total_available_balance_usd,
        row.withdrawable_balance_usd
            .map(|value| format!("${value:.2}"))
            .unwrap_or_else(|| "未知".to_owned()),
        row.total_initial_margin_usd,
        row.total_maintenance_margin_usd
    );
    let rates = format!(
        "IM {:.4}% · MM {:.4}%",
        row.account_im_rate * 100.0,
        row.account_mm_rate * 100.0
    );
    let problem = row
        .problem
        .as_ref()
        .map(account_problem_label)
        .unwrap_or_else(|| rates.clone());
    view! {
        <tr>
            <td>
                <strong>{format!("{} {}", row.venue, row.account_type)}</strong>
                <em>{format!("Equity ${:.2}", row.total_equity_usd)}</em>
            </td>
            <td>
                <span class="status-pill ready">{equity_scope_label(row.equity_scope)}</span>
                <em>{row.source.clone()}</em>
            </td>
            <td>{freshness}<em>{format!("观察 {}", row.observed_at_ms)}</em></td>
            <td>{margin}<em>{problem}</em></td>
        </tr>
    }
}

pub(super) fn account_evidence_table(
    label: &'static str,
    count: usize,
    rows: AnyView,
    empty: &'static str,
) -> AnyView {
    view! {
        <div class="table-wrap">
            <table class="clean-table settings-table runtime-health-table">
                <thead>
                    <tr>
                        <th>{label}</th>
                        <th>"状态 / 来源"</th>
                        <th>"时间 / 请求"</th>
                        <th>"问题 / 范围"</th>
                    </tr>
                </thead>
                <tbody>
                    {if count == 0 {
                        view! { <tr><td colspan="4" class="empty-cell">{empty}</td></tr> }.into_any()
                    } else {
                        rows
                    }}
                </tbody>
            </table>
        </div>
    }
    .into_any()
}

pub(super) fn account_field_quality_row(row: &AccountFieldQuality) -> impl IntoView {
    let subject = account_subject_label(&row.subject);
    let status = account_field_quality_status_label(row.status);
    let class = account_field_quality_status_class(row.status);
    let observed = row
        .observed_at_ms
        .map(|value| format!("观察 {value}"))
        .unwrap_or_else(|| "观察 -".to_owned());
    let problem = row
        .problem
        .as_ref()
        .map(account_problem_label)
        .unwrap_or_else(|| "-".to_owned());
    view! {
        <tr>
            <td><strong>{subject}</strong><em>{row.field.clone()}</em></td>
            <td><span class=class>{status}</span><em>{row.source.clone()}</em></td>
            <td>{observed}</td>
            <td>{problem}</td>
        </tr>
    }
}

pub(super) fn account_data_health_row(row: &AccountDataHealth) -> impl IntoView {
    let subject = account_subject_label(&row.subject);
    let freshness = freshness_label(row.freshness_ms);
    let request = row
        .request_id
        .as_deref()
        .map(|value| format!("request_id {value}"))
        .unwrap_or_else(|| "request_id -".to_owned());
    let problem = row
        .last_error
        .as_ref()
        .map(account_problem_label)
        .unwrap_or_else(|| "-".to_owned());
    view! {
        <tr>
            <td><strong>{subject}</strong><em>{format!("观察 {}", row.observed_at_ms)}</em></td>
            <td><span class="status-pill pending">"健康"</span><em>{row.source.clone()}</em></td>
            <td>{freshness}<em>{request}</em></td>
            <td>{problem}</td>
        </tr>
    }
}

pub(super) fn account_binding_row(row: &AccountBindingEvidence) -> impl IntoView {
    let status = account_binding_status_label(row.status);
    let class = account_binding_status_class(row.status);
    let scope = row
        .account_scope
        .as_deref()
        .unwrap_or("账户范围 -")
        .to_owned();
    let timing = match (row.checked_at_ms, row.freshness_ms) {
        (Some(checked), Some(freshness)) => format!("检查 {checked} · freshness {freshness}ms"),
        (Some(checked), None) => format!("检查 {checked}"),
        (None, Some(freshness)) => format!("freshness {freshness}ms"),
        (None, None) => "检查 -".to_owned(),
    };
    let fingerprint = row
        .credential_fingerprint
        .as_deref()
        .map(|value| format!("credential {value}"))
        .unwrap_or_else(|| "credential -".to_owned());
    let problem = row
        .problem
        .as_ref()
        .map(account_problem_label)
        .unwrap_or_else(|| scope.clone());
    view! {
        <tr>
            <td><strong>{row.venue.clone()}</strong><em>{scope}</em></td>
            <td><span class=class>{status}</span><em>{row.source.clone()}</em></td>
            <td>{timing}<em>{fingerprint}</em></td>
            <td>{problem}</td>
        </tr>
    }
}

pub(super) fn account_problem_row(problem: &ApiProblem) -> impl IntoView {
    let source = problem
        .source
        .clone()
        .unwrap_or_else(|| "source -".to_owned());
    let timing = problem
        .retry_after_ms
        .map(|value| format!("retry {value}ms"))
        .unwrap_or_else(|| "retry -".to_owned());
    let request = problem
        .request_id
        .as_deref()
        .map(|value| format!("request_id {value}"))
        .unwrap_or_else(|| "request_id -".to_owned());
    view! {
        <tr>
            <td><strong>{problem.code.clone()}</strong><em>{problem.message.clone()}</em></td>
            <td><span class="status-pill blocked">"问题"</span><em>{source}</em></td>
            <td>{timing}<em>{request}</em></td>
            <td>{account_problem_label(problem)}</td>
        </tr>
    }
}

fn account_subject_label(subject: &AccountFieldSubject) -> String {
    let venue = subject.venue.as_deref().unwrap_or("账户");
    let subject_label = match subject.kind {
        shared_types::AccountFieldSubjectKind::Account => venue.to_owned(),
        shared_types::AccountFieldSubjectKind::Balance => subject
            .currency
            .as_deref()
            .map(|currency| format!("{venue} {currency}"))
            .unwrap_or_else(|| venue.to_owned()),
        shared_types::AccountFieldSubjectKind::Position => subject
            .symbol
            .as_deref()
            .map(|symbol| format!("{venue} {symbol}"))
            .unwrap_or_else(|| venue.to_owned()),
        shared_types::AccountFieldSubjectKind::OpenOrder => subject
            .order_id
            .as_deref()
            .map(|order_id| format!("{venue} {order_id}"))
            .unwrap_or_else(|| venue.to_owned()),
    };
    subject
        .account_scope
        .as_deref()
        .map(|scope| format!("{subject_label} · {scope}"))
        .unwrap_or(subject_label)
}

fn equity_scope_label(scope: AccountEquityScope) -> &'static str {
    match scope {
        AccountEquityScope::Unified => "统一账户权益",
        AccountEquityScope::Perpetuals => "永续账户权益",
        AccountEquityScope::Spot => "现货账户权益",
        AccountEquityScope::Unknown => "权益范围未知",
    }
}

fn account_problem_label(problem: &ApiProblem) -> String {
    let mut parts = vec![format!("{}: {}", problem.code, problem.message)];
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    parts.join(" · ")
}

pub(super) fn account_state_status_label(status: shared_types::ListStatus) -> &'static str {
    match status {
        shared_types::ListStatus::Fresh => "正常",
        shared_types::ListStatus::Degraded => "需关注",
    }
}

pub(super) fn account_state_status_class(status: shared_types::ListStatus) -> &'static str {
    match status {
        shared_types::ListStatus::Fresh => "status-pill ready",
        shared_types::ListStatus::Degraded => "status-pill blocked",
    }
}

fn account_field_quality_status_label(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "实际",
        AccountFieldQualityStatus::Estimated => "估算",
        AccountFieldQualityStatus::Unknown => "未知",
        AccountFieldQualityStatus::Invalid => "无效",
        AccountFieldQualityStatus::Missing => "缺失",
    }
}

fn account_field_quality_status_class(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "status-pill ready",
        AccountFieldQualityStatus::Estimated | AccountFieldQualityStatus::Unknown => {
            "status-pill pending"
        }
        AccountFieldQualityStatus::Invalid | AccountFieldQualityStatus::Missing => {
            "status-pill blocked"
        }
    }
}

fn account_binding_status_label(status: AccountBindingStatus) -> &'static str {
    match status {
        AccountBindingStatus::Verified => "已验证",
        AccountBindingStatus::Unverified => "未验证",
        AccountBindingStatus::Failed => "失败",
    }
}

fn account_binding_status_class(status: AccountBindingStatus) -> &'static str {
    match status {
        AccountBindingStatus::Verified => "status-pill ready",
        AccountBindingStatus::Unverified => "status-pill pending",
        AccountBindingStatus::Failed => "status-pill blocked",
    }
}
