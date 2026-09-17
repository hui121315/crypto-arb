//! 机会计数与空态文案：计数元数据/选择器（`count_meta.rs`）、新鲜度/覆盖文案
//! （`freshness.rs`）、空态/KPI 文案（`empty_label.rs`）、共享纯文案助手（`format.rs`）。

#[path = "opportunity_counts/count_meta.rs"]
mod count_meta;
#[path = "opportunity_counts/empty_label.rs"]
mod empty_label;
#[path = "opportunity_counts/format.rs"]
mod format;
#[path = "opportunity_counts/freshness.rs"]
mod freshness;

pub(crate) use count_meta::OpportunityCountMeta;
pub(crate) use empty_label::{
    opportunity_empty_label, opportunity_kpi_placeholder, OpportunityEmptyLabelInput,
};
pub(crate) use format::duration_label;
