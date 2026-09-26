//! 持仓表的表格/单元格派生：分页页面、筛选/排序/数据集 key、状态与空态文案、单元格取值与格式化。
//! 字段质量匹配与证据 chip 见 `quality.rs`，表格组件见父模块 `positions_table.rs`。

use shared_types::{AccountFieldQuality, PositionRow, PositionSeverity, PositionSide};
use std::cmp::Ordering;
use std::sync::Arc;

use super::super::super::data::{pair_label, position_key};
use super::super::format::signed_money;
use super::super::section_state::{SectionData, SectionStatus};
use crate::panels::modules::pagination::TableRuntime;

#[derive(Clone, PartialEq)]
pub(super) struct TablePage {
    pub(super) source_total: usize,
    pub(super) total: usize,
    pub(super) rows: Vec<PositionRow>,
    pub(super) status: SectionStatus,
}

pub(super) fn table_page(
    section: SectionData<Vec<PositionRow>>,
    runtime: TableRuntime<PositionRow>,
) -> TablePage {
    let source_total = section.value.len();
    TablePage {
        source_total,
        total: runtime.window.total,
        rows: runtime.rows,
        status: section.status,
    }
}

pub(super) fn filtered_sorted_rows(
    section: SectionData<Vec<PositionRow>>,
    query: &str,
) -> Vec<PositionRow> {
    let mut rows = filtered_rows(section.value, query);
    sort_rows(&mut rows);
    rows
}

pub(super) fn stable_render_rows(
    previous: Option<&Vec<Arc<PositionRow>>>,
    rows: &[PositionRow],
) -> Vec<Arc<PositionRow>> {
    rows.iter()
        .cloned()
        .map(position_row_for_display)
        .map(|row| {
            let key = position_key(&row);
            previous
                .and_then(|current| {
                    current.iter().find(|existing| {
                        position_key(existing.as_ref()) == key && existing.as_ref() == &row
                    })
                })
                .cloned()
                .unwrap_or_else(|| Arc::new(row))
        })
        .collect()
}

fn position_row_for_display(mut row: PositionRow) -> PositionRow {
    // The table only renders whole minutes. Keeping second-level churn out of
    // row identity prevents a full DOM row replacement every portfolio frame.
    row.seconds_until_funding = row.seconds_until_funding.map(|seconds| (seconds / 60) * 60);
    row
}

pub(super) fn positions_dataset_key(
    section: &SectionData<Vec<PositionRow>>,
    query: &str,
) -> String {
    format!(
        "{}:{}",
        section_status_key(&section.status),
        positions_interaction_key(section, query),
    )
}

pub(super) fn positions_interaction_key(
    section: &SectionData<Vec<PositionRow>>,
    query: &str,
) -> String {
    let row_keys = section
        .value
        .iter()
        .map(position_key)
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "positions:{}:{}:{row_keys}",
        query.trim().to_ascii_lowercase(),
        section.value.len(),
    )
}

fn section_status_key(status: &SectionStatus) -> &'static str {
    match status {
        SectionStatus::Loading => "loading",
        SectionStatus::Ready => "ready",
        SectionStatus::Stale { .. } => "stale",
        SectionStatus::Error { .. } => "error",
    }
}

pub(super) fn table_status_label(
    section: SectionData<Vec<PositionRow>>,
    visible_total: usize,
) -> String {
    match section.status {
        SectionStatus::Loading => "读取中".to_owned(),
        SectionStatus::Ready => format!("{visible_total} 个持仓"),
        SectionStatus::Stale { problem } => {
            format!("{visible_total} 个持仓 · 上次快照 · {problem}")
        }
        SectionStatus::Error { problem } => format!("读取失败 · {problem}"),
    }
}

pub(super) fn table_empty_text(page: &TablePage) -> String {
    if page.source_total > 0 {
        return match &page.status {
            SectionStatus::Stale { problem } => {
                format!("没有匹配持仓，当前为上次快照：{problem}")
            }
            SectionStatus::Loading | SectionStatus::Ready | SectionStatus::Error { .. } => {
                "没有匹配持仓".to_owned()
            }
        };
    }
    page.status
        .empty_text("暂无持仓", "读取持仓中", "持仓读取失败")
}

fn filtered_rows(rows: Vec<PositionRow>, query: &str) -> Vec<PositionRow> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|row| row_matches(row, &query))
        .collect()
}

fn row_matches(row: &PositionRow, query: &str) -> bool {
    row.venue.to_ascii_lowercase().contains(query)
        || row.symbol.to_ascii_lowercase().contains(query)
        || pair_label(row).is_some_and(|pair| pair.to_ascii_lowercase().contains(query))
}

fn sort_rows(rows: &mut [PositionRow]) {
    rows.sort_by(|a, b| {
        let left = a.liquidation_distance_pct.unwrap_or(f64::MAX);
        let right = b.liquidation_distance_pct.unwrap_or(f64::MAX);
        left.partial_cmp(&right).unwrap_or(Ordering::Equal)
    });
}

pub(super) fn row_is_closing(active: Option<&str>, row_key: &str, pair_key: Option<&str>) -> bool {
    active.is_some_and(|value| value == row_key || pair_key == Some(value))
}

pub(super) struct CellDisplay {
    pub(super) value: String,
    pub(super) class: &'static str,
}

pub(super) struct FundingDisplay {
    pub(super) window: String,
    pub(super) detail: Option<String>,
    pub(super) class: &'static str,
}

pub(super) fn value_or_missing(quality: Option<&AccountFieldQuality>, value: String) -> String {
    if quality.is_some() {
        "数据待确认".to_owned()
    } else {
        value
    }
}

pub(super) fn pnl_display(value: f64, mark_quality: Option<&AccountFieldQuality>) -> CellDisplay {
    if mark_quality.is_some() {
        return CellDisplay {
            value: "数据待确认".to_owned(),
            class: "muted",
        };
    }
    CellDisplay {
        value: signed_money(value),
        class: if value >= 0.0 { "positive" } else { "negative" },
    }
}

pub(super) fn funding_display(
    row: &PositionRow,
    quality: Option<&AccountFieldQuality>,
) -> FundingDisplay {
    if quality.is_some() {
        return FundingDisplay {
            window: "数据待确认".to_owned(),
            detail: None,
            class: "muted",
        };
    }
    let detail = if row.funding_rate_verified && row.funding_rate_8h.is_finite() {
        Some(format!(
            "{} {}",
            funding_cashflow_label(row.side, row.funding_rate_8h),
            signed_rate(row.funding_rate_8h)
        ))
    } else {
        Some("现金流未知".to_owned())
    };
    FundingDisplay {
        window: funding_text(row.seconds_until_funding),
        detail,
        class: "",
    }
}

fn funding_cashflow_label(side: PositionSide, rate: f64) -> &'static str {
    if rate.abs() <= f64::EPSILON {
        return "持平";
    }
    match (side, rate.is_sign_positive()) {
        (PositionSide::Long, true) | (PositionSide::Short, false) => "将付",
        (PositionSide::Long, false) | (PositionSide::Short, true) => "将收",
    }
}

pub(super) fn side_label(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "多",
        PositionSide::Short => "空",
    }
}

pub(super) fn side_class(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "side-badge long",
        PositionSide::Short => "side-badge short",
    }
}

pub(super) fn severity_class(severity: PositionSeverity) -> &'static str {
    match severity {
        PositionSeverity::Danger => "danger-row",
        PositionSeverity::Warn => "warning-row",
        PositionSeverity::Unknown => "unknown-row",
        PositionSeverity::Ok => "",
    }
}

fn funding_text(seconds: Option<u32>) -> String {
    match seconds {
        Some(0) => "结算中".to_owned(),
        Some(value) => format!("{}m", value / 60),
        None => "-".to_owned(),
    }
}

fn signed_rate(value: f64) -> String {
    format!("{:+.4}%", value * 100.0)
}

pub(super) fn format_qty(value: f64) -> String {
    if value.abs() >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.4}")
    }
}

pub(super) fn price(value: f64) -> String {
    if !value.is_finite() {
        return "-".to_owned();
    }
    let absolute = value.abs();
    if absolute >= 1000.0 {
        format!("${value:.0}")
    } else if absolute >= 100.0 {
        format!("${value:.2}")
    } else if absolute >= 1.0 {
        format!("${value:.4}")
    } else if absolute >= 0.01 {
        format!("${value:.6}")
    } else if absolute > f64::EPSILON {
        format!("${value:.8}")
    } else {
        "$0.00".to_owned()
    }
}
