use leptos::prelude::*;
use shared_types::{ListPage, MissedOpportunity};

use crate::panels::modules::pagination::list_page_controls;

use super::format::{reason_label, record_time, signed_money, strategy_label};
use super::{attribution_chart, ReviewSectionRows};

pub(in crate::panels::modules::review) fn missed_tab(
    section: Memo<ReviewSectionRows<MissedOpportunity>>,
    page: Memo<Option<ListPage>>,
    page_loading: Memo<bool>,
    on_page: Callback<Option<String>>,
) -> impl IntoView {
    let rows = Memo::new(move |_| section.with(|section| section.rows.clone()));

    view! {
        <Show when=move || !rows.with(Vec::is_empty) fallback=move || {
            let section = section.get();
                let loaded = section.has_loaded_context();
                view! {
                    <div class=if loaded { "review-business-empty" } else { "review-business-empty is-error" }>
                        <strong>{section.empty_text("30D 内暂无错失机会")}</strong>
                        <span>{if loaded { "没有已记录但未执行的机会。" } else { "当前没有可用快照，错误证据保留在上方数据状态中。" }}</span>
                    </div>
                }
        }>
                    {attribution_chart(rows)}
                    <div class="table-wrap">
                        <table class="clean-table review-table review-missed-table" data-table-budget="server-page">
                            <thead>
                                <tr>
                                    <th>"机会"</th>
                                    <th>"发现时间"</th>
                                    <th>"当时预期 PnL"</th>
                                    <th>"错失原因"</th>
                                    <th>"记录细节"</th>
                                </tr>
                            </thead>
                            <tbody>
                                <For each=move || rows.get() key=|row| row.id.clone() children=move |initial| {
                                    let id = initial.id.clone();
                                    let row = Memo::new(move |_| rows.with(|rows| rows.iter().find(|row| row.id == id).cloned()).unwrap_or_else(|| initial.clone()));
                                    view! { <MissedRow row=row/> }
                                }/>
                            </tbody>
                        </table>
                    </div>
                    <div class="table-pager-bar">
                        {list_page_controls(page, page_loading, on_page)}
                    </div>
        </Show>
    }
}

#[component]
fn MissedRow(row: Memo<MissedOpportunity>) -> impl IntoView {
    view! {
        <tr>
            <td><strong>{move || row.get().symbol}</strong><small>{move || strategy_label(row.get().strategy)} " · " {move || row.get().opportunity_id}</small></td>
            <td>{move || record_time(row.get().detected_at_ms)}</td>
            <td><strong>{move || signed_money(row.get().expected_pnl_usd)}</strong><small>"未实现"</small></td>
            <td><span class="reason-pill">{move || reason_label(row.get().reason)}</span></td>
            <td class="review-missed-detail">{move || row.get().detail}</td>
        </tr>
    }
}
