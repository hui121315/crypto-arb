//! 期货机会表格的费率与时长文本格式化。

use crate::panels::modules::rate_format::signed_bps_percent;

pub(in crate::panels::modules::futures::components) fn signed_bps_text(
    value: Option<f64>,
    missing: &str,
) -> String {
    value.map_or_else(|| missing.into(), signed_bps_percent)
}

pub(in crate::panels::modules::futures::components) fn alignment_text(
    value: Option<i32>,
) -> String {
    value.map_or_else(|| "待窗口".into(), |minutes| format!("{minutes:+}m"))
}

pub(in crate::panels::modules::futures::components) fn hours_text(value: Option<f64>) -> String {
    value.map_or_else(|| "未知".into(), |hours| format!("{hours:.1}h"))
}
