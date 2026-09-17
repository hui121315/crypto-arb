use leptos::prelude::*;
use shared_types::{AccountFieldQualityStatus, PositionRow, RiskSnapshot};

use super::super::super::data::PortfolioAccountAccess;
use super::super::account_setup::account_data_placeholder;
use super::super::format::{pct, signed_money};
use super::super::section_state::SectionData;
use super::var_display;

pub(in crate::panels::modules::positions) fn risk_summary_panel(
    snapshot: Memo<SectionData<Option<RiskSnapshot>>>,
    positions: Memo<SectionData<Vec<PositionRow>>>,
    nav_evidence_status: Memo<Option<AccountFieldQualityStatus>>,
    account_access: Memo<PortfolioAccountAccess>,
    open_evidence: Callback<()>,
    open_controls: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="compact-risk-panel">
            {move || {
                if account_access.get().account_data_unavailable() {
                    return account_data_placeholder(
                        "风险摘要等待账户接入",
                        "取得实际账户权益与持仓后显示当前风险边界。",
                    );
                }
                let section = snapshot.get();
                match section.value {
                    Some(snapshot) => render_risk_summary(
                        &snapshot,
                        &positions.get().value,
                        nav_evidence_status.get(),
                        section.status.stale_note("风险摘要刷新失败，显示上次快照"),
                        open_evidence,
                        open_controls,
                    ),
                    None => view! {
                        <div class="risk-empty compact-risk-empty">
                            {section.status.empty_text(
                                "暂无风险快照",
                                "读取风险快照中",
                                "风险快照读取失败",
                            )}
                        </div>
                    }
                    .into_any(),
                }
            }}
        </div>
    }
}

fn render_risk_summary(
    snapshot: &RiskSnapshot,
    positions: &[PositionRow],
    nav_evidence_status: Option<AccountFieldQualityStatus>,
    stale_note: Option<String>,
    open_evidence: Callback<()>,
    open_controls: Callback<()>,
) -> AnyView {
    let (var_value, var_detail, var_tone) = var_display(
        snapshot.var_99_1d_usd,
        snapshot.var_pct_of_nav,
        snapshot.var_sample_size,
        nav_evidence_status,
    );
    let (liquidation_value, liquidation_detail, liquidation_tone) =
        nearest_liquidation_summary(positions);
    let (margin_value, margin_detail, margin_tone) = margin_summary(snapshot);
    let (funding_value, funding_detail, funding_tone) = funding_summary(snapshot);
    let (kill_value, kill_detail, kill_tone) = if snapshot.hard_limits.kill_switch_active {
        ("已触发".to_owned(), "写入路径已阻断".to_owned(), "negative")
    } else {
        ("未触发".to_owned(), "当前总闸关闭".to_owned(), "positive")
    };

    view! {
        {stale_note.map(|note| view! { <div class="compact-risk-note">{note}</div> })}
        <div class="compact-risk-list">
            {compact_risk_row("VaR-99", var_detail, var_value, var_tone)}
            {compact_risk_row(
                "最近强平",
                liquidation_detail,
                liquidation_value,
                liquidation_tone,
            )}
            {compact_risk_row("保证金占用", margin_detail, margin_value, margin_tone)}
            {compact_risk_row("临近 Funding", funding_detail, funding_value, funding_tone)}
            {compact_risk_row("Kill switch", kill_detail, kill_value, kill_tone)}
        </div>
        <div class="compact-risk-actions" role="group" aria-label="风险摘要动作">
            <button
                type="button"
                class="compact-risk-action"
                aria-controls="positions-detail-risk"
                on:click=move |_| open_evidence.run(())
            >
                "风险证据"
            </button>
            <button
                type="button"
                class="compact-risk-action"
                aria-controls="positions-detail-controls"
                on:click=move |_| open_controls.run(())
            >
                "高级控制"
            </button>
        </div>
    }
    .into_any()
}

fn compact_risk_row(
    label: &'static str,
    detail: String,
    value: String,
    tone: &'static str,
) -> AnyView {
    view! {
        <div class="compact-risk-row">
            <span><strong>{label}</strong><em>{detail}</em></span>
            <b class=format!("compact-risk-value {tone}")>{value}</b>
        </div>
    }
    .into_any()
}

fn nearest_liquidation_summary(rows: &[PositionRow]) -> (String, String, &'static str) {
    rows.iter()
        .filter_map(|row| {
            row.liquidation_distance_pct.map(|distance| {
                (
                    distance,
                    format!("{} · {}", row.symbol, row.venue.to_ascii_uppercase()),
                )
            })
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map_or_else(
            || ("未知".to_owned(), "等待强平距离证据".to_owned(), "muted"),
            |(distance, detail)| {
                let tone = if distance < 10.0 {
                    "negative"
                } else if distance < 25.0 {
                    "warning"
                } else {
                    "num"
                };
                (format!("{distance:.1}%"), detail, tone)
            },
        )
}

fn margin_summary(snapshot: &RiskSnapshot) -> (String, String, &'static str) {
    if let Some(venue) = snapshot
        .margin_utilization
        .iter()
        .find(|venue| venue.utilization_pct.is_none())
    {
        return (
            "未知".to_owned(),
            format!("{} · 等待账户权益", venue.venue.to_ascii_uppercase()),
            "muted",
        );
    }

    snapshot
        .margin_utilization
        .iter()
        .filter_map(|venue| venue.utilization_pct.map(|pct| (venue, pct)))
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map_or_else(
            || ("未知".to_owned(), "等待交易所权益证据".to_owned(), "muted"),
            |(venue, utilization_pct)| {
                let tone = if venue.estimated {
                    "warning"
                } else if utilization_pct >= 85.0 {
                    "negative"
                } else if utilization_pct >= 65.0 {
                    "warning"
                } else {
                    "num"
                };
                (
                    pct(utilization_pct),
                    format!(
                        "{} · {}",
                        venue.venue.to_ascii_uppercase(),
                        if venue.estimated {
                            "仓位口径估算"
                        } else {
                            "初始 / 账户权益"
                        }
                    ),
                    tone,
                )
            },
        )
}

fn funding_summary(snapshot: &RiskSnapshot) -> (String, String, &'static str) {
    if let Some(cluster) = snapshot
        .funding_clustering
        .iter()
        .filter(|cluster| cluster.position_count > 0)
        .min_by_key(|cluster| cluster.settles_in_minutes)
    {
        return (
            format!("≤{}m", cluster.settles_in_minutes),
            format!(
                "{} 仓位 · {}",
                cluster.position_count,
                signed_money(-cluster.total_outflow_usd),
            ),
            if cluster.total_outflow_usd > 0.0 {
                "warning"
            } else {
                "num"
            },
        );
    }

    snapshot
        .funding_clustering
        .iter()
        .map(|cluster| cluster.settles_in_minutes)
        .max()
        .map_or_else(
            || ("未知".to_owned(), "等待结算窗口证据".to_owned(), "muted"),
            |window| ("无结算".to_owned(), format!("未来 {window}m"), "positive"),
        )
}
