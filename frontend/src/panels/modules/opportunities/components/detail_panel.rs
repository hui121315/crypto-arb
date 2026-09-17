use crate::panels::modules::funding_stats::funding_cycle_trend;
use crate::panels::modules::index_composition::index_composition_detail;
use crate::panels::modules::opportunities::data::{
    with_problem_context_label, OpportunityDetail, OpportunityDetailSnapshot,
    OpportunityDetailState,
};
use crate::panels::modules::opportunity_format::evidence_profit_class;
use crate::panels::shared::RiskBadge;
use crate::state::arbitrage_stream::stream_problem_label;
use crate::state::load_state::LoadState;
use leptos::prelude::*;

const DETAIL_LIST_LIMIT: usize = 24;

pub(in crate::panels::modules::opportunities) fn detail_panel(
    detail: RwSignal<OpportunityDetailState>,
) -> impl IntoView {
    view! {
        <aside
            id="opportunity-detail-panel"
            class="opportunity-detail"
            aria-label="当前候选证据"
            tabindex="-1"
        >
            {move || {
                detail_state_view(detail.get())
            }}
        </aside>
    }
}

fn detail_state_view(state: OpportunityDetailState) -> AnyView {
    match state {
        LoadState::Ready(OpportunityDetailSnapshot::Selected(detail)) => detail_view(*detail),
        LoadState::Stale {
            value: OpportunityDetailSnapshot::Selected(detail),
            problem,
        } => view! {
            {problem_banner("详情部分失败", &problem)}
            {detail_view(*detail)}
        }
        .into_any(),
        LoadState::Error(problem) => problem_view(&problem),
        LoadState::Loading => loading_view(),
        LoadState::Ready(OpportunityDetailSnapshot::Unselected) => empty_view(),
        LoadState::Stale {
            value: OpportunityDetailSnapshot::Unselected,
            problem,
        } => problem_view(&problem),
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

fn detail_view(detail: OpportunityDetail) -> AnyView {
    let book_count = detail.books.len();
    let history_count = detail.history.len();
    let books = detail
        .books
        .iter()
        .take(DETAIL_LIST_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let history = detail
        .history
        .iter()
        .take(DETAIL_LIST_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let section_evidence = detail.section_evidence.clone();
    let evidence_count = section_evidence.len();
    let book_truncation = truncation_text(book_count);
    let history_truncation = truncation_text(history_count);
    let funding_summary = detail.funding_stats.percentile_text();
    let index_summary = detail.index_composition.compact_text();
    let (net_label, net_value, net_class) = if detail.execution_eligible {
        (
            "费后净利",
            detail.one_cycle_net.clone(),
            evidence_profit_class(detail.cost_verified, detail.one_cycle_net_bps),
        )
    } else {
        (
            "测算边际",
            format!("{} · 不可实现", detail.one_cycle_net),
            "muted",
        )
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
                {if detail.cost_verified { "成本已核验" } else { "成本待核验" }}
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
        <details class="opportunity-detail-section">
            <summary><span>"Funding 周期"</span><em>{funding_summary}</em></summary>
            <div class="opportunity-detail-section-body">
                {funding_cycle_trend(detail.funding_stats.clone())}
            </div>
        </details>
        <details class="opportunity-detail-section">
            <summary><span>"指数成分"</span><em>{index_summary}</em></summary>
            <div class="opportunity-detail-section-body">
                {index_composition_detail(detail.index_composition_detail)}
            </div>
        </details>
        <details class="opportunity-detail-section">
            <summary><span>"数据证据"</span><em>{format!("{evidence_count} 组")}</em></summary>
            <div class="opportunity-detail-section-body detail-list">
                {section_evidence.into_iter().map(|row| {
                    let problem_context = row.problem.as_ref().map(|problem| {
                        with_problem_context_label("问题", problem)
                    });
                    view! {
                    <div class="detail-row">
                        <strong>{row.section}</strong>
                        <span>
                            {row.source}
                            <small>{row.freshness}" · 请求 "{row.request_id}</small>
                            {problem_context.map(|text| view! { <small>{text}</small> })}
                        </span>
                        <em>"重试 "{row.retry_after}</em>
                    </div>
                    }
                }).collect_view()}
            </div>
        </details>
        <details class="opportunity-detail-section">
            <summary><span>"订单簿"</span><em>{format!("{book_count} 路")}</em></summary>
            <div class="opportunity-detail-section-body detail-list">
                {books.into_iter().map(|row| view! {
                    <div class="detail-row">
                        <strong>{row.venue}</strong>
                        <span>{row.bid}" / "{row.ask}<small>{row.health}</small></span>
                        <em>{row.spread}</em>
                    </div>
                }).collect_view()}
                {book_truncation.map(|text| view! { <em class="settings-message">{text}</em> })}
            </div>
        </details>
        <details class="opportunity-detail-section">
            <summary><span>"历史"</span><em>{format!("{history_count} 条")}</em></summary>
            <div class="opportunity-detail-section-body detail-list">
                <p class="settings-message">{detail.history_health}</p>
                {history.into_iter().map(|row| view! {
                    <div class="detail-row">
                        <strong>{row.time}</strong>
                        <span>{row.route}<small>{row.health}</small></span>
                        <em>{row.edge}</em>
                    </div>
                }).collect_view()}
                {history_truncation.map(|text| view! { <em class="settings-message">{text}</em> })}
            </div>
        </details>
    }
    .into_any()
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
