use leptos::prelude::*;
use shared_types::{ListPage, MissedOpportunity};

use crate::panels::modules::pagination::list_page_controls;

use super::format::{minutes_ago, reason_label, signed_money, strategy_label};
use super::{attribution_chart, ReviewSectionRows};

pub(in crate::panels::modules::review) fn missed_tab(
    section: Memo<ReviewSectionRows<MissedOpportunity>>,
    page: Memo<Option<ListPage>>,
    page_loading: Memo<bool>,
    on_page: Callback<Option<String>>,
) -> impl IntoView {
    let rows = Memo::new(move |_| section.with(|section| section.rows.clone()));

    view! {
        {move || {
            let section = section.get();
            if section.rows.is_empty() {
                let loaded = section.has_loaded_context();
                view! {
                    <div class=if loaded { "review-business-empty" } else { "review-business-empty is-error" }>
                        <strong>{section.empty_text("30D 内暂无错失机会")}</strong>
                        <span>{if loaded { "没有已记录但未执行的机会。" } else { "当前没有可用快照，错误证据保留在上方数据状态中。" }}</span>
                    </div>
                }.into_any()
            } else {
                view! {
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
                                {move || rows.get().into_iter().map(|row| view! { <MissedRow row=row/> }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                    <div class="table-pager-bar">
                        {list_page_controls(page, page_loading, on_page)}
                    </div>
                }.into_any()
            }
        }}
    }
}

#[component]
fn MissedRow(row: MissedOpportunity) -> impl IntoView {
    view! {
        <tr>
            <td><strong>{row.symbol}</strong><small>{strategy_label(row.strategy)} " · " {row.opportunity_id}</small></td>
            <td>{minutes_ago(row.detected_at_ms)}</td>
            <td><strong>{signed_money(row.expected_pnl_usd)}</strong><small>"未实现"</small></td>
            <td><span class="reason-pill">{reason_label(row.reason)}</span></td>
            <td class="review-missed-detail">{row.detail}</td>
        </tr>
    }
}
