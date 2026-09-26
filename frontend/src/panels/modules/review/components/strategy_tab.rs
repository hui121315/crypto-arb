use leptos::prelude::*;
use shared_types::{StrategyPerformance, StrategyPerformanceSampleStatus};

use crate::panels::modules::pagination::{page_controls, use_table_runtime};

use super::format::{
    environment_label, environment_token, fill_confidence_label, money, pct, proven_signed_class, proven_signed_money, strategy_label,
};
use super::ReviewSectionRows;

const PAGE_SIZE: usize = 50;
const STRATEGY_PAGE_STORAGE_KEY: &str = "crossline.review.strategy.page";
const STRATEGY_DETAIL_ID: &str = "review-strategy-detail";
type StrategyKey = (shared_types::StrategyKind, Option<shared_types::ExecutionEnvironment>);

fn strategy_key(row: &StrategyPerformance) -> StrategyKey {
    (row.kind, row.execution_environment)
}

pub(in crate::panels::modules::review) fn strategy_tab(
    section: Memo<ReviewSectionRows<StrategyPerformance>>,
) -> impl IntoView {
    let rows = Memo::new(move |_| section.with(|section| section.rows.clone()));
    let dataset_key = Memo::new(move |_| section.with(strategy_dataset_key));
    let table = use_table_runtime(STRATEGY_PAGE_STORAGE_KEY, dataset_key, rows, PAGE_SIZE);
    let selected = RwSignal::new(None::<StrategyKey>);
    let selected_row = Memo::new(move |_| {
        let selected_kind = selected.get()?;
        rows.get().into_iter().find(|row| strategy_key(row) == selected_kind)
    });
    let close_detail = Callback::new(move |()| selected.set(None));

    view! {
        <StrategySummaryStrip rows=rows/>
        <Show when=move || table.runtime.with(|table| table.rows.is_empty())>
                    <div class="review-business-empty">
                        <strong>{move || section.get().empty_text("30D 内暂无完整策略样本")}</strong>
                    </div>
        </Show>
        <Show when=move || table.runtime.with(|table| !table.rows.is_empty())>
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
                                        <th>"数据依据"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    <For each=move || table.runtime.get().rows key=strategy_key children=move |initial| {
                                        let row = Memo::new(move |_| rows.with(|rows| rows.iter().find(|row| strategy_key(row) == strategy_key(&initial)).cloned().unwrap_or_else(|| initial.clone())));
                                        view! { <StrategyRow row=row selected=selected/> }
                                    }/>
                                </tbody>
                            </table>
                        </div>
                        {move || (table.total.get() > PAGE_SIZE).then(|| view! {
                            {page_controls(table.total, table.current_page, PAGE_SIZE)}
                        })}
                    </div>
                    <For each=move || selected_row.get().into_iter() key=strategy_key children=move |initial| {
                        let row = Memo::new(move |_| selected_row.get().filter(|row| strategy_key(row) == strategy_key(&initial)).unwrap_or_else(|| initial.clone()));
                        strategy_detail(row, close_detail)
                    }/>
                </div>
        </Show>
    }
}

fn strategy_dataset_key(section: &ReviewSectionRows<StrategyPerformance>) -> String {
    let mut parts = Vec::with_capacity(4);
    parts.push(section.rows.len().to_string());
    for row in section.rows.iter().take(2) {
        parts.push(format!("{:?}:{}", strategy_key(row), row.trades_30d));
    }
    if let Some(row) = section.rows.last() {
        parts.push(format!("{:?}:{}", strategy_key(row), row.trades_30d));
    }
    parts.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionFillConfidence, StrategyKind};

    #[test]
    fn missing_actual_samples_are_not_zero_profit_or_no_losses() {
        assert_eq!(proven_signed_money(false, 0.0), "—");
        assert_eq!(optional_money(None), "待确认");
        assert_eq!(optional_money(Some(-2.0)), "-$2.00");
    }

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
        assert!(label.contains("数据依据不全"));
        assert!(label.contains("可计算 2/3"));
        assert!(label.contains("已确认 1"));
        assert!(label.contains("估算 1"));
        assert!(label.contains("完整流程 0"));
        assert!(label.contains("跳过 1"));
        assert!(label.contains("仅 受理确认 推定"));
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
            execution_environment: Some(shared_types::ExecutionEnvironment::Paper),
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
    row: Memo<StrategyPerformance>,
    selected: RwSignal<Option<StrategyKey>>,
) -> impl IntoView {
    let kind = strategy_key(&row.get_untracked());
    let label_kind = kind;
    view! {
        <tr
            data-strategy=kind.0.label_zh()
            data-environment=environment_token(kind.1)
            class:is-selected=move || selected.with(|current| current == &Some(kind))
            aria-selected=move || selected.with(|current| current == &Some(kind)).to_string()
        >
            {move || {
                let row = row.get();
                let proven = row.actual_trades_30d > 0;
                let net_class = proven_signed_class(proven, row.net_pnl_30d_usd);
                let avg_class = proven_signed_class(proven, row.avg_pnl_per_trade_usd);
                let confidence = row.lowest_fill_confidence.map(fill_confidence_label).unwrap_or("缺成交置信度");
                let net_breakdown = net_breakdown_label(&row);
                view! { <>
            <td><strong>{strategy_label(row.kind)}</strong><small class="review-trade-environment" data-environment=environment_token(row.execution_environment)>
                {format!("{} · {}D", environment_label(row.execution_environment), row.sample_window_days)}</small></td>
            <td><strong>{format!("{} / {}", row.trades_30d, row.total_trades_30d)}</strong><small>{sample_status_label(row.sample_status)} " · " {confidence}</small></td>
            <td><strong>{if proven { format!("{} / {}", row.profitable_trades_30d, row.losing_trades_30d) } else { "待确认".into() }}</strong><small>{if proven { format!("{} 笔持平", row.break_even_trades_30d) } else { "无已确认样本".into() }}</small></td>
            <td class=net_class><strong>{proven_signed_money(proven, row.net_pnl_30d_usd)}</strong><small>{net_breakdown}</small></td>
            <td class=avg_class>{proven_signed_money(proven, row.avg_pnl_per_trade_usd)}</td>
            <td>{ratio(row.profit_factor)}</td>
            <td class=if proven { "negative" } else { "muted" }>{if proven { money(row.max_drawdown_usd) } else { "待确认".into() }}</td>
                </> }
            }}
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
        strategies: rows.iter().map(|row| row.kind).collect::<std::collections::HashSet<_>>().len(),
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
fn StrategySummaryStrip(rows: Memo<Vec<StrategyPerformance>>) -> impl IntoView {
    let summary = Memo::new(move |_| strategy_summary(&rows.get()));
    let unknown = Memo::new(move |_| rows.with(|rows| rows.iter().filter(|row| row.execution_environment.is_none())
        .map(|row| row.total_trades_30d).sum::<u32>()));
    view! {
        <div class="review-strategy-summary">
            <div><span>"策略种类"</span><strong>{move || summary.get().strategies}</strong><small>{move || format!("{} 组样本 · {} 组数据依据不全", rows.with(Vec::len), summary.get().partial_strategies)}</small></div>
            {[shared_types::ExecutionEnvironment::Live, shared_types::ExecutionEnvironment::Paper].into_iter().map(move |environment| {
                let totals = Memo::new(move |_| rows.with(|rows| strategy_summary(&rows.iter()
                    .filter(|row| row.execution_environment == Some(environment)).cloned().collect::<Vec<_>>())));
                view! { <div>
                    <span>{format!("{}已确认净收益", environment_label(Some(environment)))}</span>
                    <strong class=move || proven_signed_class(totals.get().actual_trades > 0, totals.get().actual_net_pnl_usd)>
                        {move || proven_signed_money(totals.get().actual_trades > 0, totals.get().actual_net_pnl_usd)}</strong>
                    <small>{move || { let totals = totals.get(); format!("已确认 {} 笔 · 估算 {}（{} 笔）", totals.actual_trades,
                        proven_signed_money(totals.estimated_trades > 0, totals.estimated_net_pnl_usd), totals.estimated_trades) }}</small>
                </div> }
            }).collect_view()}
            <div><span>"可计算交易"</span><strong>{move || summary.get().computable_trades}</strong>
                <small>{move || trade_mix_label(&summary.get())}</small><small>{move || format!("其中 {} 笔环境待核对，不计入实盘/模拟合计", unknown.get())}</small></div>
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

fn strategy_detail(row: Memo<StrategyPerformance>, on_close: Callback<()>) -> impl IntoView {
    view! {
        <section id=STRATEGY_DETAIL_ID class="review-strategy-detail" aria-label="当前策略绩效数据依据" tabindex="-1">
            <header><div><span>"策略绩效数据依据"</span><strong>{move || format!("{} · {}", strategy_label(row.get().kind), environment_label(row.get().execution_environment))}</strong></div><button class="review-detail-close" type="button" on:click=move |_| on_close.run(())>"关闭"</button></header>
            <div class="review-strategy-evidence"><strong>"样本口径"</strong><span>{move || sample_label(&row.get())}</span></div>
            <div class="review-strategy-metrics">
                {move || { let row = row.get(); view! { <>
                <StrategyMetric label="已确认净 PnL" value=proven_signed_money(row.actual_trades_30d > 0, row.actual_net_pnl_30d_usd) class=proven_signed_class(row.actual_trades_30d > 0, row.actual_net_pnl_30d_usd)/>
                <StrategyMetric label="估算净 PnL" value=proven_signed_money(row.estimated_trades_30d > 0, row.estimated_net_pnl_30d_usd) class=proven_signed_class(row.estimated_trades_30d > 0, row.estimated_net_pnl_30d_usd)/>
                <StrategyMetric label="已确认单笔" value=proven_signed_money(row.actual_trades_30d > 0, row.avg_pnl_per_trade_usd) class=proven_signed_class(row.actual_trades_30d > 0, row.avg_pnl_per_trade_usd)/>
                <StrategyMetric label="Profit Factor" value=ratio(row.profit_factor)/>
                <StrategyMetric label="已确认盈 / 亏 / 平" value={if row.actual_trades_30d > 0 {
                    format!("{} / {} / {}", row.profitable_trades_30d, row.losing_trades_30d, row.break_even_trades_30d)
                } else { "待确认".into() }}/>
                <StrategyMetric label="最大回撤" value={if row.actual_trades_30d > 0 { money(row.max_drawdown_usd) } else { "待确认".into() }}/>
                <StrategyMetric label="独立完整流程" value=row.independent_periods_30d.to_string()/>
                <StrategyMetric label="命中率" value={if row.actual_trades_30d > 0 { pct(row.hit_rate_pct) } else { "待确认".into() }}/>
                <StrategyMetric label="尾损 P95" value={if row.actual_trades_30d > 0 && row.losing_trades_30d == 0 && row.tail_loss_p95_usd.is_none() {
                    "无亏损样本".into()
                } else { optional_money(row.tail_loss_p95_usd) }}/>
                <StrategyMetric label="最差单笔" value=optional_money(row.worst_trade_pnl_usd)/>
                <StrategyMetric label="最终结果 P50 / P95" value=format!("{} / {}", latency(row.finality_latency_p50_ms), latency(row.finality_latency_p95_ms))/>
                <StrategyMetric label="订单错误率" value=row.trade_order_error_rate_pct.map(pct).unwrap_or_else(|| "未知".into())/>
                </> } }}
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
        "{} · {}d · {} · 可计算 {}/{} · 已确认 {} · 估算 {} · 完整流程 {} · 跳过 {} · {}",
        environment_label(row.execution_environment),
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
    value.map_or_else(|| "待确认".to_owned(), money)
}

fn latency(value: Option<u64>) -> String {
    value.map_or_else(|| "未知".to_owned(), |value| format!("{value}ms"))
}

fn sample_status_label(status: StrategyPerformanceSampleStatus) -> &'static str {
    match status {
        StrategyPerformanceSampleStatus::Complete => "样本完整",
        StrategyPerformanceSampleStatus::PartialEvidence => "数据依据不全",
        StrategyPerformanceSampleStatus::NoCompleteSample => "无完整样本",
        StrategyPerformanceSampleStatus::NoTrades => "无交易样本",
    }
}
