//! 资金费周期统计：生产 DTO 投影（`from_dto.rs`）+ 视图模型（`view_model.rs`）+
//! 纯文案/样式助手（`format.rs`）+ 趋势组件（`component.rs`）。测试夹具见 `testing.rs`。

#[path = "funding_stats/component.rs"]
mod component;
#[path = "funding_stats/format.rs"]
mod format;
#[path = "funding_stats/from_dto.rs"]
mod from_dto;
#[cfg(test)]
#[path = "funding_stats/testing.rs"]
mod testing;
#[path = "funding_stats/view_model.rs"]
mod view_model;

pub(in crate::panels::modules) use component::funding_cycle_trend;
pub(crate) use view_model::FundingCycleStatsView;
