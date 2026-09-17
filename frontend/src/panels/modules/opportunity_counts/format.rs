//! 机会计数模块共享的纯文案助手：问题文案（含 retry 兜底）与时长格式化。
//! 计数选择器见 `count_meta.rs`，新鲜度/覆盖文案见 `freshness.rs`，空态文案见 `empty_label.rs`。

use crate::state::arbitrage_stream::stream_problem_label;
use shared_types::ApiProblem;

pub(in crate::panels::modules::opportunity_counts) fn problem_text(
    problem: &ApiProblem,
    fallback_retry_after_ms: Option<u64>,
) -> String {
    let mut label = stream_problem_label(problem);
    if problem.retry_after_ms.is_none() {
        if let Some(retry_after_ms) = fallback_retry_after_ms {
            label.push_str(&format!(" · retry {retry_after_ms}ms"));
        }
    }
    label
}

pub(crate) fn duration_label(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        format!("{:.1}m", ms as f64 / 60_000.0)
    }
}
