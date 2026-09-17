//! 通用区段运行态状态机（loading / ready / stale / error）。
//!
//! 历史上 positions(`SectionData<T>`) 与 review(`ReviewSectionRows<T>`) 各自实现
//! 了一份字段完全一致的 `SectionStatus`，导致"区段降级/错误可见化"逻辑重复、易漂移。
//! 这里抽出**单一**事实源：容器各模块仍可保留（positions 持单值、review 持行集），
//! 但区段的 loading/ready/stale/error 语义与错误文案统一从本状态机派生，
//! 任何面板要做"降级横幅/真实空态/错误上屏"都复用它、不再重写。
//!
//! fail-closed：`stale`/`error` 始终保留 `ApiProblem` 的 `message`/`status`/`request_id`/
//! `retry_after`，绝不把真实失败显示成空数据或"读取中"。

use shared_types::{ApiProblem, ApiRecoveryAction};

/// 一个数据区段的运行态。与具体容器（单值 / 行集）解耦，可被任意面板复用。
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum SectionStatus {
    /// 首包尚未返回。
    Loading,
    /// 已拿到新鲜数据。
    Ready,
    /// 刷新失败，但仍展示上一份快照（保留失败原因）。
    Stale { problem: String },
    /// 读取失败且无可用快照（保留失败原因）。
    Error { problem: String },
}

impl SectionStatus {
    /// 由 typed `ApiProblem` 构造 `Stale`（保留 request 上下文）。
    pub(crate) fn stale_from(problem: &ApiProblem) -> Self {
        Self::Stale {
            problem: problem_message(problem),
        }
    }

    /// 由 typed `ApiProblem` 构造 `Error`（保留 request 上下文）。
    pub(crate) fn error_from(problem: &ApiProblem) -> Self {
        Self::Error {
            problem: problem_message(problem),
        }
    }

    /// 是否为"新鲜就绪"——仅 `Ready`，stale/loading/error 都不算新鲜。
    pub(crate) const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// 是否有"可展示的已加载上下文"——`Ready` 或 `Stale`（stale 仍有上次快照可渲染）。
    pub(crate) const fn has_loaded_context(&self) -> bool {
        matches!(self, Self::Ready | Self::Stale { .. })
    }

    /// 真实空态文案：区分"读取中 / 真实空 / 刷新失败但有旧快照 / 读取失败"。
    /// 永不把失败显示成空数据或等待中。
    pub(crate) fn empty_text(
        &self,
        ready_empty: &str,
        loading: &str,
        error_prefix: &str,
    ) -> String {
        match self {
            Self::Loading => loading.to_owned(),
            Self::Ready => ready_empty.to_owned(),
            Self::Stale { problem } => format!("上次快照为空，刷新失败：{problem}"),
            Self::Error { problem } => format!("{error_prefix}：{problem}"),
        }
    }

    /// stale 横幅备注（其它态返回 `None`，不臆造提示）。
    pub(crate) fn stale_note(&self, prefix: &str) -> Option<String> {
        match self {
            Self::Stale { problem } => Some(format!("{prefix}：{problem}")),
            Self::Loading | Self::Ready | Self::Error { .. } => None,
        }
    }
}

/// 把 typed `ApiProblem` 渲染成保留 request 上下文的可读串。
pub(crate) fn problem_message(problem: &ApiProblem) -> String {
    let mut parts = vec![problem.message.clone()];
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if problem.recovery_action.is_some() {
        push_identity_context(&mut parts, problem);
        parts.extend(problem_detail_context(problem));
    }
    push_recovery_context(&mut parts, problem);
    parts.join(" · ")
}

pub(crate) fn prefixed_problem_message(prefix: &str, problem: &ApiProblem) -> String {
    let mut parts = vec![format!("{prefix}：{}", problem.message)];
    if let Some(context) = problem_context(problem) {
        parts.push(context);
    }
    parts.join(" · ")
}

pub(crate) fn problem_context(problem: &ApiProblem) -> Option<String> {
    let mut parts = Vec::new();
    push_identity_context(&mut parts, problem);
    parts.extend(problem_detail_context(problem));
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem
        .request_id
        .as_deref()
        .filter(|request_id| !request_id.trim().is_empty())
    {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    push_recovery_context(&mut parts, problem);
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(crate) fn problem_detail_context(problem: &ApiProblem) -> Vec<String> {
    let Some(details) = problem.details.as_ref() else {
        return Vec::new();
    };
    let mut parts = Vec::with_capacity(6);
    push_string_detail(&mut parts, details, "venue", "venue");
    push_string_detail(&mut parts, details, "operation", "operation");
    match (
        string_detail(details, "method"),
        string_detail(details, "path"),
    ) {
        (Some(method), Some(path)) => parts.push(format!("{method} {path}")),
        (Some(method), None) => parts.push(format!("method {method}")),
        (None, Some(path)) => parts.push(format!("path {path}")),
        (None, None) => {}
    }
    push_string_detail(&mut parts, details, "symbol", "symbol");
    for key in ["upstreamCode", "exchangeCode", "venueCode"] {
        if let Some(value) = string_detail(details, key) {
            parts.push(format!("upstream_code {value}"));
            break;
        }
    }
    if let Some(latency_ms) = details
        .get("latencyMs")
        .or_else(|| details.get("lastLatencyMs"))
        .and_then(serde_json::Value::as_u64)
    {
        parts.push(format!("latency {latency_ms}ms"));
    }
    parts
}

pub(crate) fn recovery_action_context(problem: &ApiProblem) -> Option<String> {
    problem
        .effective_recovery_action()
        .map(|action| format!("下一步 {}", recovery_action_label(action)))
}

fn push_identity_context(parts: &mut Vec<String>, problem: &ApiProblem) {
    if !problem.code.trim().is_empty() {
        parts.push(format!("code {}", problem.code));
    }
    if let Some(source) = problem
        .source
        .as_deref()
        .filter(|source| !source.trim().is_empty())
    {
        parts.push(format!("source {source}"));
    }
}

fn push_recovery_context(parts: &mut Vec<String>, problem: &ApiProblem) {
    if let Some(context) = recovery_action_context(problem) {
        parts.push(context);
    }
}

fn string_detail<'a>(details: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    details
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn push_string_detail(
    parts: &mut Vec<String>,
    details: &serde_json::Value,
    key: &str,
    label: &str,
) {
    if let Some(value) = string_detail(details, key) {
        parts.push(format!("{label} {value}"));
    }
}

const fn recovery_action_label(action: ApiRecoveryAction) -> &'static str {
    match action {
        ApiRecoveryAction::ReviewRequest => "检查请求参数",
        ApiRecoveryAction::Authenticate => "重新认证",
        ApiRecoveryAction::CheckPermissions => "检查账户权限",
        ApiRecoveryAction::Retry => "重试",
        ApiRecoveryAction::RetryAfterDelay => "等待后重试",
        ApiRecoveryAction::RefreshState => "刷新状态",
        ApiRecoveryAction::CheckRuntimeHealth => "检查运行状态",
        ApiRecoveryAction::ContactOperator => "联系操作员",
        ApiRecoveryAction::ManualReview => "人工复核",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_is_distinct_from_ready_empty() {
        let status = SectionStatus::Loading;
        assert_eq!(
            status.empty_text("暂无记录", "读取中", "读取失败"),
            "读取中"
        );
        assert!(!status.is_ready());
        assert!(!status.has_loaded_context());
    }

    #[test]
    fn ready_empty_uses_business_copy() {
        let status = SectionStatus::Ready;
        assert_eq!(
            status.empty_text("暂无记录", "读取中", "读取失败"),
            "暂无记录"
        );
        assert!(status.is_ready());
        assert!(status.has_loaded_context());
    }

    #[test]
    fn error_keeps_request_context_and_is_not_loaded() {
        let problem = ApiProblem::new("RATE_LIMITED", "rate limited")
            .with_status(429)
            .with_request_id(Some("req-1".into()))
            .with_retry_after_ms(Some(2_000))
            .with_recovery_action(ApiRecoveryAction::RetryAfterDelay);
        let status = SectionStatus::error_from(&problem);
        assert_eq!(
            status.empty_text("暂无", "读取中", "读取失败"),
            "读取失败：rate limited · HTTP 429 · request_id req-1 · retry 2000ms · code RATE_LIMITED · 下一步 等待后重试"
        );
        assert!(!status.is_ready());
        assert!(!status.has_loaded_context());
        assert_eq!(status.stale_note("显示上次快照"), None);
    }

    #[test]
    fn stale_is_visible_but_not_fresh_and_keeps_problem() {
        let status = SectionStatus::stale_from(&ApiProblem::new("TIMEOUT", "slow"));
        assert!(!status.is_ready());
        assert!(status.has_loaded_context());
        assert_eq!(
            status.empty_text("暂无", "读取中", "读取失败"),
            "上次快照为空，刷新失败：slow"
        );
        assert_eq!(
            status.stale_note("显示上次快照").as_deref(),
            Some("显示上次快照：slow")
        );
    }

    #[test]
    fn formatter_keeps_structured_context_and_recovery_action() {
        let mut problem = ApiProblem::new("UPSTREAM_HTTP", "venue failed")
            .with_source("exchange-fanout")
            .with_status(502)
            .with_request_id(Some("req-context".into()))
            .with_recovery_action(ApiRecoveryAction::CheckRuntimeHealth);
        problem.details = Some(serde_json::json!({
            "venue": "okx",
            "operation": "private_read",
            "method": "GET",
            "path": "/api/v5/account/balance",
            "symbol": "BTC-USDT-SWAP",
            "exchangeCode": "50011",
            "latencyMs": 41
        }));

        let text = prefixed_problem_message("读取失败", &problem);

        assert!(text.contains("code UPSTREAM_HTTP"));
        assert!(text.contains("venue okx"));
        assert!(text.contains("operation private_read"));
        assert!(text.contains("GET /api/v5/account/balance"));
        assert!(text.contains("symbol BTC-USDT-SWAP"));
        assert!(text.contains("upstream_code 50011"));
        assert!(text.contains("latency 41ms"));
        assert!(text.contains("下一步 检查运行状态"));
    }
}
