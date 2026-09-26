use leptos::prelude::*;
use shared_types::{ExecutedTrade, ExecutionLedgerEventType, OrderSide, ReviewLedgerEventEvidence};

use super::executed_ledger_detail::payload_summary_in_environment;
use super::format::{order_update_source_label, record_time};

pub(in crate::panels::modules::review) fn executed_event_timeline(
    row: &ExecutedTrade,
) -> impl IntoView {
    let items = timeline_items(row);
    view! {
        <section class="review-event-section" aria-label="交易过程记录">
            <header><strong>"交易过程"</strong><span>{format!("{} 条记录", items.len())}</span></header>
            {if items.is_empty() {
                view! {
                    <div class="review-timeline-empty"><strong>"暂无逐步记录"</strong><span>"目前只有交易汇总，无法还原每一步发生的时间。"</span></div>
                }.into_any()
            } else {
                view! {
                    <ol class="review-event-timeline">
                        {items.into_iter().map(|item| view! {
                            <li>
                                <time>{item.time}</time>
                                <div>
                                    <strong>{item.title}</strong>
                                    <span>{item.payload}</span>
                                    <small>{item.source}</small>
                                </div>
                            </li>
                        }).collect_view()}
                    </ol>
                }.into_any()
            }}
        </section>
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineItem {
    time: String,
    title: String,
    payload: String,
    source: String,
}

fn timeline_items(row: &ExecutedTrade) -> Vec<TimelineItem> {
    let mut events = row.evidence.ledger_events.iter().collect::<Vec<_>>();
    events.sort_by_key(|event| event.timing.occurred_at_ms);
    events.into_iter().map(|event| timeline_item(event, row.execution_environment())).collect()
}

fn timeline_item(event: &ReviewLedgerEventEvidence, environment: Option<shared_types::ExecutionEnvironment>) -> TimelineItem {
    TimelineItem {
        time: record_time(event.timing.occurred_at_ms),
        title: format!(
            "{} · {} {} {}",
            event_type_label(event.event_type),
            event.order.exchange,
            side_label(event.order.side),
            event.order.symbol,
        ),
        payload: payload_summary_in_environment(&event.payload, environment),
        source: format!("来源 · {}", order_update_source_label(event.source)),
    }
}

fn event_type_label(event_type: ExecutionLedgerEventType) -> &'static str {
    match event_type {
        ExecutionLedgerEventType::OrderState => "订单状态",
        ExecutionLedgerEventType::FillSnapshot | ExecutionLedgerEventType::FillEvent => "成交",
        ExecutionLedgerEventType::FeeSnapshot => "费用",
        ExecutionLedgerEventType::FundingPayment => "资金费",
        ExecutionLedgerEventType::Slippage => "滑点",
        ExecutionLedgerEventType::OrderbookEvidence => "盘口数据依据",
        ExecutionLedgerEventType::Cancel => "撤单",
    }
}

fn side_label(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "买入",
        OrderSide::Sell => "卖出",
    }
}
