use super::super::data::OpportunityRow;
use super::super::data::OPPORTUNITY_PAGE_SIZE;
use crate::panels::modules::opportunity_format::{
    evidence_profit_class, is_missing_quote_label, quote_price_line,
};
use crate::panels::modules::opportunity_view_model::OpportunityListViewModel;
use crate::panels::modules::pagination::server_page_controls;
use crate::panels::shared::RiskBadge;
use leptos::prelude::*;
use std::collections::HashSet;
use shared_types::OpportunityListPage;
use wasm_bindgen::JsCast;

#[path = "opportunity_table/row.rs"]
mod row;
use row::row_view;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::opportunities) struct OpportunityTableInput {
    pub(in crate::panels::modules::opportunities) opportunities: Memo<Vec<OpportunityRow>>,
    pub(in crate::panels::modules::opportunities) selected_id: RwSignal<String>,
    pub(in crate::panels::modules::opportunities) page: Memo<Option<OpportunityListPage>>,
    pub(in crate::panels::modules::opportunities) page_loading: Memo<bool>,
    pub(in crate::panels::modules::opportunities) empty_label: Memo<String>,
    pub(in crate::panels::modules::opportunities) quote_ready_ids: Memo<HashSet<String>>,
    pub(in crate::panels::modules::opportunities) on_page: Callback<Option<String>>,
    pub(in crate::panels::modules::opportunities) on_select: Callback<(usize, OpportunityRow)>,
    pub(in crate::panels::modules::opportunities) on_evidence: Callback<(usize, OpportunityRow)>,
    pub(in crate::panels::modules::opportunities) on_open: Callback<(usize, OpportunityRow)>,
}

pub(in crate::panels::modules::opportunities) fn opportunity_table(
    input: OpportunityTableInput,
) -> impl IntoView {
    let OpportunityTableInput {
        opportunities,
        selected_id,
        page,
        page_loading,
        empty_label,
        quote_ready_ids,
        on_page,
        on_select,
        on_evidence,
        on_open,
    } = input;
    let row_context = OpportunityRowContext {
        rows: opportunities,
        selected_id,
        quote_ready_ids,
        on_select,
        on_evidence,
        on_open,
    };
    view! {
        <div class="paged-table-panel">
            <div class="table-wrap paged-table-wrap">
                <table
                    class="clean-table opportunity-table"
                    data-table-budget="server-page"
                    aria-label="机会扫描候选"
                >
                    <caption class="sr-only">"机会扫描候选与当前数据依据选择"</caption>
                    <thead>
                        <tr>
                            <th>"市场"</th>
                            <th>"路由"</th>
                            <th>"毛边际"</th>
                            <th>"完整成本"</th>
                            <th>"边际判断"</th>
                            <th>"兑现周期"</th>
                            <th>"规模"</th>
                            <th>"动作"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                                opportunities
                                    .with(|rows| rows.is_empty())
                                    .then(|| view! {
                                    <tr><td colspan="8" class="empty-cell">{empty_label.get()}</td></tr>
                                })
                        }}
                        <For
                            each=move || {
                                opportunities.with(|rows| {
                                    rows.iter()
                                        .take(OPPORTUNITY_PAGE_SIZE)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                })
                            }
                            key=|row| row.id.clone()
                            children=move |initial| {
                                let id = initial.id.clone();
                                let current = Memo::new(move |_| {
                                    opportunities.with(|rows| rows.iter().enumerate()
                                        .find(|(_, row)| row.id == id).map(|(idx, row)| (idx, row.clone())))
                                });
                                row_view(current, initial, row_context)
                            }
                        />
                    </tbody>
                </table>
            </div>
            <div class="table-pager-bar">
                {server_page_controls(page, page_loading, on_page)}
            </div>
        </div>
    }
}

#[derive(Clone, Copy)]
struct OpportunityRowContext {
    rows: Memo<Vec<OpportunityRow>>,
    selected_id: RwSignal<String>,
    quote_ready_ids: Memo<HashSet<String>>,
    on_select: Callback<(usize, OpportunityRow)>,
    on_evidence: Callback<(usize, OpportunityRow)>,
    on_open: Callback<(usize, OpportunityRow)>,
}

fn row_navigation_target(key: &str, idx: usize, row_count: usize) -> Option<usize> {
    match key {
        "ArrowDown" if idx + 1 < row_count => Some(idx + 1),
        "ArrowUp" => idx.checked_sub(1),
        "Home" if idx > 0 => Some(0),
        "End" if idx + 1 < row_count => Some(row_count - 1),
        _ => None,
    }
}

fn focus_opportunity_row(idx: usize) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(element) = document.get_element_by_id(&format!("opportunity-row-{idx}")) else {
        return;
    };
    if let Some(element) = element.dyn_ref::<web_sys::HtmlElement>() {
        let _ = element.focus();
    }
}

#[component]
fn LegRouteLine(
    #[prop(into)] venue: String,
    #[prop(into)] leg: String,
    #[prop(into)] price: String,
    evidence: Option<String>,
) -> impl IntoView {
    // 行内只保留"方向 · 价格"；venue 已在上方 venue→venue 主行，腿内重复
    // 前缀去掉；证据链（新鲜度/来源/延迟）收进 title——详情面板的"数据证据"
    // 区已有完整版本，行内展开会把行高撑到 ~110px，一屏只能看 4-5 条机会。
    // 缺价时保留完整缺价原因（可执行性关键信息）。
    let price_line = leg_price_line(&price, evidence.as_deref());
    let full = match evidence
        .as_deref()
        .filter(|_| !is_missing_quote_label(&price))
    {
        Some(line) => format!("{leg} · {price_line} · {line}"),
        None => format!("{leg} · {price_line}"),
    };
    let action = leg
        .strip_prefix(venue.as_str())
        .map(str::trim_start)
        .filter(|rest| !rest.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| leg.clone());
    let inline = if is_missing_quote_label(&price) {
        price_line
    } else {
        price
    };
    view! {
        <span title=full>{action}" · "{inline}</span>
    }
}

fn leg_price_line(price: &str, evidence: Option<&str>) -> String {
    quote_price_line(price, evidence)
}

fn execution_title(row: &OpportunityListViewModel) -> String {
    if row.execution_eligible {
        "可进入对冲预览".into()
    } else {
        row.execution_blockers
            .first()
            .cloned()
            .unwrap_or_else(|| "当前不可执行".into())
    }
}

fn execution_reason(row: &OpportunityListViewModel) -> Option<&'static str> {
    row.execution_blocker_summary()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::modules::opportunity_format::{missing_quote_label, missing_quote_text};

    #[test]
    fn leg_price_line_explains_missing_price() {
        assert_eq!(
            leg_price_line(missing_quote_label(), Some("数据依据 限频 · WS")),
            format!("价格 {}", missing_quote_text(Some("数据依据 限频 · WS")))
        );
        assert_eq!(
            leg_price_line("-", None),
            format!("价格 {}", missing_quote_text(None))
        );
    }

    #[test]
    fn leg_price_line_keeps_real_price() {
        assert_eq!(leg_price_line("664.63", None), "价格 664.63");
    }

    #[test]
    fn one_cycle_class_mutes_unverified_cost() {
        assert_eq!(evidence_profit_class(false, 0.0), "muted");
    }

    #[test]
    fn row_navigation_stays_inside_the_visible_page() {
        assert_eq!(row_navigation_target("ArrowDown", 1, 50), Some(2));
        assert_eq!(row_navigation_target("ArrowUp", 1, 50), Some(0));
        assert_eq!(row_navigation_target("Home", 20, 50), Some(0));
        assert_eq!(row_navigation_target("End", 20, 50), Some(49));
        assert_eq!(row_navigation_target("ArrowUp", 0, 50), None);
        assert_eq!(row_navigation_target("ArrowDown", 49, 50), None);
    }
}
