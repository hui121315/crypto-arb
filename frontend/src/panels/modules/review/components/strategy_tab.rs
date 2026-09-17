use leptos::prelude::*;
use shared_types::{StrategyPerformance, StrategyPerformanceSampleStatus};

use crate::panels::modules::pagination::{page_controls, use_table_runtime};

use super::format::{
    fill_confidence_label, money, pct, proven_signed_class, proven_signed_money, signed_class,
    signed_money, strategy_label,
};
use super::ReviewSectionRows;

const PAGE_SIZE: usize = 50;
const STRATEGY_PAGE_STORAGE_KEY: &str = "crossline.review.strategy.page";
const STRATEGY_DETAIL_ID: &str = "review-strategy-detail";

pub(in crate::panels::modules::review) fn strategy_tab(
    section: Memo<ReviewSectionRows<StrategyPerformance>>,
) -> impl IntoView {
    let rows = Memo::new(move |_| section.with(|section| section.rows.clone()));
    let summary = Memo::new(move |_| strategy_summary(&rows.get()));
    let dataset_key = Memo::new(move |_| section.with(strategy_dataset_key));
    let table = use_table_runtime(STRATEGY_PAGE_STORAGE_KEY, dataset_key, rows, PAGE_SIZE);
    let selected = RwSignal::new(None::<shared_types::StrategyKind>);
    let selected_row = Memo::new(move |_| {
        let selected_kind = selected.get()?;
        rows.get().into_iter().find(|row| row.kind == selected_kind)
    });
    let close_detail = Callback::new(move |()| selected.set(None));

    view! {
        <StrategySummaryStrip summary=summary/>
        {move || {
            let section = section.get();
            let page_rows = table.runtime.get().rows;
            if page_rows.is_empty() {
                let loaded = section.has_loaded_context();
                return view! {
                    <div class=if loaded { "review-business-empty" } else { "review-business-empty is-error" }>
                        <strong>{section.empty_text("30D 内暂无完整策略样本")}</strong>
                        <span>{if loaded { "策略绩效只统计具备执行与收益证据的交易。" } else { "当前没有可用快照，错误证据保留在上方数据状态中。" }}</span>
                    </div>
                }.into_any();
            }
            view! {
                <div
                    class="review-strategy-workbench"
                    class:has-selection=move || selected_row.with(Option::is_some)
                >
                    <div class="review-strategy-primary">
                        <div class="table-wrap">
                            <table class="clean-table review-table review-strategy-table" data-table-budget="server-page">
                                <colgroup>
                                    <col class="review-col-strategy"/>
                                    <col class="review-col-sample"/>
                                    <col class="review-col-outcomes"/>
                                    <col class="review-col-net"/>
                                    <col class="review-col-expectancy"/>
                                    <col class="review-col-factor"/>
                                    <col class="review-col-drawdown"/>
                                    <col class="review-col-action"/>
                                </colgroup>
                                <thead>
                                    <tr>
                                        <th>"策略"</th>
                                        <th>"可计算样本"</th>
                                        <th>"已确认盈 / 亏"</th>
                                        <th>"已确认净 PnL"</th>
                                        <th>"已确认单笔"</th>
                                        <th>"Profit Factor"</th>
                                        <th>"最大回撤"</th>
                                        <th>"证据"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {page_rows.into_iter().map(|row| view! { <StrategyRow row=row selected=selected/> }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                        {move || (table.total.get() > PAGE_SIZE).then(|| view! {
                            {page_controls(table.total, table.current_page, PAGE_SIZE)}
                        })}
                    </div>
                    {move || selected_row.get().map(|row| strategy_detail(&row, close_detail))}
                </div>
            }.into_any()
        }}
    }
}

fn strategy_dataset_key(section: &ReviewSectionRows<StrategyPerformance>) -> String {
    let mut parts = Vec::with_capacity(4);
    parts.push(section.rows.len().to_string());
    for row in section.rows.iter().take(2) {
        parts.push(format!("{:?}:{}", row.kind, row.trades_30d));
    }
    if let Some(row) = section.rows.last() {
        parts.push(format!("{:?}:{}", row.kind, row.trades_30d));
    }
    parts.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionFillConfidence, StrategyKind};

    #[test]
    fn strategy_page_storage_key_is_namespaced() {
        assert!(STRATEGY_PAGE_STORAGE_KEY.starts_with("crossline.review."));
        assert!(STRATEGY_PAGE_STORAGE_KEY.ends_with(".page"));
    }

    #[test]
    fn strategy_dataset_key_changes_when_rows_change() {
        let first = ReviewSectionRows::ready(vec![strategy_perf(StrategyKind::PerpCross, 3)]);
        let second = ReviewSectionRows::ready(vec![strategy_perf(StrategyKind::PerpCross, 4)]);

        assert_ne!(strategy_dataset_key(&first), strategy_dataset_key(&second));
    }

    #[test]
    fn strategy_sample_label_explains_window_and_confidence() {
        let mut row = strategy_perf(StrategyKind::PerpCross, 2);
        row.total_trades_30d = 3;
        row.skipped_trades_30d = 1;
        row.sample_status = StrategyPerformanceSampleStatus::PartialEvidence;
        row.actual_trades_30d = 1;
        row.estimated_trades_30d = 1;
        row.lowest_fill_confidence = Some(ExecutionFillConfidence::AdapterAck);

        let label = sample_label(&row);

        assert!(label.contains("30d"));
        assert!(label.contains("证据不全"));
        assert!(label.contains("可计算 2/3"));
        assert!(label.contains("已确认 1"));
        assert!(label.contains("估算 1"));
        assert!(label.contains("闭环 0"));
        assert!(label.contains("跳过 1"));
        assert!(label.contains("仅 ACK 推定"));
    }

    #[test]
    fn strategy_summary_counts_only_loaded_complete_trade_samples() {
        let profitable = strategy_perf(StrategyKind::PerpCross, 3);
        let mut partial = strategy_perf(StrategyKind::SpotPerp, 2);
        partial.sample_status = StrategyPerformanceSampleStatus::PartialEvidence;
        partial.actual_trades_30d = 0;
        partial.estimated_trades_30d = 2;
        partial.actual_net_pnl_30d_usd = 0.0;
        partial.estimated_net_pnl_30d_usd = -4.0;
        partial.net_pnl_30d_usd = 0.0;

        let summary = strategy_summary(&[profitable, partial]);

        assert_eq!(summary.strategies, 2);
        assert_eq!(summary.computable_trades, 5);
        assert_eq!(summary.actual_trades, 3);
        assert_eq!(summary.estimated_trades, 2);
        assert_eq!(summary.partial_strategies, 1);
        assert_eq!(summary.actual_net_pnl_usd, 0.0);
        assert_eq!(summary.estimated_net_pnl_usd, -4.0);
    }

    fn strategy_perf(kind: StrategyKind, trades_30d: u32) -> StrategyPerformance {
        StrategyPerformance {
            kind,
            sample_window_days: 30,
            total_trades_30d: trades_30d,
            trades_30d,
            actual_trades_30d: trades_30d,
            estimated_trades_30d: 0,
            skipped_trades_30d: 0,
            partial_evidence_trades_30d: 0,
            sample_status: StrategyPerformanceSampleStatus::Complete,
            lowest_fill_confidence: None,
            lowest_fill_confidence_score: None,
            profitable_trades_30d: 0,
            losing_trades_30d: 0,
            break_even_trades_30d: 0,
            independent_periods_30d: 0,
            data_missing_rate_pct: 0.0,
            hit_rate_pct: 0.0,
            avg_pnl_per_trade_usd: 0.0,
            sharpe_30d: 0.0,
            sortino_30d: 0.0,
            max_drawdown_pct: 0.0,
            max_drawdown_usd: 0.0,
            gross_pnl_30d_usd: 0.0,
            gross_profit_30d_usd: 0.0,
            gross_loss_30d_usd: 0.0,
            profit_factor: None,
            tail_loss_p95_usd: None,
            worst_trade_pnl_usd: None,
            finality_latency_p50_ms: None,
            finality_latency_p95_ms: None,
            finality_latency_max_ms: None,
            trade_order_error_rate_pct: None,
            actual_net_pnl_30d_usd: 0.0,
            estimated_net_pnl_30d_usd: 0.0,
            net_pnl_30d_usd: 0.0,
            avg_holding_hours: 0.0,
        }
    }
}

#[component]
fn StrategyRow(
    row: StrategyPerformance,
    selected: RwSignal<Option<shared_types::StrategyKind>>,
) -> impl IntoView {
    let kind = row.kind;
    let label_kind = row.kind;
    let net_class = signed_class(row.net_pnl_30d_usd);
    let avg_class = signed_class(row.avg_pnl_per_trade_usd);
    let confidence = row
        .lowest_fill_confidence
        .map(fill_confidence_label)
        .unwrap_or("缺成交置信度");
    let net_breakdown = net_breakdown_label(&row);
    view! {
        <tr
            class:is-selected=move || selected.with(|current| current == &Some(kind))
            aria-selected=move || selected.with(|current| current == &Some(kind)).to_string()
        >
            <td><strong>{strategy_label(row.kind)}</strong><small>{format!("{}D", row.sample_window_days)}</small></td>
            <td><strong>{format!("{} / {}", row.trades_30d, row.total_trades_30d)}</strong><small>{sample_status_label(row.sample_status)} " · " {confidence}</small></td>
            <td><strong>{format!("{} / {}", row.profitable_trades_30d, row.losing_trades_30d)}</strong><small>{format!("{} 笔持平", row.break_even_trades_30d)}</small></td>
            <td class=net_class><strong>{signed_money(row.net_pnl_30d_usd)}</strong><small>{net_breakdown}</small></td>
            <td class=avg_class>{signed_money(row.avg_pnl_per_trade_usd)}</td>
            <td>{ratio(row.profit_factor)}</td>
            <td class="negative">{money(row.max_drawdown_usd)}</td>
            <td>
                <button
                    class="row-action review-evidence-action"
                    type="button"
                    aria-controls=STRATEGY_DETAIL_ID
                    aria-expanded=move || selected.with(|current| current == &Some(kind)).to_string()
                    on:click=move |_| {
                        let is_selected = selected.with(|current| current == &Some(kind));
                        selected.set((!is_selected).then_some(kind));
                    }
                >
                    {move || if selected.with(|current| current == &Some(label_kind)) { "收起" } else { "查看" }}
                </button>
            </td>
        </tr>
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct StrategySummary {
    strategies: usize,
    computable_trades: u32,
    actual_trades: u32,
    estimated_trades: u32,
    unknown_provenance_trades: u32,
    partial_strategies: usize,
    actual_net_pnl_usd: f64,
    estimated_net_pnl_usd: f64,
}

fn strategy_summary(rows: &[StrategyPerformance]) -> StrategySummary {
    let computable_trades = rows.iter().map(|row| row.trades_30d).sum::<u32>();
    let actual_trades = rows.iter().map(|row| row.actual_trades_30d).sum::<u32>();
    let estimated_trades = rows.iter().map(|row| row.estimated_trades_30d).sum::<u32>();
    StrategySummary {
        strategies: rows.len(),
        computable_trades,
        actual_trades,
        estimated_trades,
        unknown_provenance_trades: computable_trades
            .saturating_sub(actual_trades)
            .saturating_sub(estimated_trades),
        partial_strategies: rows
            .iter()
            .filter(|row| row.sample_status != StrategyPerformanceSampleStatus::Complete)
            .count(),
        actual_net_pnl_usd: rows.iter().map(|row| row.actual_net_pnl_30d_usd).sum(),
        estimated_net_pnl_usd: rows.iter().map(|row| row.estimated_net_pnl_30d_usd).sum(),
    }
}

#[component]
fn StrategySummaryStrip(summary: Memo<StrategySummary>) -> impl IntoView {
    view! {
        <div class="review-strategy-summary">
            <div><span>"策略样本"</span><strong>{move || summary.get().strategies}</strong><small>{move || format!("{} 个证据不全", summary.get().partial_strategies)}</small></div>
            <div><span>"可计算交易"</span><strong>{move || summary.get().computable_trades}</strong><small>{move || trade_mix_label(&summary.get())}</small></div>
            <div><span>"已确认净收益"</span><strong class=move || proven_signed_class(summary.get().actual_trades > 0, summary.get().actual_net_pnl_usd)>{move || proven_signed_money(summary.get().actual_trades > 0, summary.get().actual_net_pnl_usd)}</strong><small>{move || format!("{} 笔终态可核验", summary.get().actual_trades)}</small></div>
            <div><span>"估算净收益"</span><strong class=move || proven_signed_class(summary.get().estimated_trades > 0, summary.get().estimated_net_pnl_usd)>{move || proven_signed_money(summary.get().estimated_trades > 0, summary.get().estimated_net_pnl_usd)}</strong><small>{move || format!("{} 笔含估算", summary.get().estimated_trades)}</small></div>
        </div>
    }
}

fn trade_mix_label(summary: &StrategySummary) -> String {
    let mut label = format!(
        "已确认 {} · 估算 {}",
        summary.actual_trades, summary.estimated_trades
    );
    if summary.unknown_provenance_trades > 0 {
        label.push_str(&format!(" · 待核 {}", summary.unknown_provenance_trades));
    }
    label
}

fn strategy_detail(row: &StrategyPerformance, on_close: Callback<()>) -> impl IntoView {
    let title = strategy_label(row.kind);
    let sample = sample_label(row);
    view! {
        <section id=STRATEGY_DETAIL_ID class="review-strategy-detail" aria-label="当前策略绩效证据" tabindex="-1">
            <header><div><span>"策略绩效证据"</span><strong>{title}</strong></div><button class="review-detail-close" type="button" on:click=move |_| on_close.run(())>"关闭"</button></header>
            <div class="review-strategy-evidence"><strong>"样本口径"</strong><span>{sample}</span></div>
            <div class="review-strategy-metrics">
                <StrategyMetric label="已确认净 PnL" value=proven_signed_money(row.actual_trades_30d > 0, row.actual_net_pnl_30d_usd) class=proven_signed_class(row.actual_trades_30d > 0, row.actual_net_pnl_30d_usd)/>
                <StrategyMetric label="估算净 PnL" value=proven_signed_money(row.estimated_trades_30d > 0, row.estimated_net_pnl_30d_usd) class=proven_signed_class(row.estimated_trades_30d > 0, row.estimated_net_pnl_30d_usd)/>
                <StrategyMetric label="独立闭环" value=row.independent_periods_30d.to_string()/>
                <StrategyMetric label="命中率" value=pct(row.hit_rate_pct)/>
                <StrategyMetric label="尾损 P95" value=optional_money(row.tail_loss_p95_usd)/>
                <StrategyMetric label="最差单笔" value=optional_money(row.worst_trade_pnl_usd)/>
                <StrategyMetric label="终态 P50 / P95" value=format!("{} / {}", latency(row.finality_latency_p50_ms), latency(row.finality_latency_p95_ms))/>
                <StrategyMetric label="订单错误率" value=row.trade_order_error_rate_pct.map(pct).unwrap_or_else(|| "未知".into())/>
            </div>
        </section>
    }
}

#[component]
fn StrategyMetric(
    #[prop(into)] label: String,
    value: String,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    view! { <div><span>{label}</span><strong class=class>{value}</strong></div> }
}

fn sample_label(row: &StrategyPerformance) -> String {
    let confidence = row
        .lowest_fill_confidence
        .map(fill_confidence_label)
        .unwrap_or("缺成交置信度");
    format!(
        "{}d · {} · 可计算 {}/{} · 已确认 {} · 估算 {} · 闭环 {} · 跳过 {} · {}",
        row.sample_window_days,
        sample_status_label(row.sample_status),
        row.trades_30d,
        row.total_trades_30d,
        row.actual_trades_30d,
        row.estimated_trades_30d,
        row.independent_periods_30d,
        row.skipped_trades_30d,
        confidence
    )
}

fn net_breakdown_label(row: &StrategyPerformance) -> String {
    format!(
        "已确认 {} · 估算 {}",
        proven_signed_money(row.actual_trades_30d > 0, row.actual_net_pnl_30d_usd),
        proven_signed_money(row.estimated_trades_30d > 0, row.estimated_net_pnl_30d_usd)
    )
}

fn ratio(value: Option<f64>) -> String {
    value.map_or_else(|| "未知".to_owned(), |value| format!("{value:.2}"))
}

fn optional_money(value: Option<f64>) -> String {
    value.map_or_else(|| "无亏损样本".to_owned(), money)
}

fn latency(value: Option<u64>) -> String {
    value.map_or_else(|| "未知".to_owned(), |value| format!("{value}ms"))
}

fn sample_status_label(status: StrategyPerformanceSampleStatus) -> &'static str {
    match status {
        StrategyPerformanceSampleStatus::Complete => "样本完整",
        StrategyPerformanceSampleStatus::PartialEvidence => "证据不全",
        StrategyPerformanceSampleStatus::NoCompleteSample => "无完整样本",
        StrategyPerformanceSampleStatus::NoTrades => "无交易样本",
    }
}
