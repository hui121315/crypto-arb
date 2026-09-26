use leptos::prelude::*;
use shared_types::{
    AccountFieldQualityStatus, ExecutionLedgerQuality, PortfolioNavBreakdown, PortfolioPnlEvidence,
    PortfolioSummary, PortfolioValueEvidence, ReviewPnlField,
};

use super::super::data::PortfolioAccountAccess;
use super::format::{money, signed_money, signed_pct};
use super::section_state::SectionData;

#[path = "summary_cards/card.rs"]
mod card;
use card::{card, state_card};

#[derive(Clone, Copy)]
pub(super) enum Tone {
    Neutral,
    Profit,
    Loss,
    Warning,
    Danger,
}

impl Tone {
    pub(super) fn class(self) -> &'static str {
        match self {
            Tone::Neutral => "summary-card neutral",
            Tone::Profit => "summary-card profit",
            Tone::Loss => "summary-card loss",
            Tone::Warning => "summary-card warning",
            Tone::Danger => "summary-card danger",
        }
    }
}

pub(in crate::panels::modules::positions) fn summary_cards(
    summary: Memo<SectionData<Option<PortfolioSummary>>>,
    account_access: Memo<PortfolioAccountAccess>,
    position_values_known: Memo<bool>,
) -> impl IntoView {
    view! {
        <div class="portfolio-summary">
            {move || {
                let section = summary.get();
                let access = account_access.get();
                let account_unavailable = access.account_data_unavailable();
                match section.value {
                    Some(s) => render_cards(
                        &s,
                        (!account_unavailable)
                            .then(|| section.status.stale_note("账户概览刷新失败，显示上次快照"))
                            .flatten(),
                        account_unavailable,
                        position_values_known.get(),
                    ),
                    None => view! {
                        <div class="summary-cards">
                            {state_card(section.status.empty_text(
                                "暂无账户概览",
                                "读取账户概览中",
                                "账户概览读取失败",
                            ))}
                        </div>
                    }.into_any(),
                }
            }}
        </div>
    }
}

pub(in crate::panels::modules::positions) fn nav_breakdown_panel(
    summary: Memo<SectionData<Option<PortfolioSummary>>>,
    account_access: Memo<PortfolioAccountAccess>,
) -> impl IntoView {
    view! {
        <div class="positions-nav-breakdown-panel">
            {move || {
                if account_access.get().account_data_unavailable() {
                    return view! {
                        <div class="nav-breakdown-state">
                            <strong>"净值组成等待账户接入"</strong>
                            <span>"钱包权益、持仓权益、现金残差与未实现 PnL 保持未知。"</span>
                        </div>
                    }
                    .into_any();
                }
                let section = summary.get();
                section.value.map_or_else(
                    || view! {
                        <div class="nav-breakdown-state">
                            <strong>"净值组成暂不可用"</strong>
                            <span>{section.status.empty_text(
                                "暂无账户概览",
                                "读取账户概览中",
                                "账户概览读取失败",
                            )}</span>
                        </div>
                    }
                    .into_any(),
                    |summary| nav_breakdown(summary.nav_evidence.breakdown),
                )
            }}
        </div>
    }
}

fn render_cards(
    s: &PortfolioSummary,
    stale_note: Option<String>,
    account_unavailable: bool,
    positions_known: bool,
) -> AnyView {
    let nav_actual = !account_unavailable
        && s.nav_evidence.status == AccountFieldQualityStatus::Actual
        && s.total_nav_usd.is_finite();
    let nav_tone = if account_unavailable {
        Tone::Warning
    } else if !nav_actual {
        Tone::Danger
    } else {
        Tone::Neutral
    };
    let delta_abs = s.net_delta_pct_of_nav.abs();
    let delta_tone = if account_unavailable || !positions_known {
        Tone::Warning
    } else if !nav_actual {
        Tone::Danger
    } else if delta_abs < 1.0 {
        Tone::Neutral
    } else if delta_abs < 5.0 {
        Tone::Warning
    } else {
        Tone::Danger
    };
    let naked_tone = if account_unavailable || !positions_known {
        Tone::Warning
    } else if s.naked_position_count == 0 {
        Tone::Neutral
    } else {
        Tone::Warning
    };
    let nav_value = if nav_actual {
        money(s.total_nav_usd)
    } else {
        "未知".to_owned()
    };
    let nav_sub = if account_unavailable {
        "等待账户凭证".to_owned()
    } else if let Some(change) = s.nav_change_24h_pct.filter(|v| nav_actual && v.is_finite()) {
        format!("24h 净值变动 {}", signed_pct(change))
    } else if nav_actual {
        "24h 净值变动待确认".to_owned()
    } else {
        missing_nav_label(s)
    };
    let delta_value = if account_unavailable || !positions_known || !s.net_delta_usd.is_finite() {
        "未知".to_owned()
    } else {
        signed_money(s.net_delta_usd)
    };
    let delta_sub = if account_unavailable {
        "等待持仓权限".to_owned()
    } else if !positions_known {
        "持仓数据待确认".to_owned()
    } else if nav_actual {
        format!("{:+.1}% NAV", s.net_delta_pct_of_nav)
    } else {
        "账户净值 口径缺失".to_owned()
    };
    let pnl_tone = match s.pnl_breakdown.evidence.quality {
        ExecutionLedgerQuality::Missing => Tone::Danger,
        ExecutionLedgerQuality::Estimated => Tone::Warning,
        ExecutionLedgerQuality::Actual if s.realized_pnl_today_usd >= 0.0 => Tone::Profit,
        ExecutionLedgerQuality::Actual => Tone::Loss,
    };
    let naked_value =
        if account_unavailable || !positions_known || !s.naked_exposure_usd.is_finite() {
            "未知".to_owned()
        } else {
            money(s.naked_exposure_usd)
        };
    let naked_sub = if account_unavailable {
        "等待持仓权限".to_owned()
    } else if !positions_known {
        "不能按空仓计算".to_owned()
    } else {
        format!("{} 个未配对", s.naked_position_count)
    };

    view! {
        <div class="summary-cards">
            {card("账户净值", nav_value, nav_sub, nav_tone)}
            {card("净 Delta", delta_value, delta_sub, delta_tone)}
            {card("裸单暴露", naked_value, naked_sub, naked_tone)}
            {card(
                "当日已实现 PnL",
                realized_pnl_value(s),
                pnl_evidence_label(&s.pnl_breakdown.evidence),
                pnl_tone,
            )}
        </div>
        {stale_note.map(stale_snapshot_status)}
    }
    .into_any()
}

fn stale_snapshot_status(message: String) -> AnyView {
    view! {
        <details class="portfolio-summary-stale">
            <summary>
                <strong>"账户概览使用上次快照"</strong>
                <span>"查看原因"</span>
            </summary>
            <p>{message}</p>
        </details>
    }
    .into_any()
}

fn nav_breakdown(breakdown: PortfolioNavBreakdown) -> AnyView {
    let rows = [
        ("钱包权益", breakdown.wallet_equity, false),
        ("持仓权益", breakdown.position_equity, false),
        ("现金残差", breakdown.cash, false),
        ("未实现 PnL", breakdown.unrealized_pnl, true),
    ];
    view! {
        <details class="nav-breakdown-disclosure">
            <summary>
                <span>"净值组成与口径"</span>
                <em>"4 项账户数据依据"</em>
            </summary>
            <dl class="nav-breakdown" aria-label="账户净值组成">
                {rows.into_iter().map(|(label, evidence, signed)| {
                    let class = format!("nav-breakdown-item {}", value_status_class(evidence.status));
                    let value = evidence_value(&evidence, signed);
                    let detail = evidence_detail(&evidence);
                    view! {
                        <div class=class>
                            <dt>{label}</dt>
                            <dd class="num">{value}</dd>
                            <small>{detail}</small>
                        </div>
                    }
                }).collect_view()}
            </dl>
        </details>
    }
    .into_any()
}

fn evidence_value(evidence: &PortfolioValueEvidence, signed: bool) -> String {
    evidence
        .value_usd
        .filter(|v| {
            v.is_finite()
                && matches!(
                    evidence.status,
                    AccountFieldQualityStatus::Actual | AccountFieldQualityStatus::Estimated
                )
        })
        .map_or_else(
            || "未知".to_owned(),
            |value| {
                if signed {
                    signed_money(value)
                } else {
                    money(value)
                }
            },
        )
}

fn realized_pnl_value(summary: &PortfolioSummary) -> String {
    match summary.pnl_breakdown.evidence.quality {
        ExecutionLedgerQuality::Missing => "未知".to_owned(),
        _ if !summary.realized_pnl_today_usd.is_finite() => "未知".to_owned(),
        ExecutionLedgerQuality::Estimated => {
            format!("约 {}", signed_money(summary.realized_pnl_today_usd))
        }
        ExecutionLedgerQuality::Actual => signed_money(summary.realized_pnl_today_usd),
    }
}

fn evidence_detail(evidence: &PortfolioValueEvidence) -> String {
    let mut parts = vec![
        value_status_label(evidence.status).to_owned(),
        evidence.source.clone(),
    ];
    if let Some(problem) = evidence.problem.as_ref() {
        parts.push(problem.message.clone());
    }
    parts.join(" · ")
}

fn value_status_label(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "实际",
        AccountFieldQualityStatus::Estimated => "估算",
        AccountFieldQualityStatus::Unknown => "未知",
        AccountFieldQualityStatus::Invalid => "无效",
        AccountFieldQualityStatus::Missing => "缺失",
    }
}

fn value_status_class(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "is-actual",
        AccountFieldQualityStatus::Estimated => "is-estimated",
        AccountFieldQualityStatus::Unknown
        | AccountFieldQualityStatus::Invalid
        | AccountFieldQualityStatus::Missing => "is-missing",
    }
}

fn pnl_evidence_label(evidence: &PortfolioPnlEvidence) -> String {
    let source = if evidence.source.contains("sql_realized_window") {
        "SQL 账本"
    } else {
        "执行账本"
    };
    let field_detail = if evidence.quality == ExecutionLedgerQuality::Missing
        && evidence.missing_fields.is_empty()
    {
        "账本数据依据待确认".to_owned()
    } else if !evidence.missing_fields.is_empty() {
        format!("缺失 {}", pnl_fields(&evidence.missing_fields))
    } else if !evidence.estimated_fields.is_empty() {
        format!("估算 {}", pnl_fields(&evidence.estimated_fields))
    } else {
        "字段实际".to_owned()
    };
    let run_detail = match (evidence.close_run_count, evidence.unwind_run_count) {
        (0, _) => format!("{} 组", evidence.realized_group_count),
        (close, 0) => format!("{} 组 · {} 次平仓", evidence.realized_group_count, close),
        (close, unwind) => format!(
            "{} 组 · {} 次平仓 / {} 次补偿",
            evidence.realized_group_count, close, unwind
        ),
    };
    format!("{field_detail} · {source} · {run_detail}")
}

fn pnl_fields(fields: &[ReviewPnlField]) -> String {
    fields
        .iter()
        .map(|field| match field {
            ReviewPnlField::Gross => "价差",
            ReviewPnlField::Fee => "费用",
            ReviewPnlField::Funding => "资金费",
            ReviewPnlField::Slippage => "滑点",
            ReviewPnlField::Net => "净额",
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn missing_nav_label(summary: &PortfolioSummary) -> String {
    if summary.nav_evidence.missing_venues.is_empty() {
        "账户权益数据依据缺失".to_owned()
    } else {
        format!("缺少 {}", summary.nav_evidence.missing_venues.join(" / "))
    }
}

#[cfg(test)]
#[path = "summary_cards_tests.rs"]
mod tests;
