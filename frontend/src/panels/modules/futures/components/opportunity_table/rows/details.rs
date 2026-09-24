use leptos::prelude::*;

use crate::panels::modules::funding_stats::funding_cycle_trend;
use crate::panels::modules::futures::columns::{expandable_for_strategy, ColumnId};
use crate::panels::modules::futures::data::FuturesOpportunityRow;

use super::super::format::{detail_value, execution_title, source_age_text};

pub(super) fn evidence_panel(
    opp: Memo<FuturesOpportunityRow>,
    on_close: Callback<()>,
    panel_id: String,
) -> impl IntoView {
    view! {
        <section id=panel_id class="futures-evidence-panel" aria-label="当前候选证据">
            <header>
                <div class="futures-evidence-identity">
                    <span>"当前候选证据"</span>
                    <strong>{move || opp.with(|row| format!("{} · {}", row.pair, row.strategy_label))}</strong>
                </div>
                <div class=move || format!("futures-evidence-decision {}", if opp.get().execution_eligible { "is-ready" } else { "is-observation" })>
                    <span>{move || if opp.get().execution_eligible { "可预检" } else { "仅观察" }}</span>
                    <strong>{move || execution_title(&opp.get())}</strong>
                </div>
                <button type="button" on:click=move |_| on_close.run(())>"收起"</button>
            </header>
            <div class="futures-detail-grid is-primary">
                {move || primary_details(&opp.get())}
            </div>
            <details class="futures-evidence-more">
                <summary>
                    <span>"完整策略证据"</span>
                    <strong>{move || format!("{} 项", expandable_for_strategy(opp.get().strategy_kind).len())}</strong>
                </summary>
                <div class="futures-detail-grid is-advanced">
                    {move || {
                        let row = opp.get();
                        expandable_for_strategy(row.strategy_kind).iter().copied().map(|col| {
                            view! { <DetailItem label=col.label() value=detail_value(&row, col)/> }
                        }).collect_view()
                    }}
                </div>
            </details>
        </section>
    }
}

fn primary_details(opp: &FuturesOpportunityRow) -> impl IntoView {
    let show_funding = matches!(
        opp.strategy_kind,
        Some(
            shared_types::StrategyKind::PerpCross
                | shared_types::StrategyKind::SpotPerp
                | shared_types::StrategyKind::CrossSpotPerp
        )
    );
    let execution_detail = if opp.execution_eligible {
        "可构建新的完整双腿；构建时重新核验双腿盘口与可执行数量".to_owned()
    } else if opp.execution_blockers.is_empty() {
        "后端判定当前不可执行".to_owned()
    } else {
        opp.execution_blockers.join("；")
    };
    let strategy_detail = detail_value(opp, ColumnId::StrategyKind);
    let source_detail = source_age_text(opp);
    let long_market_detail = opp
        .long_market_evidence
        .clone()
        .unwrap_or_else(|| "做多腿行情证据缺失".to_owned());
    let short_market_detail = opp
        .short_market_evidence
        .clone()
        .unwrap_or_else(|| "做空腿行情证据缺失".to_owned());
    let funding_stats = opp.funding_stats.clone();
    let profit_detail = format!(
        "毛 {} · 成本 {} · 费后 {}",
        opp.gross_one_cycle_text(),
        opp.round_trip_cost_text(),
        opp.one_cycle_net_text()
    );
    let cost_evidence = opp.cost_evidence_label();
    view! {
                        {show_funding.then(|| view! {
                            <div class="detail-item funding-detail-item">
                                <span>"历史费率差"</span>
                                {funding_cycle_trend(funding_stats.clone())}
                            </div>
                        })}
                        <DetailItem label="策略" value=strategy_detail/>
                        <DetailItem label="执行条件" value=execution_detail/>
                        <DetailItem label="收益拆分" value=profit_detail/>
                        <DetailItem label="成本证据" value=cost_evidence/>
                        <DetailItem label="行情快照" value=source_detail/>
                        <DetailItem label="做多腿行情" value=long_market_detail/>
                        <DetailItem label="做空腿行情" value=short_market_detail/>
    }
}

#[component]
fn DetailItem(#[prop(into)] label: String, #[prop(into)] value: String) -> impl IntoView {
    view! {
        <div class="detail-item">
            <span>{label}</span>
            <strong>{value}</strong>
        </div>
    }
}
