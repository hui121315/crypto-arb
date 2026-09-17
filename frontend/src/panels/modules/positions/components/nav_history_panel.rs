use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::ApiProblem;

use super::super::data::PortfolioAccountAccess;
use super::account_setup::account_data_placeholder;
use super::format::money;

#[path = "nav_history_panel/derive.rs"]
mod derive;
use derive::{
    history_chips, history_diagnostics, history_notice, nav_points, nav_trend, problem_message,
    Chip, HistoryDiagnostic, HistoryNotice, NavHistoryResponse, NavPoint, NavTrend, CHART_HEIGHT,
    CHART_WIDTH,
};

pub(in crate::panels::modules::positions) fn nav_history_panel(
    history: RwSignal<LoadState<NavHistoryResponse>>,
    account_access: Memo<PortfolioAccountAccess>,
) -> impl IntoView {
    view! {
        <div class="balance-panel">
            {move || if account_access.get().account_data_unavailable() {
                account_data_placeholder(
                    "NAV 历史等待账户接入",
                    "取得实际账户权益后才会写入可信 NAV 样本。",
                )
            } else {
                render_history(history.get())
            }}
        </div>
    }
}

fn render_history(state: LoadState<NavHistoryResponse>) -> AnyView {
    match state {
        LoadState::Loading => state_note("读取仓位权益历史中".to_owned()),
        LoadState::Error(problem) => state_note(format!(
            "仓位权益历史读取失败：{}",
            problem_message(&problem)
        )),
        LoadState::Ready(response) => render_response(&response, None),
        LoadState::Stale { value, problem } => render_response(&value, Some(&problem)),
    }
}

fn render_response(response: &NavHistoryResponse, stale_problem: Option<&ApiProblem>) -> AnyView {
    let chips = history_chips(response);
    let notice = history_notice(response, stale_problem);
    let diagnostics = history_diagnostics(response, stale_problem);
    let points = nav_points(&response.rows);
    match nav_trend(&points) {
        Some(trend) => render_trend(trend, chips, notice, diagnostics),
        None => render_empty(chips, notice, diagnostics, points.first()),
    }
}

fn render_trend(
    trend: NavTrend,
    chips: Vec<Chip>,
    notice: Option<HistoryNotice>,
    diagnostics: Vec<HistoryDiagnostic>,
) -> AnyView {
    view! {
        {render_chips(chips)}
        <div class="risk-meter-row">
            <span>"仓位权益历史"</span>
            <svg
                class="sparkline"
                width="100%"
                height=CHART_HEIGHT
                viewBox=format!("0 0 {CHART_WIDTH} {CHART_HEIGHT}")
                preserveAspectRatio="none"
                role="img"
                aria-label="仓位权益历史趋势"
            >
                <path
                    d=format!("M 0 {} L {} {}", CHART_HEIGHT - 1, CHART_WIDTH, CHART_HEIGHT - 1)
                    fill="none"
                    stroke="var(--color-border)"
                    stroke-width="1"
                    vector-effect="non-scaling-stroke"
                />
                <path
                    d=trend.path
                    fill="none"
                    stroke=trend.stroke
                    stroke-width="2"
                    vector-effect="non-scaling-stroke"
                />
            </svg>
            <div>
                <span>"最新"</span>
                <em class="num">{trend.latest}</em>
            </div>
            <div>
                <span>{trend.sample}</span>
                <em class="num">{trend.change}</em>
            </div>
        </div>
        {render_history_notice(notice)}
        {render_history_diagnostics(diagnostics)}
    }
    .into_any()
}

fn render_empty(
    chips: Vec<Chip>,
    notice: Option<HistoryNotice>,
    diagnostics: Vec<HistoryDiagnostic>,
    point: Option<&NavPoint>,
) -> AnyView {
    let text = point
        .map(|point| format!("只有 1 个样本 · 最新 {}", money(point.nav_usd)))
        .unwrap_or_else(|| "暂无仓位权益历史样本".to_owned());
    let show_empty_note = point.is_some() || notice.is_none();
    view! {
        {render_chips(chips)}
        {render_history_notice(notice)}
        {if show_empty_note {
            view! { <div class="risk-empty nav-history-empty">{text}</div> }.into_any()
        } else {
            ().into_any()
        }}
        {render_history_diagnostics(diagnostics)}
    }
    .into_any()
}

fn render_chips(chips: Vec<Chip>) -> AnyView {
    if chips.is_empty() {
        return ().into_any();
    }
    view! {
        <div class="balance-evidence">
            {chips.into_iter().map(|chip| view! {
                <span class=chip.class_name>{chip.label}</span>
            }).collect_view()}
        </div>
    }
    .into_any()
}

fn render_history_notice(notice: Option<HistoryNotice>) -> AnyView {
    let Some(notice) = notice else {
        return ().into_any();
    };
    let class = format!("nav-history-notice {}", notice.tone);
    view! {
        <div class=class>
            <div>
                <strong>{notice.title}</strong>
                <span>{notice.message}</span>
            </div>
            <div class="nav-history-facts">
                {notice.facts.into_iter().map(|fact| view! {
                    <span>{fact}</span>
                }).collect_view()}
            </div>
        </div>
    }
    .into_any()
}

fn render_history_diagnostics(rows: Vec<HistoryDiagnostic>) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    let count = rows.len();
    view! {
        <details class="nav-history-diagnostics">
            <summary>
                <span>"技术诊断"</span>
                <em>{count}</em>
            </summary>
            <div>
                {rows.into_iter().map(|row| view! {
                    <div class="nav-history-diagnostic-row">
                        <span>{row.label}</span>
                        <code>{row.value}</code>
                    </div>
                }).collect_view()}
            </div>
        </details>
    }
    .into_any()
}

fn state_note(text: String) -> AnyView {
    view! { <div class="risk-empty">{text}</div> }.into_any()
}
