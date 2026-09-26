//! 持仓表组件装配：搜索/分页运行态接线、表头与行渲染。
//! 表格/单元格派生见 `derive.rs`，字段质量 chip 见 `quality.rs`，测试见 `testing.rs`。

#[path = "positions_table/derive.rs"]
mod derive;
#[path = "positions_table/quality.rs"]
mod quality;
#[path = "positions_table/row.rs"]
mod row;
#[cfg(test)]
#[path = "positions_table/testing.rs"]
mod testing;

use leptos::prelude::*;
use shared_types::{AccountDataHealth, AccountFieldQuality, PositionRow};
use std::sync::Arc;

use super::super::data::{close_selection_requires_live, has_execution_projection, CloseExecutionGate, PortfolioAccountAccess, PositionCloseAction};
use super::super::data::{has_pair_evidence, pair_close_key, pair_label, position_key};
use super::account_evidence::{render_account_surface_evidence, AccountSurfaceEvidence};
use super::account_setup::account_data_placeholder;
use super::format::money;
use super::pair_protection::pair_liquidation_risk;
use super::section_state::SectionData;
use crate::panels::modules::pagination::{page_controls, use_table_runtime};
use crate::panels::routing::RunRouteContext;
use crate::state::module_runtime::{store_choice, stored_choice};

use derive::{
    filtered_sorted_rows, format_qty, funding_display, pnl_display, positions_dataset_key,
    positions_interaction_key, price, row_is_closing, severity_class, side_class, side_label,
    stable_render_rows, table_empty_text, table_page, table_status_label, value_or_missing,
    TablePage,
};
use quality::{
    position_quality_by_field, position_quality_for_row, position_row_evidence_panel,
    position_row_evidence_toggle, position_row_health_for_row,
};
use row::{render_body, RowBodyContext};

const PAGE_SIZE: usize = 50;
const POSITIONS_QUERY_STORAGE_KEY: &str = "crossline.positions.query";
const POSITIONS_PAGE_STORAGE_KEY: &str = "crossline.positions.page";

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PositionTableEvidence {
    field_quality: Memo<Vec<AccountFieldQuality>>,
    row_health: Memo<Vec<AccountDataHealth>>,
    account: Memo<Option<AccountSurfaceEvidence>>,
}

impl PositionTableEvidence {
    pub(in crate::panels::modules::positions) fn new(
        field_quality: Memo<Vec<AccountFieldQuality>>,
        row_health: Memo<Vec<AccountDataHealth>>,
        account: Memo<Option<AccountSurfaceEvidence>>,
    ) -> Self {
        Self {
            field_quality,
            row_health,
            account,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PositionTableRuntime {
    on_close: Callback<PositionRow>,
    on_close_pair: Callback<PositionRow>,
    closing_key: RwSignal<Option<String>>,
    account_access: Memo<PortfolioAccountAccess>,
    ledger_flow_active: Memo<bool>,
    close_execution_gate: Memo<CloseExecutionGate>,
}

impl PositionTableRuntime {
    pub(in crate::panels::modules::positions) fn new(
        action: PositionCloseAction,
        account_access: Memo<PortfolioAccountAccess>,
        ledger_flow_active: Memo<bool>,
        close_execution_gate: Memo<CloseExecutionGate>,
    ) -> Self {
        Self {
            on_close: action.close_one,
            on_close_pair: action.close_pair,
            closing_key: action.active_key,
            account_access,
            ledger_flow_active,
            close_execution_gate,
        }
    }
}

pub(in crate::panels::modules::positions) fn positions_table(
    rows: Memo<SectionData<Vec<PositionRow>>>,
    run_scope: RwSignal<Option<RunRouteContext>>,
    evidence: PositionTableEvidence,
    surface: PositionTableRuntime,
) -> impl IntoView {
    let query = RwSignal::new(stored_positions_query());
    let expanded_evidence_key = RwSignal::new(None::<String>);
    let close_confirmation_key = RwSignal::new(None::<String>);
    // Scope only the table. Risk totals and close validation still use the full account snapshot.
    let scoped_rows = Memo::new(move |_| {
        let mut section = rows.get();
        if let Some(scope) = run_scope.get() {
            section.value.retain(|row| scope.matches_position(row));
        }
        section
    });
    let effective_query = Memo::new(move |_| {
        if run_scope.get().is_some() {
            String::new()
        } else {
            query.get()
        }
    });
    let table_rows =
        Memo::new(move |_| filtered_sorted_rows(scoped_rows.get(), &effective_query.get()));
    let execution_projection = Memo::new(move |_| has_execution_projection(&rows.get().value));
    let dataset_key = Memo::new(move |_| {
        format!(
            "{:?}:{}",
            run_scope.get(),
            positions_dataset_key(&scoped_rows.get(), &effective_query.get())
        )
    });
    let interaction_key = Memo::new(move |_| {
        format!(
            "{:?}:{}",
            run_scope.get(),
            positions_interaction_key(&scoped_rows.get(), &effective_query.get())
        )
    });
    let table = use_table_runtime(
        POSITIONS_PAGE_STORAGE_KEY,
        dataset_key,
        table_rows,
        PAGE_SIZE,
    );
    let table_wrap = NodeRef::<leptos::html::Div>::new();
    let total_rows = table.total;
    let current_page = table.current_page;
    let runtime = table.runtime;
    let page = Memo::new(move |_| table_page(scoped_rows.get(), runtime.get()));
    let render_rows =
        Memo::new(move |previous| page.with(|page| stable_render_rows(previous, &page.rows)));
    let all_rows =
        Memo::new(move |_| rows.with(|section| Arc::<[PositionRow]>::from(section.value.clone())));
    // Market repricing leaves confirmation intact; position identity and source changes do not.
    let close_context = Memo::new(move |_| all_rows.with(|rows| rows.iter().map(|row| (
        position_key(row), row.origin, row.quantity.to_bits(),
        row.pair_evidence.as_ref().map(|pair| (
            pair.run_id.clone(), pair.partner_venue.clone(), pair.partner_symbol.clone(), pair.partner_side,
        )),
    )).collect::<Vec<_>>()));

    Effect::new(move |_| {
        store_choice(POSITIONS_QUERY_STORAGE_KEY, &query.get());
    });
    Effect::new(move |_| {
        interaction_key.get();
        expanded_evidence_key.set(None);
        close_confirmation_key.set(None);
        if let Some(element) = table_wrap.get() {
            element.set_scroll_left(0);
        }
    });

    Effect::new(move |_| {
        surface.close_execution_gate.track();
        close_context.track();
        close_confirmation_key.set(None);
    });

    // 提成 Memo<bool>：每 2s 快照刷新时行数据必然变化，但该布尔值几乎从不翻转。
    // 若直接在渲染闭包里读 rows.get()，整个表格子树（含搜索输入框）会随每次刷新
    // 重建，正在输入的搜索框会丢焦点。
    let requires_account_setup = Memo::new(move |_| {
        table_requires_account_setup(
            &surface.account_access.get(),
            &rows.get(),
            surface.ledger_flow_active.get(),
        )
    });
    view! {
        <div
            class="paged-table-panel"
            class:is-sparse=move || total_rows.get() <= 3
        >
            <Show when=move || run_scope.get().is_some()>
                <div class="positions-run-scope" role="status">
                    <div><strong>"关联运行持仓"</strong>
                        <span>{move || run_scope.get().map(|scope| scope.run_id)}</span>
                        <small>{move || if scoped_rows.get().value.is_empty() {
                            "尚未找到明确关联的持仓；不代表没有持仓或已平仓。风险摘要仍为全账户。"
                        } else { "仅显示本次运行关联持仓；风险摘要仍为全账户。" }}</small>
                    </div>
                    <a class="row-action" href="#positions" on:click=move |_| {
                        query.set(String::new());
                        run_scope.set(None);
                    }>"查看全部持仓"</a>
                </div>
            </Show>
            {move || if requires_account_setup.get() {
                account_data_placeholder(
                    "持仓等待账户接入",
                    "配置账户读取权限后显示仓位、强平距离、资金费 与配对关系。",
                )
            } else {
                view! {
                    <div class="positions-table-tools">
                        <input
                            type="search"
                            placeholder="搜索场所、标的、配对"
                            disabled=move || run_scope.get().is_some()
                            title=move || if run_scope.get().is_some() { "当前按运行记录筛选" } else { "搜索场所、标的、配对" }
                            prop:value=move || effective_query.get()
                            on:input=move |ev| query.set(event_target_value(&ev))
                        />
                        <span>{move || table_status_label(rows.get(), total_rows.get())}</span>
                    </div>
                    {move || if execution_projection.get()
                        || (surface.ledger_flow_active.get()
                            && surface.account_access.get().account_data_unavailable())
                    {
                        view! {
                            <div class="positions-table-source-note">
                                <strong>"执行账本模式"</strong>
                                <span>"模拟持仓可直接配对平仓；交易所私有账户读数仍待配置。"</span>
                            </div>
                        }.into_any()
                    } else {
                        render_account_surface_evidence(evidence.account.get())
                    }}
                    <div
                        node_ref=table_wrap
                        class="table-wrap positions-table-wrap"
                        hidden=move || run_scope.get().is_some() && scoped_rows.get().value.is_empty()
                        tabindex="0"
                        aria-label="持仓表，可横向滚动；操作列固定在右侧"
                    >
                        <table
                            class="clean-table positions-table"
                            class:is-empty=move || total_rows.get() == 0
                        >
                            <caption class="sr-only">
                                "当前持仓、价格、盈亏、强平、资金费、配对、独立双腿机会与平仓操作"
                            </caption>
                            <colgroup>
                                <col class="positions-col-position" />
                                <col class="positions-col-size" />
                                <col class="positions-col-price" />
                                <col class="positions-col-pnl" />
                                <col class="positions-col-liquidation" />
                                <col class="positions-col-funding" />
                                <col class="positions-col-pair" />
                                <col class="positions-col-action" />
                            </colgroup>
                            <thead>
                                <tr>
                                    <th scope="col">"仓位"</th>
                                    <th scope="col" class="num">"数量 / 杠杆"</th>
                                    <th scope="col" class="num">"入场 / 标记"</th>
                                    <th scope="col" class="num">"PnL / 保证金"</th>
                                    <th scope="col" class="num">"强平"</th>
                                    <th scope="col" class="num">"资金费"</th>
                                    <th scope="col">"配对 / 对冲"</th>
                                    <th scope="col" class="positions-action-column">"操作"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {render_body(
                                    page,
                                    render_rows,
                                    RowBodyContext {
                                        all_rows,
                                        field_quality: evidence.field_quality,
                                        row_health: evidence.row_health,
                                        on_close: surface.on_close,
                                        on_close_pair: surface.on_close_pair,
                                        closing_key: surface.closing_key,
                                        close_confirmation_key,
                                        expanded_evidence_key,
                                        close_execution_gate: surface.close_execution_gate,
                                    },
                                )}
                            </tbody>
                        </table>
                    </div>
                    {move || (total_rows.get() > PAGE_SIZE).then(|| view! {
                        <div class="table-pager-bar">
                            {page_controls(total_rows, current_page, PAGE_SIZE)}
                        </div>
                    })}
                }.into_any()
            }}
        </div>
    }
}

fn table_requires_account_setup(
    access: &PortfolioAccountAccess,
    rows: &SectionData<Vec<PositionRow>>,
    ledger_flow_active: bool,
) -> bool {
    access.account_data_unavailable() && access.coverage_incomplete()
        && rows.has_fresh_value() && rows.value.is_empty() && !ledger_flow_active
}

fn stored_positions_query() -> String {
    stored_choice(POSITIONS_QUERY_STORAGE_KEY, |value| Some(value.to_owned())).unwrap_or_default()
}
