use crate::panels::modules::funding_stats::funding_cycle_trend;
use crate::panels::modules::index_composition::index_composition_detail;
use crate::panels::modules::opportunities::data::{
    with_problem_context_label, BookLine, DetailEvidence, HistoryLine, OpportunityDetail, OpportunityDetailSnapshot,
};
use crate::panels::modules::opportunity_format::evidence_profit_class;
use crate::panels::shared::RiskBadge;
use crate::state::arbitrage_stream::stream_problem_label;
use crate::state::load_state::LoadState;
use leptos::prelude::*;

const DETAIL_LIST_LIMIT: usize = 24;

#[cfg(test)]
use crate::panels::modules::opportunities::data::OpportunityDetailState;

pub(in crate::panels::modules::opportunities) fn detail_panel(
    data: crate::panels::modules::opportunities::data::OpportunityDetailData,
    snapshot_usable: Memo<bool>,
    clock: RwSignal<(i64, i64)>,
) -> impl IntoView {
    let detail = data.state;
    view! {
        <aside id="opportunity-detail-panel" class="opportunity-detail" class:is-reference=move || !snapshot_usable.get()
            aria-label="当前候选数据依据" tabindex="-1">
            <div class="opportunity-detail-toolbar">
                <span>{move || if data.loading.get() { "数据依据读取中" } else { "当前候选数据依据" }}</span>
                <button type="button" class="btn-secondary"
                    disabled=move || data.loading.get() || detail.get().value().and_then(OpportunityDetailSnapshot::detail).is_none()
                    on:click=move |_| data.refresh.run(())>"刷新数据依据"</button>
            </div>
            {move || detail.get().problem().map(|problem| problem_banner("详情读取降级", problem))}
            <Show when=move || !snapshot_usable.get() && detail.get().value().and_then(OpportunityDetailSnapshot::detail).is_some()>
                <p class="settings-message is-error opportunity-detail-snapshot-status" role="status">
                    "候选报价待更新 · 上次测算仅供参考"
                </p>
            </Show>
            <For
                each=move || { detail.get().value().and_then(OpportunityDetailSnapshot::detail).cloned().into_iter().collect::<Vec<_>>() }
                key=|row| row.id.clone()
                children=move |initial| {
                    let selected = Memo::new(move |_| detail.get().value()
                        .and_then(OpportunityDetailSnapshot::detail)
                        .filter(|row| row.id == initial.id).cloned().unwrap_or_else(|| initial.clone()));
                    view! {
                        {move || selected.try_get().map(|detail| detail_metrics(detail, snapshot_usable.get()))}
                        {["资金费 周期", "指数成分", "数据数据依据", "订单簿", "历史"].into_iter().enumerate().map(|(idx, label)| {
                            let section = Memo::new(move |_| selected.try_get().map(|detail| DetailSection::from_detail(&detail, idx)));
                            view! {
                            <details class="opportunity-detail-section">
                                <summary><span>{label}</span><em>{move || selected.try_get().map(|detail| detail_section_summary(&detail, idx)).unwrap_or_default()}</em></summary>
                                <div class="opportunity-detail-section-body">
                                    {move || section.try_get().flatten().map(|section| detail_section_body(section, clock))}
                                </div>
                            </details>
                        }}).collect_view()}
                    }
                }
            />
            {move || if detail.get().value().and_then(OpportunityDetailSnapshot::detail).is_none() {
                Some(match detail.get() {
                    LoadState::Loading => loading_view(),
                    LoadState::Error(problem) | LoadState::Stale { problem, .. } => problem_view(&problem),
                    _ => empty_view(),
                })
            } else { None }}
        </aside>
    }
}

#[cfg(test)]
fn empty_detail_problem(state: &OpportunityDetailState) -> Option<&shared_types::ApiProblem> {
    match state {
        LoadState::Error(problem)
        | LoadState::Stale {
            value: OpportunityDetailSnapshot::Unselected,
            problem,
        } => Some(problem),
        LoadState::Ready(_)
        | LoadState::Stale {
            value: OpportunityDetailSnapshot::Selected(_),
            ..
        }
        | LoadState::Loading => None,
    }
}

fn detail_metrics(detail: OpportunityDetail, snapshot_usable: bool) -> AnyView {
    let funding_summary = detail.funding_stats.percentile_text();
    let index_summary = detail.index_composition.compact_text();
    let (net_label, net_value, net_class) = if !snapshot_usable {
        ("上次测算边际", detail.one_cycle_net.clone(), "muted")
    } else if detail.execution_eligible {
        (
            "费后净利",
            detail.one_cycle_net.clone(),
            evidence_profit_class(detail.cost_verified, detail.one_cycle_net_bps),
        )
    } else {
        ("测算边际", detail.one_cycle_net.clone(), "muted")
    };
    // scope 与 domain 常常同词（如"永续跨所"），拼接会渲染成"永续跨所 · 永续跨所"。
    let scope_line = if detail.market_scope == detail.domain {
        detail.market_scope.clone()
    } else {
        format!("{} · {}", detail.market_scope, detail.domain)
    };
    view! {
        <div class="detail-head">
            <div>
                <span>{scope_line}</span>
                <strong>{detail.pair.clone()}</strong>
            </div>
            <span class="evidence-badge" title=detail.round_trip_cost.clone()>
                {if !snapshot_usable { "上次成本数据依据" } else if detail.cost_verified { "成本已核对" } else { "成本待核对" }}
            </span>
        </div>
        <div class="detail-metrics">
            <div><span>"毛边际"</span><strong class="muted">{detail.gross_one_cycle}</strong></div>
            <div><span>"完整成本"</span><strong>{detail.round_trip_cost}</strong></div>
            <div><span>{net_label}</span><strong class=net_class>{net_value}</strong></div>
            <div><span>"周期分位"</span><strong>{funding_summary.clone()}</strong></div>
            <div><span>"指数成分"</span><strong title=detail.index_composition.detail.clone()>{index_summary.clone()}</strong></div>
            <div><span>"风险"</span><RiskBadge risk=detail.risk/></div>
        </div>
        <section>
            <h3>"策略说明"</h3>
            <p>{detail.reason}</p>
        </section>
    }
    .into_any()
}

fn detail_section_summary(detail: &OpportunityDetail, idx: usize) -> String {
    match idx {
        0 => detail.funding_stats.percentile_text(),
        1 => detail.index_composition.compact_text(),
        2 => format!("{} 组", detail.section_evidence.len()),
        3 => format!("{} 路", detail.books.len()),
        _ => format!("{} 条", detail.history.len()),
    }
}

#[derive(Clone, PartialEq)]
enum DetailSection {
    Funding(crate::panels::modules::funding_stats::FundingCycleStatsView),
    Index(crate::panels::modules::index_composition::IndexCompositionDetailView),
    Evidence(Vec<DetailEvidence>),
    Books(Vec<BookLine>, Vec<DetailEvidence>),
    History(Vec<HistoryLine>, String, Option<DetailEvidence>),
}

impl DetailSection {
    fn from_detail(detail: &OpportunityDetail, index: usize) -> Self {
        match index {
            0 => Self::Funding(detail.funding_stats.clone()),
            1 => Self::Index(detail.index_composition_detail.clone()),
            2 => Self::Evidence(detail.section_evidence.clone()),
            3 => Self::Books(detail.books.clone(), detail.section_evidence.iter().take(2).cloned().collect()),
            _ => Self::History(detail.history.clone(), detail.history_health.clone(), detail.section_evidence.get(2).cloned()),
        }
    }
}

fn detail_section_body(section: DetailSection, clock: RwSignal<(i64, i64)>) -> AnyView {
    match section {
        DetailSection::Funding(stats) => funding_cycle_trend(stats).into_any(),
        DetailSection::Index(index) => index_composition_detail(index).into_any(),
        DetailSection::Evidence(evidence) => view! {
            <div class="detail-list">
                {evidence.into_iter().map(|row| {
                    let section = row.section.clone();
                    view! {
                        <div class="detail-row detail-evidence-row">
                            <strong>{section}</strong>
                            {evidence_caption(row, clock)}
                        </div>
                    }
                }).collect_view()}
            </div>
        }.into_any(),
        DetailSection::Books(books, evidence) => view! {
            <div class="detail-list">
                {books.iter().take(DETAIL_LIST_LIMIT).cloned().enumerate().map(|(index, row)| {
                    let evidence = evidence.get(index).cloned();
                    let role = if index == 0 { "多腿" } else { "空腿" };
                    view! {
                        <div class="detail-row detail-book-row"><strong>{row.venue}<small>{role}</small></strong>
                            <em>"价差 "{row.spread}</em>
                            <div class="detail-book-quotes">
                                <span><small>"买一"</small><b>{row.bid}</b></span>
                                <span><small>"卖一"</small><b>{row.ask}</b></span>
                            </div>
                            <div class="detail-book-evidence">
                                {evidence.map(|item| evidence_caption(item, clock))}
                            </div>
                        </div>
                    }
                }).collect_view()}
                {truncation_text(books.len()).map(|text| view! { <em class="settings-message">{text}</em> })}
            </div>
        }.into_any(),
        DetailSection::History(history, health, evidence) => view! {
            <div class="detail-list">
                {evidence.map(|item| evidence_caption(item, clock))}
                <p class="settings-message">"读取时："{health}</p>
                {history.is_empty().then(|| view! { <p class="settings-message">"没有可展示的历史记录"</p> })}
                {history.iter().take(DETAIL_LIST_LIMIT).cloned().map(|row| view! {
                    <div class="detail-row"><strong>{row.time}</strong>
                        <span title=format!("读取时：{}", row.health)>{row.route}</span><em>{row.edge}</em>
                    </div>
                }).collect_view()}
                {truncation_text(history.len()).map(|text| view! { <em class="settings-message">{text}</em> })}
            </div>
        }.into_any(),
    }
}

fn evidence_caption(row: DetailEvidence, clock: RwSignal<(i64, i64)>) -> AnyView {
    let request = row.data_request_id().to_owned();
    let status = if row.retained.is_some() { format!("保留旧值 · 本次{}", row.status) } else { row.status.clone() };
    let latest_request = row.retained.as_ref().map(|_| row.request_id.clone());
    let latest_source = row.source.clone();
    let problem = row.problem.as_ref().map(|problem| with_problem_context_label(&problem.message, problem));
    let retry = (row.retry_after != "-").then(|| format!("建议重试间隔 {}", row.retry_after));
    view! {
        <div class="detail-evidence-caption">
            <span class="detail-evidence-age">{move || row.data_label_at(clock.get())}</span>
            <small>{status}</small>
            <details class="detail-evidence-context">
                <summary>"读取详情"</summary>
                <small>"数据请求 "{request}</small>
                <small>"本次读取来源 "{latest_source}</small>
                {latest_request.map(|request| view! { <small>"本次请求 "{request}</small> })}
                {retry.map(|text| view! { <small>{text}</small> })}
                {problem.map(|text| view! { <small class="settings-message is-error">{text}</small> })}
            </details>
        </div>
    }.into_any()
}

fn problem_banner(prefix: &str, problem: &shared_types::ApiProblem) -> AnyView {
    let message = format!("{prefix} · {}", stream_problem_label(problem));
    view! {
        <p class="settings-message is-error">
            {message}
        </p>
    }
    .into_any()
}

fn problem_view(problem: &shared_types::ApiProblem) -> AnyView {
    view! {
        <div class="detail-empty">
            {problem_banner("详情获取失败", problem)}
        </div>
    }
    .into_any()
}

fn truncation_text(total: usize) -> Option<String> {
    let hidden = total.saturating_sub(DETAIL_LIST_LIMIT);
    (hidden > 0).then(|| format!("已截断 {hidden} 条"))
}

fn loading_view() -> AnyView {
    view! {
        <div class="detail-empty">
            <strong>"详情加载中"</strong>
        </div>
    }
    .into_any()
}

fn empty_view() -> AnyView {
    view! {
        <div class="detail-empty">
            <strong>"选择一条机会"</strong>
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_empty_detail_state_keeps_problem_visible() {
        let state = LoadState::Stale {
            value: OpportunityDetailSnapshot::Unselected,
            problem: shared_types::ApiProblem::new("DETAIL_RATE_LIMITED", "rate limited")
                .with_request_id(Some("req-detail-1".into()))
                .with_retry_after_ms(Some(2_000)),
        };

        let problem = empty_detail_problem(&state);

        assert_eq!(
            problem.as_ref().map(|problem| problem.code.as_str()),
            Some("DETAIL_RATE_LIMITED")
        );
        assert_eq!(
            problem
                .as_ref()
                .and_then(|problem| problem.request_id.as_deref()),
            Some("req-detail-1")
        );
        assert_eq!(
            problem.as_ref().and_then(|problem| problem.retry_after_ms),
            Some(2_000)
        );
    }

    #[test]
    fn ready_empty_detail_state_stays_selection_prompt() {
        let state = LoadState::Ready(OpportunityDetailSnapshot::Unselected);

        assert!(empty_detail_problem(&state).is_none());
    }
}
