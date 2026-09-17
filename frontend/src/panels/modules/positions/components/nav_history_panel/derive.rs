//! 仓位权益历史的纯数据派生：样本提取、趋势/迷你图路径、证据 chip 与问题文案。
//! 视图渲染与组件见父模块 `nav_history_panel.rs`。

use shared_types::{
    history::PortfolioNavHistoryRow, ApiProblem, HistoryResponse, VenueOperationHealth,
    VenueOperationStatus,
};

use super::super::format::{money, signed_pct};

#[path = "derive/diagnostics.rs"]
mod diagnostics;
pub(super) use diagnostics::{
    history_diagnostics, history_notice, HistoryDiagnostic, HistoryNotice,
};

pub(super) const CHART_WIDTH: u32 = 280;
pub(super) const CHART_HEIGHT: u32 = 96;

pub(super) type NavHistoryResponse = HistoryResponse<PortfolioNavHistoryRow>;

#[derive(Clone)]
pub(super) struct Chip {
    pub(super) label: String,
    pub(super) class_name: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct NavPoint {
    pub(super) occurred_at_ms: i64,
    pub(super) nav_usd: f64,
}

pub(super) struct NavTrend {
    pub(super) path: String,
    pub(super) stroke: &'static str,
    pub(super) latest: String,
    pub(super) change: String,
    pub(super) sample: String,
}

pub(super) fn nav_points(rows: &[PortfolioNavHistoryRow]) -> Vec<NavPoint> {
    let mut points = rows
        .iter()
        .filter(|row| row.nav_usd.is_finite())
        .map(|row| NavPoint {
            occurred_at_ms: row.occurred_at_ms,
            nav_usd: row.nav_usd,
        })
        .collect::<Vec<_>>();
    points.sort_by_key(|point| point.occurred_at_ms);
    points
}

pub(super) fn nav_trend(points: &[NavPoint]) -> Option<NavTrend> {
    if points.len() < 2 {
        return None;
    }
    let first = points.first()?;
    let last = points.last()?;
    let change_pct = nav_change_pct(first.nav_usd, last.nav_usd);
    Some(NavTrend {
        path: sparkline_path(points),
        stroke: "var(--color-accent)",
        latest: money(last.nav_usd),
        change: change_pct
            .map(|change| format!("净值变动 {}", signed_pct(change)))
            .unwrap_or_else(|| "基准仓位权益为 0".to_owned()),
        sample: format!(
            "样本 {} · {}",
            points.len(),
            duration_label(last.occurred_at_ms.saturating_sub(first.occurred_at_ms))
        ),
    })
}

fn sparkline_path(points: &[NavPoint]) -> String {
    let (min, max) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), point| {
            (min.min(point.nav_usd), max.max(point.nav_usd))
        });
    let range = (max - min).max(1e-9);
    let step = CHART_WIDTH as f64 / points.len().saturating_sub(1) as f64;
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let x = index as f64 * step;
            let y = CHART_HEIGHT as f64 - ((point.nav_usd - min) / range) * CHART_HEIGHT as f64;
            format!("{} {:.1} {:.1}", if index == 0 { "M" } else { "L" }, x, y)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn nav_change_pct(first: f64, last: f64) -> Option<f64> {
    (first.abs() > f64::EPSILON).then_some((last - first) / first.abs() * 100.0)
}

pub(super) fn history_chips(response: &NavHistoryResponse) -> Vec<Chip> {
    let mut chips = Vec::with_capacity(4);
    chips.push(chip(
        format!("样本 {}", response.count),
        "balance-evidence-chip",
    ));
    let backend = response.backend_status.backend.trim();
    if !backend.is_empty() {
        let durability = if response.backend_status.durable {
            "持久化"
        } else {
            "临时"
        };
        chips.push(chip(
            format!("{} {durability}", backend.to_ascii_uppercase()),
            "balance-evidence-chip",
        ));
    }
    if let Some(freshness_ms) = response.freshness_ms {
        chips.push(chip(
            format!("更新 {}", duration_label(freshness_ms)),
            "balance-evidence-chip",
        ));
    }
    if let Some(health) = response.storage_health.as_ref() {
        chips.push(storage_chip(health));
    }
    chips
}

fn storage_chip(health: &VenueOperationHealth) -> Chip {
    chip(
        format!("存储 {}", status_label(health.status)),
        status_class(health.status),
    )
}

fn chip(label: String, class_name: &'static str) -> Chip {
    Chip { label, class_name }
}

pub(super) fn problem_message(problem: &ApiProblem) -> String {
    let mut parts = vec![problem.message.clone()];
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(source) = problem.source.as_deref() {
        parts.push(format!("source {source}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    parts.join(" · ")
}

fn status_label(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "正常",
        VenueOperationStatus::Warn => "需关注",
        VenueOperationStatus::Blocked => "阻断",
        VenueOperationStatus::Unknown => "待确认",
        VenueOperationStatus::Unsupported => "不支持",
    }
}

fn status_class(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "balance-evidence-chip ok",
        VenueOperationStatus::Warn | VenueOperationStatus::Unknown => "balance-evidence-chip warn",
        VenueOperationStatus::Blocked => "balance-evidence-chip blocked",
        VenueOperationStatus::Unsupported => "balance-evidence-chip unknown",
    }
}

fn duration_label(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 60_000 {
        format!("{}s", ms / 1_000)
    } else if ms < 3_600_000 {
        format!("{}m", ms / 60_000)
    } else {
        format!("{:.1}h", ms as f64 / 3_600_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_points_sort_and_drop_non_finite_values() {
        let rows = vec![row(2_000, 120.0), row(1_000, f64::NAN), row(1_500, 110.0)];

        let points = nav_points(&rows);

        assert_eq!(points.len(), 2);
        assert_eq!(points[0].occurred_at_ms, 1_500);
        assert_eq!(points[1].nav_usd, 120.0);
    }

    #[test]
    fn nav_trend_labels_interval_change() -> Result<(), String> {
        let points = vec![point(1_000, 100.0), point(3_601_000, 125.0)];

        let trend = nav_trend(&points).ok_or_else(|| "missing trend".to_owned())?;

        assert!(trend.path.starts_with('M'));
        assert_eq!(trend.change, "净值变动 +25.00%");
        assert!(trend.sample.contains("样本 2"));
        assert!(trend.sample.contains("1.0h"));
        Ok(())
    }

    fn row(occurred_at_ms: i64, nav_usd: f64) -> PortfolioNavHistoryRow {
        PortfolioNavHistoryRow {
            occurred_at_ms,
            nav_usd,
        }
    }

    fn point(occurred_at_ms: i64, nav_usd: f64) -> NavPoint {
        NavPoint {
            occurred_at_ms,
            nav_usd,
        }
    }
}
