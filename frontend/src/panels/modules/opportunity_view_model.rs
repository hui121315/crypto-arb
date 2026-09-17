//! 机会列表视图模型：结构/缓存/构造（`model.rs`）、展示文案（`labels.rs`）、
//! 派生与格式化助手（`format.rs`）及共享投影单测（`testing.rs`）。

#[path = "opportunity_view_model/cache.rs"]
mod cache;
#[path = "opportunity_view_model/format.rs"]
mod format;
#[path = "opportunity_view_model/labels.rs"]
mod labels;
#[path = "opportunity_view_model/model.rs"]
mod model;
#[cfg(test)]
#[path = "opportunity_view_model/patch.rs"]
mod patch;
#[cfg(test)]
#[path = "opportunity_view_model/testing.rs"]
mod testing;

#[cfg(test)]
pub(crate) use cache::rejected_main_p0_ids;
pub(crate) use cache::view_models_from_rows;
pub(crate) use model::{OpportunityListViewModel, OpportunityListViewRow};
#[cfg(test)]
pub(crate) use patch::patch_projected_rows;
