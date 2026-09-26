//! 期货机会分页表格组件（表头/行/详情/单元格渲染）。
//! 单元格纯文案派生见 `format.rs`。

#[path = "opportunity_table/format.rs"]
mod format;
#[path = "opportunity_table/rows.rs"]
mod rows;

use leptos::prelude::*;
use std::collections::HashSet;

use super::super::columns::ColumnId;
use super::super::data::FuturesOpportunityRow;
use crate::panels::modules::pagination::server_page_controls;
use shared_types::OpportunityListPage;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::futures) struct FuturesOpportunityTableInput {
    pub(in crate::panels::modules::futures) opportunities: Memo<Vec<FuturesOpportunityRow>>,
    pub(in crate::panels::modules::futures) visible: RwSignal<Vec<ColumnId>>,
    pub(in crate::panels::modules::futures) page: Memo<Option<OpportunityListPage>>,
    pub(in crate::panels::modules::futures) page_loading: Memo<bool>,
    pub(in crate::panels::modules::futures) empty_label: Memo<String>,
    pub(in crate::panels::modules::futures) on_page: Callback<Option<String>>,
    pub(in crate::panels::modules::futures) on_build: Callback<FuturesOpportunityRow>,
    pub(in crate::panels::modules::futures) quote_ready_ids: Memo<HashSet<String>>,
}

pub(in crate::panels::modules::futures) fn futures_opportunity_table(
    input: FuturesOpportunityTableInput,
) -> impl IntoView {
    let FuturesOpportunityTableInput {
        opportunities,
        visible,
        page,
        page_loading,
        empty_label,
        on_page,
        on_build,
        quote_ready_ids,
    } = input;
    let selected = RwSignal::new(None::<String>);
    let sparse = Memo::new(move |_| opportunities.with(|rows| rows.len() <= 4));
    let on_evidence_close = Callback::new(move |()| selected.set(None));
    Effect::new(move |_| {
        opportunities.with(|rows| {
            if selected
                .get_untracked()
                .is_some_and(|id| !rows.iter().any(|row| row.id == id))
            {
                selected.set(None);
            }
        });
    });
    view! {
        <div class="paged-table-panel" class:is-sparse=move || sparse.get()>
            <div class="table-wrap paged-table-wrap" tabindex="0" aria-label="期货套利候选表格">
                <table class="clean-table futures-table" data-table-budget="server-page" aria-label="期货套利候选">
                    <colgroup>
                        {move || visible.get().into_iter().map(|col| {
                            view! { <col class=col.width_class()/> }
                        }).collect_view()}
                    </colgroup>
                    <thead>
                        <tr>
                            {move || visible.get().into_iter().map(|col| {
                                view! { <th scope="col" class=col.cell_class()>{col.label()}</th> }
                            }).collect_view()}
                        </tr>
                    </thead>
                    <tbody>
                        <Show when=move || opportunities.with(Vec::is_empty)>
                            <tr>
                                <td colspan=move || visible.get().len().max(1).to_string() class="empty-cell">
                                    <span class="futures-empty-message">{move || empty_label.get()}</span>
                                </td>
                            </tr>
                        </Show>
                        {rows::render_body(rows::RenderBodyInput {
                                opportunities,
                                visible,
                                selected,
                                on_build,
                                on_evidence_close,
                                quote_ready_ids,
                            })}
                    </tbody>
                </table>
            </div>
            <div class="table-pager-bar">
                {server_page_controls(page, page_loading, on_page)}
            </div>
        </div>
    }
}
