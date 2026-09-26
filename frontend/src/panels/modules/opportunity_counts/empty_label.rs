//! 机会空态文案：按 加载/搜索/预热/降级/错误/过期/筛选 优先级区分空态，
//! KPI 占位与空态复用同一套诊断后缀（带 request id 与 retry）。
//! 计数选择器见 `count_meta.rs`，新鲜度文案见 `freshness.rs`，纯文案助手见 `format.rs`。

use crate::state::arbitrage_stream::stream_problem_label;
use crate::state::load_state::LoadState;
use shared_types::{ApiProblem, OpportunityEnvelopeStatus};

use super::count_meta::OpportunityCountMeta;
use super::format::problem_text;

pub(crate) struct OpportunityEmptyLabelInput<'a, T> {
    pub(crate) noun: &'a str,
    pub(crate) stream_state: &'a LoadState<T>,
    pub(crate) meta: &'a OpportunityCountMeta,
    pub(crate) stream_problem: Option<&'a ApiProblem>,
    pub(crate) search_loading: bool,
    pub(crate) search_problem: Option<&'a ApiProblem>,
    pub(crate) search_active: bool,
    pub(crate) search_rows_count: usize,
    pub(crate) local_filter_active: bool,
}

pub(crate) fn opportunity_empty_label<T>(input: &OpportunityEmptyLabelInput<'_, T>) -> String {
    if input.search_loading {
        return format!("品种搜索中，正在补拉{}", input.noun);
    }
    if let Some(problem) = input.search_problem {
        return format!("品种搜索失败 · {}", stream_problem_label(problem));
    }
    if let LoadState::Loading = input.stream_state {
        return format!("机会快照加载中，等待首批{}", input.noun);
    }
    if let LoadState::Error(problem) = input.stream_state {
        return format!("机会快照错误 · {}", stream_problem_label(problem));
    }
    if input.meta.status == OpportunityEnvelopeStatus::Warming {
        return format!(
            "机会快照预热中{}，等待首批{}",
            diagnostic_suffix(input.meta),
            input.noun
        );
    }
    if let Some(problem) = input.stream_problem {
        return format!("机会流降级 · {}", stream_problem_label(problem));
    }
    if input.search_active && input.search_rows_count == 0 {
        return format!("品种搜索无匹配{}，可调整品种或清空搜索", input.noun);
    }
    if input.local_filter_active {
        return format!("当前筛选无匹配{}", input.noun);
    }
    match input.meta.status {
        OpportunityEnvelopeStatus::Degraded => format!(
            "数据源降级{}，当前范围暂无{}",
            diagnostic_suffix(input.meta),
            input.noun
        ),
        OpportunityEnvelopeStatus::Error => {
            format!(
                "机会快照错误{}，当前范围暂无{}",
                diagnostic_suffix(input.meta),
                input.noun
            )
        }
        OpportunityEnvelopeStatus::Stale => {
            format!("机会快照已过期，当前范围暂无{}", input.noun)
        }
        OpportunityEnvelopeStatus::Fresh | OpportunityEnvelopeStatus::Warming => {
            format!("当前范围暂无{}", input.noun)
        }
    }
}

pub(crate) fn opportunity_kpi_placeholder<T>(
    stream_state: &LoadState<T>,
    meta: &OpportunityCountMeta,
    visible_rows_count: usize,
) -> Option<&'static str> {
    if meta.rows_retained {
        return Some("保留上次报价");
    }
    if visible_rows_count > 0 {
        return None;
    }
    match stream_state {
        LoadState::Loading => Some("加载中"),
        LoadState::Error(_) => Some("错误"),
        _ if meta.status == OpportunityEnvelopeStatus::Warming => Some("预热中"),
        _ if meta.status == OpportunityEnvelopeStatus::Error => Some("错误"),
        _ => None,
    }
}

fn retry_suffix(meta: &OpportunityCountMeta) -> String {
    meta.retry_after_ms
        .map(|ms| format!(" · 建议 {ms}ms 后重试"))
        .unwrap_or_default()
}

fn diagnostic_suffix(meta: &OpportunityCountMeta) -> String {
    let Some(problem) = meta
        .error
        .as_ref()
        .or_else(|| meta.partial_failures.first())
    else {
        return retry_suffix(meta);
    };
    format!(" · {}", problem_text(problem, meta.retry_after_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_label_distinguishes_warming_and_filter_empty() {
        let warming = OpportunityCountMeta {
            status: OpportunityEnvelopeStatus::Warming,
            retry_after_ms: Some(5_000),
            ..Default::default()
        };
        let fresh = OpportunityCountMeta::default();

        assert!(opportunity_empty_label(&OpportunityEmptyLabelInput {
            noun: "候选机会",
            stream_state: &LoadState::Ready(()),
            meta: &warming,
            stream_problem: None,
            search_loading: false,
            search_problem: None,
            search_active: false,
            search_rows_count: 0,
            local_filter_active: false,
        })
        .contains("预热中"));
        assert!(opportunity_empty_label(&OpportunityEmptyLabelInput {
            noun: "候选机会",
            stream_state: &LoadState::Ready(()),
            meta: &fresh,
            stream_problem: None,
            search_loading: false,
            search_problem: None,
            search_active: false,
            search_rows_count: 0,
            local_filter_active: true,
        })
        .contains("当前筛选"));
    }

    #[test]
    fn empty_label_distinguishes_search_empty_from_filter_empty() {
        let label = opportunity_empty_label(&OpportunityEmptyLabelInput {
            noun: "候选机会",
            stream_state: &LoadState::Ready(()),
            meta: &OpportunityCountMeta::default(),
            stream_problem: None,
            search_loading: false,
            search_problem: None,
            search_active: true,
            search_rows_count: 0,
            local_filter_active: true,
        });

        assert!(label.contains("品种搜索无匹配"));
        assert!(!label.contains("当前筛选"));
    }

    #[test]
    fn kpi_placeholder_hides_false_zero_during_warmup() {
        let warming = OpportunityCountMeta {
            status: OpportunityEnvelopeStatus::Warming,
            ..Default::default()
        };
        let fresh = OpportunityCountMeta::default();

        assert_eq!(
            opportunity_kpi_placeholder(&LoadState::<()>::Loading, &fresh, 0),
            Some("加载中")
        );
        assert_eq!(
            opportunity_kpi_placeholder(&LoadState::Ready(()), &warming, 0),
            Some("预热中")
        );
        assert_eq!(
            opportunity_kpi_placeholder(&LoadState::Ready(()), &warming, 2),
            None
        );
    }

    #[test]
    fn empty_label_surfaces_search_problem_first() {
        let problem = ApiProblem::new("RATE_LIMITED", "rate limited");
        let label = opportunity_empty_label(&OpportunityEmptyLabelInput {
            noun: "候选策略",
            stream_state: &LoadState::Ready(()),
            meta: &OpportunityCountMeta::default(),
            stream_problem: None,
            search_loading: false,
            search_problem: Some(&problem),
            search_active: false,
            search_rows_count: 0,
            local_filter_active: false,
        });

        assert!(label.contains("品种搜索失败"));
        assert!(label.contains("rate limited"));
    }

    #[test]
    fn degraded_empty_label_surfaces_partial_failure_source() {
        let problem = ApiProblem::new("MARKET_DATA_DEGRADED", "market data degraded")
            .with_source("market-data-cache")
            .with_retry_after_ms(Some(2_000));
        let meta = OpportunityCountMeta {
            status: OpportunityEnvelopeStatus::Degraded,
            partial_failures: vec![problem],
            ..Default::default()
        };

        let label = opportunity_empty_label(&OpportunityEmptyLabelInput {
            noun: "候选机会",
            stream_state: &LoadState::Ready(()),
            meta: &meta,
            stream_problem: None,
            search_loading: false,
            search_problem: None,
            search_active: false,
            search_rows_count: 0,
            local_filter_active: false,
        });

        assert!(label.contains("数据源降级"));
        assert!(label.contains("market-data-cache"));
        assert!(label.contains("retry 2000ms"));
    }
}
