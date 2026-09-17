//! 复盘模块的纯状态→区段/文案派生：行集、分页、元信息与场所质量图表元数据。
//! 顶层组件与 Tab 选择见父模块 `view.rs`。

use crate::panels::modules::opportunity_counts::duration_label;
use crate::state::load_state::LoadState;
use crate::state::section::problem_message;
use shared_types::{
    ApiProblem, FundingPaymentIngestReport, FundingPaymentIngestSkipReason, ListPage, ListStatus,
    ReviewDataSource, ReviewEnvelope, ReviewLedgerStatus, ReviewPnlField, VenueOperationStatus,
    VenueQualityEnvelope, VenueQualitySource,
};

use super::super::components::{ReviewSectionRows, VenueQualityChartMeta};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReviewStatePresentation {
    pub(super) summary: String,
    pub(super) detail: String,
    pub(super) tone: &'static str,
    pub(super) badge: String,
    pub(super) action: &'static str,
}

impl ReviewStatePresentation {
    fn new(
        summary: String,
        detail: String,
        tone: &'static str,
        badge: impl Into<String>,
        action: &'static str,
    ) -> Self {
        Self {
            summary,
            detail,
            tone,
            badge: badge.into(),
            action,
        }
    }
}

pub(super) fn review_rows<T: Clone>(state: &LoadState<ReviewEnvelope<T>>) -> ReviewSectionRows<T> {
    match state {
        LoadState::Loading => ReviewSectionRows::loading(),
        LoadState::Ready(envelope) => review_rows_from_envelope(envelope),
        LoadState::Stale { value, problem } => {
            ReviewSectionRows::stale(value.rows.clone(), problem_meta(problem))
        }
        LoadState::Error(problem) => ReviewSectionRows::error(problem_meta(problem)),
    }
}

fn review_rows_from_envelope<T: Clone>(envelope: &ReviewEnvelope<T>) -> ReviewSectionRows<T> {
    let rows = envelope.rows.clone();
    if let Some(problem) = envelope.problems.first() {
        return ReviewSectionRows::stale(rows, problem_meta(problem));
    }
    if envelope.status != ListStatus::Fresh {
        return ReviewSectionRows::stale(rows, list_status_problem(envelope.status));
    }
    ReviewSectionRows::ready(rows)
}

fn list_status_problem(status: ListStatus) -> String {
    match status {
        ListStatus::Fresh => String::new(),
        ListStatus::Degraded => "快照已降级".into(),
    }
}

pub(super) fn review_page<T>(state: &LoadState<ReviewEnvelope<T>>) -> Option<ListPage> {
    state.value().map(|envelope| envelope.page.clone())
}

pub(super) fn venue_quality_rows(
    state: &LoadState<VenueQualityEnvelope>,
) -> ReviewSectionRows<shared_types::VenueQuality> {
    // 直接复用通用 TableRuntime（`TableSection::from_load_state`）派生区段态——
    // venue-quality 没有 review_rows 那种 envelope 级 problems/status 旁路，
    // 因此是 TableRuntime 的标准消费形态：只提供取行集 + 文案口径两个闭包。
    ReviewSectionRows::from_load_state(state, |envelope| envelope.rows.clone(), problem_meta)
}

pub(super) fn state_meta<T>(state: &LoadState<ReviewEnvelope<T>>) -> String {
    match state {
        LoadState::Loading => "读取中".into(),
        LoadState::Ready(envelope) => envelope_meta(envelope),
        LoadState::Stale { value, problem } => {
            format!(
                "{} · 降级 · {}",
                envelope_meta(value),
                problem_meta(problem)
            )
        }
        LoadState::Error(problem) => format!("读取失败 · {}", problem_meta(problem)),
    }
}

pub(super) fn state_headline<T>(state: &LoadState<ReviewEnvelope<T>>) -> String {
    match state {
        LoadState::Loading => "正在读取复盘数据".into(),
        LoadState::Ready(envelope) => envelope_headline(envelope),
        LoadState::Stale { value, .. } => format!("{} · 显示上次快照", envelope_headline(value)),
        LoadState::Error(_) => "复盘数据读取失败".into(),
    }
}

pub(super) fn review_state_presentation<T>(
    state: &LoadState<ReviewEnvelope<T>>,
) -> ReviewStatePresentation {
    let summary = state_headline(state);
    let detail = state_meta(state);
    match state {
        LoadState::Loading => {
            ReviewStatePresentation::new(summary, detail, "is-loading", "读取中", "正在读取")
        }
        LoadState::Stale { problem, .. } => ReviewStatePresentation::new(
            summary,
            detail,
            "is-warning",
            format!("旧快照 · {}", problem.code),
            "查看降级证据",
        ),
        LoadState::Error(problem) => ReviewStatePresentation::new(
            summary,
            detail,
            "is-danger",
            problem.code.clone(),
            "查看错误证据",
        ),
        LoadState::Ready(envelope) => ready_review_presentation(summary, detail, envelope),
    }
}

fn ready_review_presentation<T>(
    summary: String,
    detail: String,
    envelope: &ReviewEnvelope<T>,
) -> ReviewStatePresentation {
    if let Some(health) = envelope.storage_health.as_ref() {
        if health.status == VenueOperationStatus::Blocked {
            return ReviewStatePresentation::new(
                summary,
                detail,
                "is-danger",
                "存储阻断",
                "查看错误证据",
            );
        }
        if health.status != VenueOperationStatus::Ok {
            return ReviewStatePresentation::new(
                summary,
                detail,
                "is-warning",
                "存储待核验",
                "查看数据证据",
            );
        }
    }
    if let Some(problem) = envelope.problems.first() {
        return ReviewStatePresentation::new(
            summary,
            detail,
            "is-warning",
            problem.code.clone(),
            "查看降级证据",
        );
    }
    if envelope.status != ListStatus::Fresh {
        return ReviewStatePresentation::new(
            summary,
            detail,
            "is-warning",
            "数据降级",
            "查看降级证据",
        );
    }
    if !envelope.missing_fields.is_empty()
        || envelope.ledger_status == Some(ReviewLedgerStatus::PartialEvidence)
    {
        return ReviewStatePresentation::new(
            summary,
            detail,
            "is-warning",
            "证据不全",
            "查看数据证据",
        );
    }
    ReviewStatePresentation::new(
        summary,
        detail,
        "is-ready",
        envelope
            .ledger_status
            .map(ledger_status_label)
            .unwrap_or("数据可用"),
        "查看数据证据",
    )
}

fn envelope_headline<T>(envelope: &ReviewEnvelope<T>) -> String {
    format!(
        "{} · {}/{} 行",
        source_label(envelope.source),
        envelope.page.returned_count,
        envelope.page.total_rows,
    )
}

fn envelope_meta<T>(envelope: &ReviewEnvelope<T>) -> String {
    let returned = envelope.page.returned_count;
    let total = envelope.page.total_rows;
    let mut parts = vec![
        source_label(envelope.source).to_string(),
        format!("{returned}/{total} 行"),
    ];
    if envelope.page.has_more {
        parts.push("还有下一页".to_string());
    }
    if let Some(request_id) = envelope.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(problem) = envelope.problems.first() {
        parts.push(problem_summary(
            "分页降级",
            problem,
            envelope.problems.len(),
        ));
    }
    if let Some(status) = envelope.ledger_status {
        parts.push(ledger_status_label(status).to_string());
    }
    if !envelope.missing_fields.is_empty() {
        parts.push(format!(
            "缺 {}",
            missing_fields_label(&envelope.missing_fields)
        ));
    }
    if let Some(report) = &envelope.funding_payment_ingest {
        parts.push(funding_ingest_label(report));
    }
    if let Some(label) = storage_health_label(envelope) {
        parts.push(label);
    }
    parts.join(" · ")
}

fn funding_ingest_label(report: &FundingPaymentIngestReport) -> String {
    let mut parts = vec![format!(
        "资金费入账 {}/{}",
        report.ledger_events, report.mapped
    )];
    if report.invalid > 0 {
        parts.push(format!("无效 {}", report.invalid));
    }
    if report.skipped > 0 {
        parts.push(format!("跳过 {}", report.skipped));
    }
    if report.duplicate_or_already_recorded > 0 {
        parts.push(format!("重复 {}", report.duplicate_or_already_recorded));
    }
    if report.no_matching_order > 0 {
        parts.push(format!("无匹配 {}", report.no_matching_order));
    }
    if report.no_filled_anchor > 0 {
        parts.push(format!("缺成交锚点 {}", report.no_filled_anchor));
    }
    if report.ambiguous_order_group > 0 {
        parts.push(format!("歧义 {}", report.ambiguous_order_group));
    }
    if report.unmatched_or_ambiguous_order > 0 {
        parts.push(format!(
            "未归因/歧义 {}",
            report.unmatched_or_ambiguous_order
        ));
    }
    if report.route_failures > 0 {
        parts.push(format!("路由失败 {}", report.route_failures));
    }
    if report.unsupported {
        parts.push("未配置".to_string());
    }
    if let Some(reason) = report.skip_reasons.first() {
        parts.push(format!(
            "首因 {}:{}",
            funding_skip_reason_label(reason.reason),
            reason.count
        ));
    }
    parts.join(" / ")
}

fn funding_skip_reason_label(reason: FundingPaymentIngestSkipReason) -> &'static str {
    match reason {
        FundingPaymentIngestSkipReason::InvalidRow => "无效行",
        FundingPaymentIngestSkipReason::DuplicateOrAlreadyRecorded => "重复",
        FundingPaymentIngestSkipReason::InvalidMatchKey => "匹配键无效",
        FundingPaymentIngestSkipReason::NoMatchingOrder => "无匹配订单",
        FundingPaymentIngestSkipReason::NoFilledAnchor => "缺成交锚点",
        FundingPaymentIngestSkipReason::AmbiguousOrderGroup => "歧义订单组",
        FundingPaymentIngestSkipReason::UnmatchedOrAmbiguousOrder => "未归因或歧义",
    }
}

pub(super) fn venue_quality_meta(state: &LoadState<VenueQualityEnvelope>) -> String {
    match state {
        LoadState::Loading => "读取中".into(),
        LoadState::Ready(envelope) => quality_envelope_meta(envelope),
        LoadState::Stale { value, problem } => {
            format!(
                "{} · 降级 · {}",
                quality_envelope_meta(value),
                problem_meta(problem)
            )
        }
        LoadState::Error(problem) => format!("读取失败 · {}", problem_meta(problem)),
    }
}

pub(super) fn venue_quality_headline(state: &LoadState<VenueQualityEnvelope>) -> String {
    match state {
        LoadState::Loading => "正在读取场所执行质量".into(),
        LoadState::Ready(envelope) => quality_headline(envelope),
        LoadState::Stale { value, .. } => format!("{} · 显示上次快照", quality_headline(value)),
        LoadState::Error(_) => "场所执行质量读取失败".into(),
    }
}

pub(super) fn venue_quality_state_presentation(
    state: &LoadState<VenueQualityEnvelope>,
) -> ReviewStatePresentation {
    let summary = venue_quality_headline(state);
    let detail = venue_quality_meta(state);
    match state {
        LoadState::Loading => {
            ReviewStatePresentation::new(summary, detail, "is-loading", "读取中", "正在读取")
        }
        LoadState::Stale { problem, .. } => ReviewStatePresentation::new(
            summary,
            detail,
            "is-warning",
            format!("旧快照 · {}", problem.code),
            "查看降级证据",
        ),
        LoadState::Error(problem) => ReviewStatePresentation::new(
            summary,
            detail,
            "is-danger",
            problem.code.clone(),
            "查看错误证据",
        ),
        LoadState::Ready(envelope) => {
            if let Some(problem) = envelope
                .rows
                .iter()
                .find_map(|row| row.last_problem.as_ref())
            {
                return ReviewStatePresentation::new(
                    summary,
                    detail,
                    "is-warning",
                    problem.code.clone(),
                    "查看降级证据",
                );
            }
            if envelope.attention_count > 0 {
                return ReviewStatePresentation::new(
                    summary,
                    detail,
                    "is-warning",
                    format!("{} 项需关注", envelope.attention_count),
                    "查看数据证据",
                );
            }
            let badge = if envelope.sampled_count == 0 {
                "暂无运行样本"
            } else {
                "执行质量可用"
            };
            ReviewStatePresentation::new(summary, detail, "is-ready", badge, "查看数据证据")
        }
    }
}

fn quality_headline(envelope: &VenueQualityEnvelope) -> String {
    let attention = if envelope.attention_count == 0 {
        "无需关注".to_owned()
    } else {
        format!("{} 需关注", envelope.attention_count)
    };
    format!(
        "{} 场所 · {} 已采样 · {attention}",
        envelope.row_count, envelope.sampled_count
    )
}

pub(super) fn venue_quality_chart_meta(
    state: &LoadState<VenueQualityEnvelope>,
    now_ms: i64,
) -> VenueQualityChartMeta {
    match state {
        LoadState::Loading => VenueQualityChartMeta::waiting(),
        LoadState::Ready(envelope) => quality_envelope_chart_meta(envelope, now_ms),
        LoadState::Stale { value, problem } => {
            let meta = quality_envelope_chart_meta(value, now_ms);
            VenueQualityChartMeta::stale(meta.source, meta.freshness, problem_meta(problem))
        }
        LoadState::Error(problem) => VenueQualityChartMeta::failed(problem_meta(problem)),
    }
}

fn quality_envelope_chart_meta(
    envelope: &VenueQualityEnvelope,
    now_ms: i64,
) -> VenueQualityChartMeta {
    VenueQualityChartMeta::ready(
        quality_source_label(envelope.source),
        duration_label(now_ms.saturating_sub(envelope.generated_at_ms)),
    )
}

fn quality_envelope_meta(envelope: &VenueQualityEnvelope) -> String {
    let mut parts = vec![
        quality_source_label(envelope.source).to_owned(),
        format!("{} 场所", envelope.row_count),
        format!("{} 已采样", envelope.sampled_count),
        format!("{} operation", envelope.operation_count),
    ];
    if envelope.attention_count > 0 {
        parts.push(format!("{} 需关注", envelope.attention_count));
    }
    if let Some(retry_after_ms) = envelope.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if let Some(request_id) = envelope.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(problem) = envelope
        .rows
        .iter()
        .filter_map(|row| row.last_problem.as_ref())
        .max_by_key(|problem| problem.retry_after_ms.unwrap_or(0))
    {
        let prefix = format!("最近问题 {}", problem.code);
        parts.push(problem_summary(&prefix, problem, envelope.attention_count));
    }
    parts.join(" · ")
}

fn quality_source_label(source: VenueQualitySource) -> &'static str {
    match source {
        VenueQualitySource::RuntimeSamples => "执行质量样本",
        VenueQualitySource::NeutralNoSample => "无样本基线",
    }
}

fn source_label(source: ReviewDataSource) -> &'static str {
    match source {
        ReviewDataSource::ExecutionLedger => "执行账本",
        ReviewDataSource::MissedOpportunityStore => "错失记录",
    }
}

fn ledger_status_label(status: ReviewLedgerStatus) -> &'static str {
    match status {
        ReviewLedgerStatus::LedgerBacked => "账本可读",
        ReviewLedgerStatus::PartialEvidence => "证据不全",
        ReviewLedgerStatus::NoCompleteRows => "无完整双腿",
        ReviewLedgerStatus::NoLedgerEvents => "暂无交易记录",
    }
}

fn storage_health_label<T>(envelope: &ReviewEnvelope<T>) -> Option<String> {
    let health = envelope.storage_health.as_ref()?;
    match health.status {
        VenueOperationStatus::Ok => None,
        VenueOperationStatus::Warn => Some(format!("存储警告：{}", health.message)),
        VenueOperationStatus::Blocked => Some(format!("存储阻断：{}", health.message)),
        VenueOperationStatus::Unknown => Some(format!("存储待验证：{}", health.message)),
        VenueOperationStatus::Unsupported => Some(format!("存储不支持：{}", health.message)),
    }
}

fn missing_fields_label(fields: &[ReviewPnlField]) -> String {
    fields
        .iter()
        .map(|field| match field {
            ReviewPnlField::Gross => "毛收益",
            ReviewPnlField::Fee => "费用",
            ReviewPnlField::Funding => "Funding",
            ReviewPnlField::Slippage => "滑点",
            ReviewPnlField::Net => "净收益",
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn problem_meta(problem: &ApiProblem) -> String {
    problem_message(problem)
}

fn problem_summary(prefix: &str, problem: &ApiProblem, total: usize) -> String {
    let mut summary = format!("{prefix}：{}", problem_meta(problem));
    if total > 1 {
        summary.push_str(&format!(" +{}", total - 1));
    }
    summary
}

#[cfg(test)]
#[path = "derive/tests.rs"]
mod tests;
