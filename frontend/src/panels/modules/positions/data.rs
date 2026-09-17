//! Positions 模块数据层。
//!
//! 为满足模块尺寸硬规则拆成聚焦子模块（生产代码各 ≤300 行）：
//! - [`snapshot`]：portfolio 快照 / NAV 历史的读取态、2s 轮询兜底 WS、请求版本闸、
//!   degraded envelope 落 `Stale`、WS close-run 首包前排队回灌。
//! - [`actions`]：平仓 / 一键清仓 / 补偿 / kill-switch 的提交类 hooks。
//! - [`requests`]：上述动作的请求 DTO 构造与 `ApiClient` 网络任务（缺证据 fail-closed）。
//! - [`runs`]：close-run 状态机派生（`ActionState` / 失败文案 / 补偿提示）与表格行键助手。

mod access;
mod actions;
mod requests;
mod runs;
mod runtime;
mod snapshot;

#[cfg(test)]
mod tests;

pub(in crate::panels::modules::positions) use access::*;
pub(in crate::panels::modules::positions) use actions::*;
pub(in crate::panels::modules::positions) use runs::*;
pub(in crate::panels) use runtime::create_positions_runtime;
pub(in crate::panels::modules::positions) use runtime::is_current_partial_snapshot_problem;
pub(in crate::panels) use runtime::PositionsRuntime;
pub(in crate::panels::modules::positions) use snapshot::*;
