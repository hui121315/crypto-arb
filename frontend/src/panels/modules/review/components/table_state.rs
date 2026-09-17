//! 复盘四段表格的区段行集。
//!
//! 行集 + 区段运行态的组合已抽到通用 [`crate::state::table_runtime::TableSection`]
//! （TableRuntime），review 直接复用它，仅保留本模块自己的 `problem_meta` 文案；
//! 这里只 re-export 类型别名 + 区段占位行组件，不再各写一份 loading/ready/stale/
//! error 状态机。
use leptos::prelude::*;

pub(in crate::panels::modules::review) use crate::state::table_runtime::TableSection as ReviewSectionRows;

pub(in crate::panels::modules::review) fn section_state_row(
    text: String,
    colspan: &'static str,
) -> impl IntoView {
    view! {
        <tr>
            <td colspan=colspan class="empty-cell">{text}</td>
        </tr>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_is_distinct_from_ready_empty() {
        let section: ReviewSectionRows<u8> = ReviewSectionRows::loading();

        assert_eq!(section.empty_text("暂无记录"), "读取中");
        assert!(!section.has_loaded_context());
    }

    #[test]
    fn error_keeps_problem_visible() {
        let section: ReviewSectionRows<u8> = ReviewSectionRows::error("HTTP 502 · req-1".into());

        assert_eq!(section.empty_text("暂无记录"), "读取失败：HTTP 502 · req-1");
        assert!(!section.has_loaded_context());
    }

    #[test]
    fn stale_keeps_rows_available_for_table() {
        let section = ReviewSectionRows::stale(vec![7_u8], "timeout".into());

        assert_eq!(section.rows, vec![7]);
        assert!(section.has_loaded_context());
    }

    #[test]
    fn stale_empty_keeps_problem_visible() {
        let section: ReviewSectionRows<u8> = ReviewSectionRows::stale(Vec::new(), "timeout".into());

        assert_eq!(
            section.empty_text("暂无记录"),
            "上次快照为空，刷新失败：timeout"
        );
    }

    #[test]
    fn ready_empty_uses_business_empty_copy() {
        let section: ReviewSectionRows<u8> = ReviewSectionRows::ready(Vec::new());

        assert_eq!(section.empty_text("暂无记录"), "暂无记录");
        assert!(section.has_loaded_context());
    }
}
