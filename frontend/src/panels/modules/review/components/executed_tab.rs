use leptos::prelude::*;
use shared_types::{ExecutedTrade, ListPage, ReviewPnlField};

use crate::panels::modules::pagination::list_page_controls;

use super::executed_evidence::{evidence_summary, review_execution_environment};
use super::executed_ledger_detail::ledger_event_drilldown_summary;
use super::executed_timeline::executed_event_timeline;
use super::format::{money, proven_signed_money, record_time, signed_money, strategy_label};
use super::{section_state_row, ReviewSectionRows};

const SELECTED_TRADE_DETAIL_ID: &str = "review-selected-trade-detail";

#[path = "executed_navigation.rs"]
mod navigation;

pub(in crate::panels::modules::review) fn executed_tab(
    section: Memo<ReviewSectionRows<ExecutedTrade>>,
    page: Memo<Option<ListPage>>,
    page_loading: Memo<bool>,
    on_page: Callback<Option<String>>,
    scope: RwSignal<Option<shared_types::review::ReviewScope>>,
) -> impl IntoView {
    let selected = RwSignal::new(None::<String>);
    let select_first = RwSignal::new(false);
    Effect::new(move |_| {
        let scoped = scope.get().is_some();
        selected.set(None);
        select_first.set(scoped);
    });
    Effect::new(move |_| {
        let rows = section.get().rows;
        if select_first.get() && !rows.is_empty() {
            selected.set(Some(rows[0].id.clone()));
            select_first.set(false);
        }
    });
    let selected_row = Memo::new(move |_| {
        let selected_id = selected.get()?;
        section.with(|section| {
            section
                .rows
                .iter()
                .find(|row| row.id == selected_id)
                .cloned()
        })
    });
    let close_detail = Callback::new(move |()| selected.set(None));

    view! {
        <ExecutedSummaryStrip section=section/>
        <div
            class="review-executed-workbench"
            class:has-selection=move || selected_row.with(Option::is_some)
        >
            <div class="review-executed-primary">
                <div class="table-wrap">
                    <table class="clean-table review-table review-executed-table" data-table-budget="server-page">
                        <colgroup>
                            <col class="review-col-trade"/>
                            <col class="review-col-venues"/>
                            <col class="review-col-time"/>
                            <col class="review-col-money"/>
                            <col class="review-col-cost"/>
                            <col class="review-col-money"/>
                            <col class="review-col-money"/>
                            <col class="review-col-action"/>
                        </colgroup>
                        <thead>
                            <tr>
                                <th>"标的 / 策略"</th>
                                <th>"双腿"</th>
                                <th>"记录 / 持有"</th>
                                <th>"毛 PnL"</th>
                                <th>"手续费"</th>
                                <th>"资金费"</th>
                                <th>"净 PnL"</th>
                                <th>"数据依据"</th>
                            </tr>
                        </thead>
                        <tbody>
                            <Show when=move || section.with(|section| section.rows.is_empty())>
                                {move || section_state_row(section.get().empty_text("暂无执行记录"), "8")}
                            </Show>
                            <For each=move || section.get().rows key=|row| row.id.clone() children=move |initial| {
                                let row = Memo::new(move |_| section.with(|section| section.rows.iter()
                                    .find(|row| row.id == initial.id).cloned().unwrap_or_else(|| initial.clone())));
                                view! { <ExecutedRow row=row selected=selected/> }
                            }/>
                        </tbody>
                    </table>
                </div>
                <div class="table-pager-bar">
                    {list_page_controls(page, page_loading, on_page)}
                </div>
            </div>
            <For each=move || selected_row.get().into_iter() key=|row| row.id.clone() children=move |initial| {
                let row = Memo::new(move |_| selected_row.get().filter(|row| row.id == initial.id).unwrap_or_else(|| initial.clone()));
                executed_trade_detail(row, close_detail)
            }/>
        </div>
    }
}

#[component]
fn ExecutedRow(row: Memo<ExecutedTrade>, selected: RwSignal<Option<String>>) -> impl IntoView {
    let selection_id = row.get_untracked().id;
    let data_id = selection_id.clone();
    let selected_id = selection_id.clone();
    let is_selected =
        Memo::new(move |_| selected.with(|current| current.as_ref() == Some(&selected_id)));

    view! {
        <tr
            data-trade-id=data_id
            class:is-selected=move || is_selected.get()
            aria-selected=move || is_selected.get().to_string()
        >
            <TradeMetaCells
                row=row
            />
            <TradePnlCells row=row/>
            <td>
                <button
                    class="row-action review-evidence-action"
                    type="button"
                    aria-controls=SELECTED_TRADE_DETAIL_ID
                    aria-expanded=move || is_selected.get().to_string()
                    on:click=move |_| {
                        let is_selected = selected.with(|current| current.as_deref() == Some(selection_id.as_str()));
                        selected.set((!is_selected).then(|| selection_id.clone()));
                    }
                >
                    {move || if is_selected.get() { "收起" } else { "查看" }}
                </button>
            </td>
        </tr>
    }
}

#[component]
fn TradeMetaCells(row: Memo<ExecutedTrade>) -> impl IntoView {
    let environment = Memo::new(move |_| review_execution_environment(&row.get()));
    view! {
        <>
            <td>
                <strong>{move || row.get().symbol}</strong>
                <small class="review-trade-meta">
                    <span>{move || strategy_label(row.get().strategy)}</span>
                    <span aria-hidden="true">"·"</span>
                    <span class="review-trade-environment" data-environment=move || environment.get().tone()>
                        {move || environment.get().label()}
                    </span>
                </small>
                <small class="review-mobile-context">{move || row.with(|row| format!("{} / {}", row.long_venue, row.short_venue))}</small>
            </td>
            <td><strong>{move || format!("{} / {}", row.get().long_venue, row.get().short_venue)}</strong><small>"做多 / 做空"</small></td>
            <td>
                <strong>{move || record_time(row.get().opened_at_ms)}</strong>
                <small>{move || row.get().holding_minutes.map(|m| format!("持有 {m}m")).unwrap_or_else(|| "持有时间未知".into())}</small>
            </td>
        </>
    }
}

#[component]
fn TradePnlCells(row: Memo<ExecutedTrade>) -> impl IntoView {
    let fee = pnl_memo(row, ReviewPnlField::Fee);
    view! {
        <>
            <PnlCell display=pnl_memo(row, ReviewPnlField::Gross)/>
            <PnlCell display=fee/>
            <PnlCell display=pnl_memo(row, ReviewPnlField::Funding)/>
            <PnlCell display=pnl_memo(row, ReviewPnlField::Net) strong=true/>
        </>
    }
}

#[component]
fn PnlCell(display: Memo<PnlDisplay>, #[prop(optional)] strong: bool) -> impl IntoView {
    view! {
        <td class=move || display.get().class>
            <span class:font-bold=strong>{move || display.get().value}</span>
            <Show when=move || display.get().badge != "数据待确认">
                <small class="review-pnl-quality">{move || display.get().badge}</small>
            </Show>
        </td>
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ExecutedSummary {
    rows: usize,
    actual_count: usize,
    actual_net_usd: f64,
    estimated_count: usize,
    estimated_net_usd: f64,
    missing_count: usize,
}

fn executed_summary(rows: &[ExecutedTrade]) -> ExecutedSummary {
    summarize_trades(rows.iter())
}

fn summarize_trades<'a>(rows: impl Iterator<Item = &'a ExecutedTrade>) -> ExecutedSummary {
    rows.fold(
        ExecutedSummary::default(),
        |mut summary, row| {
            summary.rows += 1;
            if row.missing_fields.contains(&ReviewPnlField::Net)
                || (!row.actual_fields.contains(&ReviewPnlField::Net)
                    && !row.estimated_fields.contains(&ReviewPnlField::Net))
            {
                summary.missing_count += 1;
            } else if row.estimated_fields.contains(&ReviewPnlField::Net) {
                summary.estimated_count += 1;
                summary.estimated_net_usd += row.net_pnl_usd;
            } else {
                summary.actual_count += 1;
                summary.actual_net_usd += row.net_pnl_usd;
            }
            summary
        },
    )
}

#[component]
fn ExecutedSummaryStrip(section: Memo<ReviewSectionRows<ExecutedTrade>>) -> impl IntoView {
    let summary = Memo::new(move |_| section.with(|section| executed_summary(&section.rows)));
    let unknown = Memo::new(move |_| section.with(|section| section.rows.iter()
        .filter(|row| row.execution_environment().is_none()).count()));
    view! {
        <div class="review-executed-summary">
            <ReviewSummaryMetric label="本页记录" value=move || summary.get().rows.to_string() meta=|| "当前页已加载".to_owned()/>
            {[shared_types::ExecutionEnvironment::Live, shared_types::ExecutionEnvironment::Paper].into_iter().map(move |environment| {
                let totals = Memo::new(move |_| section.with(|section| summarize_trades(section.rows.iter()
                    .filter(|row| row.execution_environment() == Some(environment)))));
                view! {
                    <ReviewSummaryMetric label=format!("{}已确认净收益", super::format::environment_label(Some(environment)))
                        value=move || proven_signed_money(totals.get().actual_count > 0, totals.get().actual_net_usd)
                        meta=move || { let totals = totals.get(); format!("已确认 {} 笔 · 估算 {}（{} 笔）", totals.actual_count,
                            proven_signed_money(totals.estimated_count > 0, totals.estimated_net_usd), totals.estimated_count) }/>
                }
            }).collect_view()}
            <ReviewSummaryMetric label="净收益不可用" value=move || summary.get().missing_count.to_string()
                meta=move || format!("另有 {} 笔环境待核对，不计入实盘/模拟合计", unknown.get())/>
        </div>
    }
}

#[component]
fn ReviewSummaryMetric(
    #[prop(into)] label: String,
    value: impl Fn() -> String + Send + Sync + 'static,
    meta: impl Fn() -> String + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <div><span>{label}</span><strong>{value}</strong><small>{meta}</small></div>
    }
}

fn executed_trade_detail(row: Memo<ExecutedTrade>, on_close: Callback<()>) -> impl IntoView {
    let net = pnl_memo(row, ReviewPnlField::Net);
    let related = Memo::new(move |_| row.with(navigation::related_runs));

    view! {
        <section
            id=SELECTED_TRADE_DETAIL_ID
            class="review-selected-trade"
            aria-label="当前交易数据依据"
            tabindex="-1"
        >
            <header>
                <div><span>{move || row.with(|row| format!("{} · {}", strategy_label(row.strategy), review_execution_environment(row).label()))}</span>
                    <strong>{move || row.with(|row| format!("{} · {} / {}", row.symbol, row.long_venue, row.short_venue))}</strong></div>
                <div class="review-selected-result"><strong class=move || net.get().class>{move || net.get().value}</strong><small>{move || net.get().badge}</small></div>
                <button class="review-detail-close" type="button" on:click=move |_| on_close.run(())>"关闭"</button>
            </header>
            <div class="review-selected-pnl">
                <ReviewPnlMetric label="毛 PnL" display=pnl_memo(row, ReviewPnlField::Gross)/>
                <ReviewPnlMetric label="手续费" display=pnl_memo(row, ReviewPnlField::Fee)/>
                <ReviewPnlMetric label="资金费" display=pnl_memo(row, ReviewPnlField::Funding)/>
                <ReviewPnlMetric label="滑点归因" display=pnl_memo(row, ReviewPnlField::Slippage) note="已含成交价"/>
            </div>
            <div class="review-evidence-summary"><strong>"数据依据完整度"</strong><span>{move || evidence_summary(&row.get())}</span></div>
            <For each=move || related.get() key=|scope| (scope.run_id.clone(), scope.ticket_id.clone(), scope.opportunity_id.clone()) children=move |scope| {
                use crate::panels::workstation::ModuleId;
                let execution = scope.href(ModuleId::Execution);
                let positions = scope.href(ModuleId::Positions);
                view! {
                    <nav class="review-record-scope review-related-run" aria-label="原运行后续操作">
                        <div><strong>"原运行"</strong><span>{scope.run_id}</span></div>
                        <a class="row-action" href=execution>"查看原执行"</a>
                        <a class="row-action" href=positions>"关联持仓"</a>
                    </nav>
                }
            }/>
            {move || executed_event_timeline(&row.get())}
            <details class="review-ledger-disclosure">
                <summary><strong>"技术明细与 CloseRun 成本"</strong><span>{move || format!("{} 个事件", row.get().evidence.ledger_events.len())}</span></summary>
                <p>{move || ledger_event_drilldown_summary(&row.get())}</p>
            </details>
        </section>
    }
}

#[component]
fn ReviewPnlMetric(
    #[prop(into)] label: String,
    display: Memo<PnlDisplay>,
    #[prop(optional)] note: &'static str,
) -> impl IntoView {
    view! {
        <div><span>{label}</span><strong class=move || display.get().class>{move || display.get().value}</strong><small>{move || display.get().badge}</small>
            {(!note.is_empty()).then(|| view! { <small>{note}</small> })}
        </div>
    }
}

fn pnl_memo(row: Memo<ExecutedTrade>, field: ReviewPnlField) -> Memo<PnlDisplay> {
    Memo::new(move |_| {
        row.with(|row| {
            let value = field_value(row, field);
            let formatted = if matches!(field, ReviewPnlField::Fee | ReviewPnlField::Slippage) {
                money(value)
            } else {
                signed_money(value)
            };
            pnl_display(row, field, formatted)
        })
    })
}

fn signed_class(value: f64) -> &'static str {
    if value >= 0.0 {
        "positive"
    } else {
        "negative"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PnlDisplay {
    value: String,
    class: &'static str,
    badge: &'static str,
}

fn pnl_display(row: &ExecutedTrade, field: ReviewPnlField, value: String) -> PnlDisplay {
    if row.missing_fields.contains(&field)
        || (!row.actual_fields.contains(&field) && !row.estimated_fields.contains(&field))
    {
        return PnlDisplay {
            value: "数据待确认".into(),
            class: "muted",
            badge: "数据待确认",
        };
    }
    let class = if matches!(
        field,
        ReviewPnlField::Gross | ReviewPnlField::Funding | ReviewPnlField::Net
    ) {
        signed_class(field_value(row, field))
    } else {
        ""
    };
    PnlDisplay {
        value,
        class,
        badge: field_badge(row, field),
    }
}

fn field_value(row: &ExecutedTrade, field: ReviewPnlField) -> f64 {
    match field {
        ReviewPnlField::Gross => row.gross_pnl_usd,
        ReviewPnlField::Fee => row.fee_usd,
        ReviewPnlField::Funding => row.funding_usd,
        ReviewPnlField::Slippage => row.slippage_usd,
        ReviewPnlField::Net => row.net_pnl_usd,
    }
}

fn field_badge(row: &ExecutedTrade, field: ReviewPnlField) -> &'static str {
    if row.missing_fields.contains(&field) {
        "数据待确认"
    } else if row.estimated_fields.contains(&field) {
        "估算"
    } else if row.actual_fields.contains(&field) {
        "已确认"
    } else {
        "数据待确认"
    }
}

#[cfg(test)]
#[path = "executed_tab_tests.rs"]
mod tests;
