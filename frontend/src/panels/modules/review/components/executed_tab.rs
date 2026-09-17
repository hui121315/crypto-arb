use leptos::prelude::*;
use shared_types::{ExecutedTrade, ListPage, ReviewPnlField};

use crate::panels::modules::pagination::list_page_controls;

use super::executed_evidence::{
    evidence_summary, review_execution_environment, ReviewExecutionEnvironment,
};
use super::executed_ledger_detail::ledger_event_drilldown_summary;
use super::executed_timeline::executed_event_timeline;
use super::format::{minutes_ago, money, proven_signed_money, signed_money, strategy_label};
use super::{section_state_row, ReviewSectionRows};

const SELECTED_TRADE_DETAIL_ID: &str = "review-selected-trade-detail";

pub(in crate::panels::modules::review) fn executed_tab(
    section: Memo<ReviewSectionRows<ExecutedTrade>>,
    page: Memo<Option<ListPage>>,
    page_loading: Memo<bool>,
    on_page: Callback<Option<String>>,
) -> impl IntoView {
    let selected = RwSignal::new(None::<String>);
    let summary = Memo::new(move |_| section.with(|section| executed_summary(&section.rows)));
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
        <ExecutedSummaryStrip summary=summary/>
        <div
            class="review-executed-workbench"
            class:has-selection=move || selected.with(Option::is_some)
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
                                <th>"费用 / 滑点"</th>
                                <th>"Funding"</th>
                                <th>"净 PnL"</th>
                                <th>"证据"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {move || {
                                let section = section.get();
                                let rows = section.rows.clone();
                                if rows.is_empty() {
                                    return view! {
                                        {section_state_row(section.empty_text("暂无执行记录"), "8")}
                                    }.into_any();
                                }
                                rows.into_iter().map(|row| view! { <ExecutedRow row=row selected=selected/> }).collect_view().into_any()
                            }}
                        </tbody>
                    </table>
                </div>
                <div class="table-pager-bar">
                    {list_page_controls(page, page_loading, on_page)}
                </div>
            </div>
            {move || selected_row.get().map(|row| executed_trade_detail(&row, close_detail))}
        </div>
    }
}

#[component]
fn ExecutedRow(row: ExecutedTrade, selected: RwSignal<Option<String>>) -> impl IntoView {
    let strategy = strategy_label(row.strategy);
    let symbol = row.symbol.clone();
    let long_venue = row.long_venue.clone();
    let short_venue = row.short_venue.clone();
    let opened_at_ms = row.opened_at_ms;
    let holding_minutes = row.holding_minutes;
    let selection_id = row.id.clone();
    let row_state_id = row.id.clone();
    let row_aria_id = row.id.clone();
    let expanded_id = row.id.clone();
    let label_id = row.id.clone();
    let environment = review_execution_environment(&row);

    view! {
        <tr
            class:is-selected=move || selected.with(|current| current.as_deref() == Some(row_state_id.as_str()))
            aria-selected=move || selected.with(|current| current.as_deref() == Some(row_aria_id.as_str())).to_string()
        >
            <TradeMetaCells
                strategy=strategy
                symbol=symbol
                long_venue=long_venue
                short_venue=short_venue
                opened_at_ms=opened_at_ms
                holding_minutes=holding_minutes
                environment=environment
            />
            <TradePnlCells row=row/>
            <td>
                <button
                    class="row-action review-evidence-action"
                    type="button"
                    aria-controls=SELECTED_TRADE_DETAIL_ID
                    aria-expanded=move || selected.with(|current| current.as_deref() == Some(expanded_id.as_str())).to_string()
                    on:click=move |_| {
                        let is_selected = selected.with(|current| current.as_deref() == Some(selection_id.as_str()));
                        selected.set((!is_selected).then(|| selection_id.clone()));
                    }
                >
                    {move || if selected.with(|current| current.as_deref() == Some(label_id.as_str())) { "收起" } else { "查看" }}
                </button>
            </td>
        </tr>
    }
}

#[component]
fn TradeMetaCells(
    #[prop(into)] strategy: String,
    symbol: String,
    long_venue: String,
    short_venue: String,
    opened_at_ms: i64,
    holding_minutes: Option<u32>,
    environment: ReviewExecutionEnvironment,
) -> impl IntoView {
    view! {
        <>
            <td>
                <strong>{symbol}</strong>
                <small class="review-trade-meta">
                    <span>{strategy}</span>
                    <span aria-hidden="true">"·"</span>
                    <span class="review-trade-environment" data-environment=environment.tone()>
                        {environment.label()}
                    </span>
                </small>
            </td>
            <td><strong>{long_venue} " / " {short_venue}</strong><small>"做多 / 做空"</small></td>
            <td>
                <strong>{minutes_ago(opened_at_ms)}</strong>
                <small>{holding_minutes.map(|m| format!("持有 {m}m")).unwrap_or_else(|| "持有时间未知".into())}</small>
            </td>
        </>
    }
}

#[component]
fn TradePnlCells(row: ExecutedTrade) -> impl IntoView {
    let gross = pnl_display(&row, ReviewPnlField::Gross, signed_money(row.gross_pnl_usd));
    let fee = pnl_display(&row, ReviewPnlField::Fee, money(row.fee_usd));
    let funding = pnl_display(&row, ReviewPnlField::Funding, signed_money(row.funding_usd));
    let slippage = pnl_display(&row, ReviewPnlField::Slippage, money(row.slippage_usd));
    let net = pnl_display(&row, ReviewPnlField::Net, signed_money(row.net_pnl_usd));

    view! {
        <>
            <PnlCell
                value=gross.value
                class=gross.class
                badge=gross.badge
            />
            <PnlCell
                value=format!("{} + {}", fee.value, slippage.value)
                class=""
                badge=format!("费用 {} · 滑点 {}", fee.badge, slippage.badge)
            />
            <PnlCell
                value=funding.value
                class=funding.class
                badge=funding.badge
            />
            <PnlCell
                value=net.value
                class=net.class
                badge=net.badge
                strong=true
            />
        </>
    }
}

#[component]
fn PnlCell(
    value: String,
    #[prop(into)] class: String,
    #[prop(into)] badge: String,
    #[prop(optional)] strong: bool,
) -> impl IntoView {
    let badge_view = (badge != "缺证据").then(|| {
        view! { <small class="review-pnl-quality">{badge}</small> }
    });
    view! {
        <td class=class>
            {move || {
                if strong {
                    view! { <strong>{value.clone()}</strong> }.into_any()
                } else {
                    view! { <span>{value.clone()}</span> }.into_any()
                }
            }}
            {badge_view}
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
    rows.iter().fold(
        ExecutedSummary {
            rows: rows.len(),
            ..ExecutedSummary::default()
        },
        |mut summary, row| {
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
fn ExecutedSummaryStrip(summary: Memo<ExecutedSummary>) -> impl IntoView {
    view! {
        <div class="review-executed-summary">
            <ReviewSummaryMetric label="本页记录" value=move || summary.get().rows.to_string() meta=|| "当前页已加载".to_owned()/>
            <ReviewSummaryMetric label="已确认净收益" value=move || proven_signed_money(summary.get().actual_count > 0, summary.get().actual_net_usd) meta=move || format!("{} 笔终态可核验", summary.get().actual_count)/>
            <ReviewSummaryMetric label="估算净收益" value=move || proven_signed_money(summary.get().estimated_count > 0, summary.get().estimated_net_usd) meta=move || format!("{} 笔待补终态", summary.get().estimated_count)/>
            <ReviewSummaryMetric label="净收益不可用" value=move || summary.get().missing_count.to_string() meta=|| "未计入合计".to_owned()/>
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

fn executed_trade_detail(row: &ExecutedTrade, on_close: Callback<()>) -> impl IntoView {
    let environment = review_execution_environment(row);
    let net = pnl_display(row, ReviewPnlField::Net, signed_money(row.net_pnl_usd));
    let evidence = evidence_summary(row);
    let drilldown = ledger_event_drilldown_summary(row);
    let event_count = row.evidence.ledger_events.len();
    let title = format!("{} · {} / {}", row.symbol, row.long_venue, row.short_venue);
    let context = format!("{} · {}", strategy_label(row.strategy), environment.label());
    let timeline = executed_event_timeline(row);

    view! {
        <section
            id=SELECTED_TRADE_DETAIL_ID
            class="review-selected-trade"
            aria-label="当前交易证据"
            tabindex="-1"
        >
            <header>
                <div><span>{context}</span><strong>{title}</strong></div>
                <div class="review-selected-result"><strong class=net.class>{net.value}</strong><small>{net.badge}</small></div>
                <button class="review-detail-close" type="button" on:click=move |_| on_close.run(())>"关闭"</button>
            </header>
            <div class="review-selected-pnl">
                <ReviewPnlMetric label="毛 PnL" display=pnl_display(row, ReviewPnlField::Gross, signed_money(row.gross_pnl_usd))/>
                <ReviewPnlMetric label="费用" display=pnl_display(row, ReviewPnlField::Fee, money(row.fee_usd))/>
                <ReviewPnlMetric label="Funding" display=pnl_display(row, ReviewPnlField::Funding, signed_money(row.funding_usd))/>
                <ReviewPnlMetric label="滑点" display=pnl_display(row, ReviewPnlField::Slippage, money(row.slippage_usd))/>
            </div>
            <div class="review-evidence-summary"><strong>"证据完整度"</strong><span>{evidence}</span></div>
            {timeline}
            <details class="review-ledger-disclosure">
                <summary><strong>"技术明细与 CloseRun 成本"</strong><span>{format!("{event_count} 个事件")}</span></summary>
                <p>{drilldown}</p>
            </details>
        </section>
    }
}

#[component]
fn ReviewPnlMetric(#[prop(into)] label: String, display: PnlDisplay) -> impl IntoView {
    view! {
        <div><span>{label}</span><strong class=display.class>{display.value}</strong><small>{display.badge}</small></div>
    }
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
            value: "缺证据".into(),
            class: "muted",
            badge: "缺证据",
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
        "缺证据"
    } else if row.estimated_fields.contains(&field) {
        "估算"
    } else if row.actual_fields.contains(&field) {
        "已确认"
    } else {
        "缺证据"
    }
}

#[cfg(test)]
#[path = "executed_tab_tests.rs"]
mod tests;
