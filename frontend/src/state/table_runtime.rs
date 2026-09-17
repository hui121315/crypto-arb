//! 通用表格运行态（TableRuntime）。
//!
//! 历史上每个表格面板（review / positions / opportunities / futures …）都各写一份
//! 「行集 + 区段状态」的组合：review 有 `ReviewSectionRows<T>`、positions 有
//! `SectionData<Vec<T>>`，再各自把 `LoadState<Env>` 手工 `match` 成 loading/ready/
//! stale/error 行集。逻辑一致却分散，错误文案与降级语义容易漂移。
//!
//! 这里把该组合抽成**单一**事实源 `TableSection<Row>`，叠在
//! [`crate::state::section::SectionStatus`] 之上：任何表格都用
//! [`TableSection::from_load_state`] 从同一个 `LoadState<Env>` 派生行集与区段态，
//! 不再各写一份 `match`。
//!
//! fail-closed：`stale`/`error` 始终保留 `ApiProblem` 的
//! `message`/`status`/`request_id`/`retry_after`（见 [`SectionStatus`]），
//! 绝不把真实失败渲染成空数据或"读取中"。

use crate::state::load_state::LoadState;
use crate::state::section::SectionStatus;
use shared_types::ApiProblem;

/// 一个表格区段：当前可渲染的行集 + 区段运行态。
///
/// `Row` 是面板自己的行 view-model（如 `ExecutedTrade` / `PositionRow`）。
/// `stale` 时 `rows` 是上一份快照（仍可渲染），`error`/`loading` 时为空。
#[derive(Clone, PartialEq)]
pub(crate) struct TableSection<Row> {
    pub(crate) rows: Vec<Row>,
    status: SectionStatus,
}

impl<Row> TableSection<Row> {
    /// 首包尚未返回。
    pub(crate) fn loading() -> Self {
        Self {
            rows: Vec::new(),
            status: SectionStatus::Loading,
        }
    }

    /// 拿到新鲜行集。
    pub(crate) fn ready(rows: Vec<Row>) -> Self {
        Self {
            rows,
            status: SectionStatus::Ready,
        }
    }

    /// 刷新失败但保留上一份行集（携带失败原因文案）。
    pub(crate) fn stale(rows: Vec<Row>, problem: String) -> Self {
        Self {
            rows,
            status: SectionStatus::Stale { problem },
        }
    }

    /// 读取失败且无可用行集（携带失败原因文案）。
    pub(crate) fn error(problem: String) -> Self {
        Self {
            rows: Vec::new(),
            status: SectionStatus::Error { problem },
        }
    }

    /// 是否有"可展示的已加载上下文"（Ready 或 Stale）——决定是否渲染汇总/图表。
    pub(crate) const fn has_loaded_context(&self) -> bool {
        self.status.has_loaded_context()
    }

    /// 真实空态文案（默认"读取中 / 读取失败"前缀）：区分读取中 / 真实空 /
    /// 刷新失败但有旧快照 / 读取失败，永不把失败显示成空数据或等待中。
    pub(crate) fn empty_text(&self, ready_empty: &str) -> String {
        self.status.empty_text(ready_empty, "读取中", "读取失败")
    }
}

impl<Row: Clone> TableSection<Row> {
    /// 从任意 `LoadState<Env>` 派生表格区段——`TableRuntime` 的核心：
    /// 各面板只提供「如何从 envelope 取行集」与「如何把 `ApiProblem` 渲成文案」两个
    /// 闭包，loading/ready/stale/error 语义统一由本函数 + [`SectionStatus`] 派生，
    /// 不再各写一份 `match`。`problem_text` 让各面板保留自己的文案口径（如 review
    /// 的 `problem_meta`）。
    ///
    /// - `Loading`        -> 读取中（空行集）
    /// - `Ready(env)`     -> `rows_of(env)`
    /// - `Stale{value,..}`-> 上一份 `rows_of(value)` + 保留 `problem`
    /// - `Error(problem)` -> 空行集 + 保留 `problem`
    pub(crate) fn from_load_state<Env>(
        state: &LoadState<Env>,
        rows_of: impl Fn(&Env) -> Vec<Row>,
        problem_text: impl Fn(&ApiProblem) -> String,
    ) -> Self {
        match state {
            LoadState::Loading => Self::loading(),
            LoadState::Ready(env) => Self::ready(rows_of(env)),
            LoadState::Stale { value, problem } => {
                Self::stale(rows_of(value), problem_text(problem))
            }
            LoadState::Error(problem) => Self::error(problem_text(problem)),
        }
    }
}

/// 单值区段：当前可渲染的值 + 区段运行态。`TableSection<Row>` 的标量兄弟——
/// 用于一个区段只承载单个 view-model（如 `PortfolioSummary` / `RiskSnapshot`）
/// 或一个不暴露行级占位的集合的场景。stale 时 `value` 是上一份快照。
/// 与行集形态 `TableSection` 共属同一个 `TableRuntime` 模块。
///
/// 与 `TableSection` 共享同一个 [`SectionStatus`] 核心，因此 loading/ready/stale/
/// error 语义与 fail-closed 错误文案在两种形状间完全一致，不再各写一份。
#[derive(Clone, PartialEq)]
pub(crate) struct SectionSlot<T> {
    pub(crate) value: T,
    pub(crate) status: SectionStatus,
}

impl<T> SectionSlot<T> {
    /// 拿到新鲜值。
    pub(crate) fn ready(value: T) -> Self {
        Self {
            value,
            status: SectionStatus::Ready,
        }
    }

    /// 刷新失败但保留上一份值（携带 request 上下文）。
    pub(crate) fn stale(value: T, problem: &ApiProblem) -> Self {
        Self {
            value,
            status: SectionStatus::stale_from(problem),
        }
    }

    /// 区段是否为"新鲜就绪"。
    pub(crate) const fn has_fresh_value(&self) -> bool {
        self.status.is_ready()
    }
}

impl<T: Default> SectionSlot<T> {
    /// 首包尚未返回（值取 `Default`，区段态为 Loading）。
    pub(crate) fn loading() -> Self {
        Self {
            value: T::default(),
            status: SectionStatus::Loading,
        }
    }

    /// 读取失败且无可用值（值取 `Default`，保留 request 上下文）。
    pub(crate) fn error(problem: &ApiProblem) -> Self {
        Self {
            value: T::default(),
            status: SectionStatus::error_from(problem),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(code: &str) -> ApiProblem {
        ApiProblem::new(code, "boom")
            .with_status(502)
            .with_request_id(Some("req-7".into()))
    }

    /// review 等面板的文案口径：保留失败消息并附上请求 id（见 review 的 `problem_meta`）。
    fn meta(problem: &ApiProblem) -> String {
        match problem.request_id.as_deref() {
            Some(request_id) => format!("{} · {request_id}", problem.message),
            None => problem.message.clone(),
        }
    }

    #[test]
    fn loading_is_distinct_from_ready_empty() {
        let section: TableSection<u8> = TableSection::loading();
        assert_eq!(section.empty_text("暂无记录"), "读取中");
        assert!(!section.has_loaded_context());
        assert!(section.rows.is_empty());
    }

    #[test]
    fn ready_empty_uses_business_copy() {
        let section: TableSection<u8> = TableSection::ready(Vec::new());
        assert_eq!(section.empty_text("暂无记录"), "暂无记录");
        assert!(section.has_loaded_context());
    }

    #[test]
    fn error_keeps_problem_text_and_is_not_loaded() {
        let section: TableSection<u8> = TableSection::error("HTTP 502 · req-1".into());
        assert_eq!(section.empty_text("暂无记录"), "读取失败：HTTP 502 · req-1");
        assert!(!section.has_loaded_context());
    }

    #[test]
    fn stale_keeps_rows_and_problem_visible() {
        let section = TableSection::stale(vec![7_u8], "timeout".into());
        assert_eq!(section.rows, [7]);
        assert!(section.has_loaded_context());
        assert_eq!(
            section.empty_text("暂无记录"),
            "上次快照为空，刷新失败：timeout"
        );
    }

    #[test]
    fn from_load_state_maps_every_variant() {
        let rows_of = |env: &Vec<u8>| env.clone();

        let loading = TableSection::from_load_state(&LoadState::<Vec<u8>>::Loading, rows_of, meta);
        assert!(!loading.has_loaded_context());
        assert!(loading.rows.is_empty());

        let ready = TableSection::from_load_state(&LoadState::Ready(vec![1_u8, 2]), rows_of, meta);
        assert_eq!(ready.rows, [1, 2]);
        assert!(ready.has_loaded_context());

        let stale = TableSection::from_load_state(
            &LoadState::Stale {
                value: vec![3_u8],
                problem: problem("STALE"),
            },
            rows_of,
            meta,
        );
        assert_eq!(stale.rows, [3]);
        assert!(stale.has_loaded_context());
        // stale 保留上一份行集；empty_text 用调用方口径的失败文案（含 request_id）。
        assert_eq!(
            stale.empty_text("暂无"),
            "上次快照为空，刷新失败：boom · req-7"
        );

        let error = TableSection::from_load_state(
            &LoadState::<Vec<u8>>::Error(problem("ERR")),
            rows_of,
            meta,
        );
        assert!(error.rows.is_empty());
        assert!(!error.has_loaded_context());
        assert_eq!(error.empty_text("暂无"), "读取失败：boom · req-7");
    }
}
