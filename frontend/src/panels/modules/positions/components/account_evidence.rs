//! Shared account-envelope and credential-binding evidence for positions/balances surfaces.

use crate::api::ws::now_ms;
use leptos::prelude::*;
use shared_types::{AccountBindingEvidence, AccountBindingStatus, ApiProblem, ListStatus};

#[derive(Debug, Clone, PartialEq)]
pub(in crate::panels::modules::positions) struct AccountSurfaceEvidence {
    source: String,
    observed_at_ms: i64,
    status: ListStatus,
    bindings: Vec<AccountBindingEvidence>,
}

impl AccountSurfaceEvidence {
    pub(in crate::panels::modules::positions) fn new(
        source: impl Into<String>,
        observed_at_ms: i64,
        status: ListStatus,
        bindings: Vec<AccountBindingEvidence>,
    ) -> Self {
        Self {
            source: source.into(),
            observed_at_ms,
            status,
            bindings,
        }
    }
}

pub(super) fn render_account_surface_evidence(evidence: Option<AccountSurfaceEvidence>) -> AnyView {
    let Some(evidence) = evidence else {
        return ().into_any();
    };
    let envelope_chip = account_envelope_chip(&evidence).into_any();
    view! {
        <div class="balance-evidence">
            {envelope_chip}
            {binding_disclosure(&evidence.bindings)}
        </div>
    }
    .into_any()
}

fn binding_disclosure(bindings: &[AccountBindingEvidence]) -> AnyView {
    if bindings.is_empty() {
        return ().into_any();
    }
    let (summary, state_class) = binding_status_summary(bindings);
    let class = format!("balance-evidence-more balance-binding-evidence {state_class}");
    let title = format!("展开 {} 条账户绑定数据依据", bindings.len());
    view! {
        <details class=class>
            <summary title=title>
                <span>"账户绑定"</span>
                <strong>{summary}</strong>
            </summary>
            <div class="balance-evidence">
                {bindings.iter().map(account_binding_details).collect_view()}
            </div>
        </details>
    }
    .into_any()
}

fn binding_status_summary(bindings: &[AccountBindingEvidence]) -> (String, &'static str) {
    let verified = bindings
        .iter()
        .filter(|binding| binding.status == AccountBindingStatus::Verified)
        .count();
    let failed = bindings
        .iter()
        .filter(|binding| binding.status == AccountBindingStatus::Failed)
        .count();
    let unverified = bindings.len().saturating_sub(verified + failed);
    if failed > 0 {
        return (
            format!("{verified}/{} 已核对 · {failed} 失败", bindings.len()),
            "blocked",
        );
    }
    if unverified > 0 {
        return (
            format!("{verified}/{} 已核对 · {unverified} 待确认", bindings.len()),
            "warn",
        );
    }
    (format!("{verified}/{} 已核对", bindings.len()), "ok")
}

fn account_envelope_chip(evidence: &AccountSurfaceEvidence) -> impl IntoView {
    let age = observed_age_ms(evidence.observed_at_ms, now_ms()).map(duration_label);
    let class = format!(
        "balance-evidence-chip {}",
        envelope_status_class(evidence.status)
    );
    let title = envelope_title(evidence, age.as_deref());
    view! {
        <span class=class title=title>
            "业务 · " {evidence.source.clone()} " · " {envelope_status_label(evidence.status)}
            {age.map(|value| view! { <em>{value} " 前"</em> })}
        </span>
    }
}

fn account_binding_details(binding: &AccountBindingEvidence) -> impl IntoView {
    let class = format!(
        "balance-evidence-chip {}",
        binding_status_class(binding.status)
    );
    let title = binding_detail(binding, now_ms());
    let title_attr = title.clone();
    let summary = binding_summary(binding);
    view! {
        <span class=class title=title_attr aria-label=title tabindex="0">{summary}</span>
    }
}

fn binding_summary(binding: &AccountBindingEvidence) -> String {
    let mut summary = format!(
        "{} · {} · {}",
        binding.venue,
        binding.account_scope.as_deref().unwrap_or("范围未知"),
        binding_status_label(binding.status)
    );
    if let Some(freshness_ms) = binding.freshness_ms {
        summary.push_str(" · 新鲜度 ");
        summary.push_str(&duration_label_u64(freshness_ms));
    }
    summary
}

fn binding_detail(binding: &AccountBindingEvidence, now: u64) -> String {
    let mut parts = vec![
        binding_summary(binding),
        format!("source {}", binding.source),
        format!(
            "fingerprint {}",
            binding
                .credential_fingerprint
                .as_deref()
                .unwrap_or("未提供")
        ),
    ];
    if let Some(checked_at_ms) = binding.checked_at_ms {
        parts.push(format!("checkedAt {checked_at_ms}"));
        if let Some(age) = observed_age_ms(checked_at_ms, now) {
            parts.push(format!("checked {} 前", duration_label(age)));
        }
    }
    if let Some(freshness_ms) = binding.freshness_ms {
        parts.push(format!(
            "binding freshness {}",
            duration_label_u64(freshness_ms)
        ));
    }
    if let Some(problem) = binding.problem.as_ref() {
        parts.push(api_problem_detail(problem));
    }
    parts.join(" · ")
}

fn api_problem_detail(problem: &ApiProblem) -> String {
    let mut parts = vec![format!("problem {}: {}", problem.code, problem.message)];
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {}", duration_label(retry_after_ms as i64)));
    }
    if let Some(source) = problem.source.as_deref() {
        parts.push(format!("problem source {source}"));
    }
    if let Some(details) = problem.details.as_ref() {
        parts.push(format!("details {details}"));
    }
    parts.join(" · ")
}

fn envelope_title(evidence: &AccountSurfaceEvidence, age: Option<&str>) -> String {
    let age = age.unwrap_or("未知");
    format!(
        "业务来源 {} · observedAt {} · 业务年龄 {} · {}",
        evidence.source,
        evidence.observed_at_ms,
        age,
        envelope_status_label(evidence.status)
    )
}

fn observed_age_ms(observed_at_ms: i64, now: u64) -> Option<i64> {
    if observed_at_ms <= 0 {
        return None;
    }
    let now = now.min(i64::MAX as u64) as i64;
    Some(now.saturating_sub(observed_at_ms).max(0))
}

fn duration_label(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let secs = ms / 1_000;
    if secs < 60 {
        return format!("{secs}s");
    }
    format!("{}m", secs / 60)
}

fn duration_label_u64(ms: u64) -> String {
    duration_label(ms.min(i64::MAX as u64) as i64)
}

fn envelope_status_label(status: ListStatus) -> &'static str {
    match status {
        ListStatus::Fresh => "FRESH",
        ListStatus::Degraded => "DEGRADED",
    }
}

fn envelope_status_class(status: ListStatus) -> &'static str {
    match status {
        ListStatus::Fresh => "ok",
        ListStatus::Degraded => "warn",
    }
}

fn binding_status_label(status: AccountBindingStatus) -> &'static str {
    match status {
        AccountBindingStatus::Verified => "VERIFIED",
        AccountBindingStatus::Unverified => "UNVERIFIED",
        AccountBindingStatus::Failed => "FAILED",
    }
}

fn binding_status_class(status: AccountBindingStatus) -> &'static str {
    match status {
        AccountBindingStatus::Verified => "ok",
        AccountBindingStatus::Unverified => "unknown",
        AccountBindingStatus::Failed => "blocked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_detail_keeps_scope_fingerprint_and_problem_context() {
        let mut problem = ApiProblem::new("ACCOUNT_SCOPE_PROBE_FAILED", "permission denied")
            .with_status(403)
            .with_request_id(Some("req-bind".into()))
            .with_retry_after_ms(Some(2_000))
            .with_source("account_mode_probe");
        problem.details = Some(serde_json::json!({"operation": "account_scope"}));
        let binding = AccountBindingEvidence {
            venue: "okx".into(),
            account_scope: Some("cross_margin".into()),
            status: AccountBindingStatus::Failed,
            source: "credential_inventory".into(),
            checked_at_ms: Some(8_000),
            freshness_ms: Some(750),
            credential_fingerprint: Some("hmac-sha256:0123456789abcdef".into()),
            problem: Some(problem),
        };

        let detail = binding_detail(&binding, 10_000);

        assert!(detail.contains("cross_margin"));
        assert!(detail.contains("hmac-sha256:0123456789abcdef"));
        assert!(detail.contains("新鲜度 750ms"));
        assert!(detail.contains("binding freshness 750ms"));
        assert!(detail.contains("ACCOUNT_SCOPE_PROBE_FAILED"));
        assert!(detail.contains("request_id req-bind"));
        assert!(detail.contains("retry 2s"));
        assert!(detail.contains("account_scope"));
    }

    #[test]
    fn observed_age_is_distinct_and_saturating() {
        assert_eq!(observed_age_ms(8_000, 10_000), Some(2_000));
        assert_eq!(observed_age_ms(12_000, 10_000), Some(0));
        assert_eq!(observed_age_ms(0, 10_000), None);
    }

    #[test]
    fn binding_summary_reports_verified_and_blocked_counts() {
        let bindings = [
            AccountBindingEvidence {
                venue: "binance".into(),
                account_scope: None,
                status: AccountBindingStatus::Verified,
                source: "credential_inventory".into(),
                checked_at_ms: None,
                freshness_ms: None,
                credential_fingerprint: None,
                problem: None,
            },
            AccountBindingEvidence {
                venue: "gate".into(),
                account_scope: None,
                status: AccountBindingStatus::Failed,
                source: "credential_inventory".into(),
                checked_at_ms: None,
                freshness_ms: None,
                credential_fingerprint: None,
                problem: None,
            },
        ];

        assert_eq!(
            binding_status_summary(&bindings),
            ("1/2 已核对 · 1 失败".to_owned(), "blocked")
        );
    }
}
