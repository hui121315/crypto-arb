use leptos::prelude::*;
use shared_types::{AutoProfitCloseConfig, PositionRow};
use std::collections::BTreeSet;

use super::section_state::SectionData;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PairLiquidationRisk {
    pub(super) distance_pct: f64,
    pub(super) venue: String,
    pub(super) evidence_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PairCoverage {
    pair_count: usize,
    unpaired_count: usize,
}

pub(super) fn pair_liquidation_risk(
    row: &PositionRow,
    rows: &[PositionRow],
) -> Option<PairLiquidationRisk> {
    let partner = pair_partner(row, rows)?;
    let row_distance = row.liquidation_distance_pct.and_then(valid_distance);
    let partner_distance = partner.liquidation_distance_pct.and_then(valid_distance);
    let evidence_complete = row_distance.is_some() && partner_distance.is_some();
    let (distance_pct, venue) = match (row_distance, partner_distance) {
        (Some(left), Some(right)) if left <= right => (left, row.venue.clone()),
        (Some(_), Some(right)) => (right, partner.venue.clone()),
        (Some(left), None) => (left, row.venue.clone()),
        (None, Some(right)) => (right, partner.venue.clone()),
        (None, None) => return None,
    };
    Some(PairLiquidationRisk {
        distance_pct,
        venue,
        evidence_complete,
    })
}

pub(in crate::panels::modules::positions) fn pair_protection_bar(
    config: Memo<Option<AutoProfitCloseConfig>>,
    rows: Memo<SectionData<Vec<PositionRow>>>,
    open_settings: Callback<()>,
) -> impl IntoView {
    let coverage = Memo::new(move |_| pair_coverage(&rows.get().value));
    view! {
        <section class="positions-control-card pair-protection-bar">
            <header class="positions-control-card-header pair-protection-title">
                <span>"双边退出保护"</span>
                <strong>{move || protection_state(config.get().as_ref(), coverage.get())}</strong>
            </header>
            <div class="positions-control-card-body">
                <p class="positions-control-scope">
                    {move || pair_coverage_label(coverage.get())}
                </p>
                <p class="positions-control-description">
                    {move || current_pair_risk_label(&rows.get().value, coverage.get())}
                </p>
            </div>
            <div class="pair-protection-metrics">
                <div class="pair-protection-rule">
                    <span>"止盈"</span>
                    <strong>{move || take_profit_state(config.get().as_ref())}</strong>
                    <em>{move || take_profit_threshold(config.get().as_ref())}</em>
                </div>
                <div class="pair-protection-rule">
                    <span>"止损"</span>
                    <strong>{move || stop_loss_state(config.get().as_ref())}</strong>
                    <em>{move || stop_loss_threshold(config.get().as_ref())}</em>
                </div>
                <div class="pair-protection-rule">
                    <span>"强平保护"</span>
                    <strong>{move || liquidation_state(config.get().as_ref())}</strong>
                    <em>{move || liquidation_threshold(config.get().as_ref())}</em>
                </div>
            </div>
            <button
                class="row-action positions-control-primary-action"
                type="button"
                on:click=move |_| open_settings.run(())
            >
                "调整保护"
            </button>
            <em class="positions-action-message">
                {move || protection_evidence_label(config.get().as_ref())}
            </em>
        </section>
    }
}

fn protection_state(
    config: Option<&AutoProfitCloseConfig>,
    coverage: PairCoverage,
) -> &'static str {
    match config {
        None => "读取中",
        Some(config) if !protection_enabled(config) => "全部停用",
        Some(_) if coverage.pair_count == 0 => "等待配对",
        Some(_) => "监控中",
    }
}

fn protection_enabled(config: &AutoProfitCloseConfig) -> bool {
    config.enabled || config.stop_loss_enabled || config.liquidation_guard_enabled
}

fn take_profit_state(config: Option<&AutoProfitCloseConfig>) -> &'static str {
    config.map_or("待读取", |config| switch_label(config.enabled))
}

fn take_profit_threshold(config: Option<&AutoProfitCloseConfig>) -> String {
    config.map_or_else(
        || "阈值待证".to_owned(),
        |config| {
            format!(
                "${:.2} / {:.2}%",
                config.min_net_profit_usd,
                config.min_roi_bps / 100.0
            )
        },
    )
}

fn stop_loss_state(config: Option<&AutoProfitCloseConfig>) -> &'static str {
    config.map_or("待读取", |config| switch_label(config.stop_loss_enabled))
}

fn stop_loss_threshold(config: Option<&AutoProfitCloseConfig>) -> String {
    config.map_or_else(
        || "阈值待证".to_owned(),
        |config| {
            format!(
                "${:.2} / {:.2}%",
                config.max_net_loss_usd,
                config.max_loss_roi_bps / 100.0
            )
        },
    )
}

fn liquidation_state(config: Option<&AutoProfitCloseConfig>) -> &'static str {
    config.map_or("待读取", |config| {
        switch_label(config.liquidation_guard_enabled)
    })
}

fn liquidation_threshold(config: Option<&AutoProfitCloseConfig>) -> String {
    config.map_or_else(
        || "阈值待证".to_owned(),
        |config| {
            format!(
                "任一腿 <= {:.2}% · 越线立即退出",
                config.liquidation_exit_distance_pct
            )
        },
    )
}

fn protection_evidence_label(config: Option<&AutoProfitCloseConfig>) -> String {
    config.map_or_else(
        || "正在读取保护证据规则".to_owned(),
        |config| {
            format!(
                "止盈/止损需连续 {} 份双腿账户样本 · 强平保护按最新交易所距离",
                config.confirmation_samples
            )
        },
    )
}

fn current_pair_risk_label(rows: &[PositionRow], coverage: PairCoverage) -> String {
    if coverage.pair_count == 0 {
        return "当前无双边仓位 · 保护规则未生效".to_owned();
    }
    let mut visited = BTreeSet::new();
    let risk = rows
        .iter()
        .filter(|row| {
            row.pair_evidence
                .as_ref()
                .is_some_and(|pair| visited.insert(pair.run_id.clone()))
        })
        .filter_map(|row| pair_liquidation_risk(row, rows))
        .min_by(|left, right| left.distance_pct.total_cmp(&right.distance_pct));
    risk.map_or_else(
        || format!("当前 {} 组配对 · 待强平距离证据", coverage.pair_count),
        |risk| {
            if risk.distance_pct <= 0.0 && risk.evidence_complete {
                format!(
                    "已越过报告强平线 {:.2}% · {} 腿",
                    risk.distance_pct.abs(),
                    risk.venue
                )
            } else if risk.distance_pct <= 0.0 {
                format!(
                    "已知腿越过报告强平线 {:.2}% · {} 腿 · 另一腿待证",
                    risk.distance_pct.abs(),
                    risk.venue
                )
            } else if risk.evidence_complete {
                format!("当前最小距离 {:.2}% · {} 腿", risk.distance_pct, risk.venue)
            } else {
                format!(
                    "当前已知最小距离 {:.2}% · {} 腿 · 另一腿待证",
                    risk.distance_pct, risk.venue
                )
            }
        },
    )
}

fn pair_coverage(rows: &[PositionRow]) -> PairCoverage {
    let mut runs = BTreeSet::new();
    let mut paired_positions = 0;
    for row in rows {
        let Some(pair) = row.pair_evidence.as_ref() else {
            continue;
        };
        if pair_partner(row, rows).is_some() {
            paired_positions += 1;
            runs.insert(pair.run_id.clone());
        }
    }
    PairCoverage {
        pair_count: runs.len(),
        unpaired_count: rows.len().saturating_sub(paired_positions),
    }
}

fn pair_coverage_label(coverage: PairCoverage) -> String {
    format!(
        "{} 组可保护配对 · {} 个未配对仓位",
        coverage.pair_count, coverage.unpaired_count
    )
}

fn pair_partner<'a>(row: &PositionRow, rows: &'a [PositionRow]) -> Option<&'a PositionRow> {
    let evidence = row.pair_evidence.as_ref()?;
    rows.iter().find(|candidate| {
        candidate
            .pair_evidence
            .as_ref()
            .is_some_and(|candidate_pair| {
                shared_types::venue_names_equal(&candidate.venue, &evidence.partner_venue)
                    && candidate
                        .symbol
                        .eq_ignore_ascii_case(&evidence.partner_symbol)
                    && candidate.side == evidence.partner_side
                    && candidate_pair.run_id == evidence.run_id
                    && candidate_pair
                        .partner_venue
                        .eq_ignore_ascii_case(&row.venue)
                    && candidate_pair
                        .partner_symbol
                        .eq_ignore_ascii_case(&row.symbol)
                    && candidate_pair.partner_side == row.side
            })
    })
}

const fn switch_label(enabled: bool) -> &'static str {
    if enabled {
        "开启"
    } else {
        "关闭"
    }
}

fn valid_distance(value: f64) -> Option<f64> {
    // 后端距离已有符号化：负值 = 已越过强平价，是有效且最危险的证据
    //（min 选择会让其自然胜出），只过滤非有限值。
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests;
