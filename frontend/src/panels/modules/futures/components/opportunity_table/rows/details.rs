use leptos::prelude::*;

use crate::panels::modules::funding_stats::funding_cycle_trend;
use crate::panels::modules::futures::columns::{expandable_for_strategy, ColumnId};
use crate::panels::modules::futures::data::FuturesOpportunityRow;

use super::super::format::{detail_value, execution_title, source_age_text};

pub(super) fn evidence_panel(
    opp: &FuturesOpportunityRow,
    on_close: Callback<()>,
    panel_id: String,
) -> impl IntoView {
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
    let (decision_label, decision_class) = if opp.execution_eligible {
        ("可预检", "is-ready")
    } else {
        ("仅观察", "is-observation")
    };
    let decision_detail = execution_title(opp);
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
    let detail_columns = expandable_for_strategy(opp.strategy_kind);
    let detail_count = detail_columns.len();
    let funding_stats = opp.funding_stats.clone();
    let profit_detail = format!(
        "毛 {} · 成本 {} · 费后 {}",
        opp.gross_one_cycle_text(),
        opp.round_trip_cost_text(),
        opp.one_cycle_net_text()
    );
    let cost_evidence = opp.cost_evidence_label();
    view! {
        <section id=panel_id class="futures-evidence-panel" aria-label="当前候选证据">
            <header>
                <div class="futures-evidence-identity">
                    <span>"当前候选证据"</span>
                    <strong>{format!("{} · {}", opp.pair, opp.strategy_label)}</strong>
                </div>
                <div class=format!("futures-evidence-decision {decision_class}")>
                    <span>{decision_label}</span>
                    <strong>{decision_detail}</strong>
                </div>
                <button type="button" on:click=move |_| on_close.run(())>"收起"</button>
            </header>
            <div class="futures-detail-grid is-primary">
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
            </div>
            <details class="futures-evidence-more">
                <summary>
                    <span>"完整策略证据"</span>
                    <strong>{format!("{detail_count} 项")}</strong>
                </summary>
                <div class="futures-detail-grid is-advanced">
                        {detail_columns.iter().copied().map(|col| {
                            view! { <DetailItem label=col.label() value=detail_value(opp, col)/> }
                        }).collect_view()}
                </div>
            </details>
        </section>
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
