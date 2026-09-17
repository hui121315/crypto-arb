use leptos::prelude::*;
use shared_types::{ExecutedTrade, ExecutionLedgerEventType, OrderSide, ReviewLedgerEventEvidence};

use super::executed_ledger_detail::payload_summary;
use super::format::{minutes_ago, order_update_source_label};

pub(in crate::panels::modules::review) fn executed_event_timeline(
    row: &ExecutedTrade,
) -> impl IntoView {
    let items = timeline_items(row);
    view! {
        <section class="review-event-section" aria-label="交易账本事件时间线">
            <header><strong>"账本事件"</strong><span>{format!("{} 条", items.len())}</span></header>
            {if items.is_empty() {
                view! {
                    <div class="review-timeline-empty"><strong>"暂无账本事件"</strong><span>"当前交易只有汇总证据，不能伪造事件顺序。"</span></div>
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
    events.into_iter().map(timeline_item).collect()
}

fn timeline_item(event: &ReviewLedgerEventEvidence) -> TimelineItem {
    TimelineItem {
        time: minutes_ago(event.timing.occurred_at_ms),
        title: format!(
            "{} · {} {} {}",
            event_type_label(event.event_type),
            event.order.exchange,
            side_label(event.order.side),
            event.order.symbol,
        ),
        payload: payload_summary(&event.payload),
        source: format!("来源 · {}", order_update_source_label(event.source)),
    }
}

fn event_type_label(event_type: ExecutionLedgerEventType) -> &'static str {
    match event_type {
        ExecutionLedgerEventType::OrderState => "订单状态",
        ExecutionLedgerEventType::FillSnapshot | ExecutionLedgerEventType::FillEvent => "成交",
        ExecutionLedgerEventType::FeeSnapshot => "费用",
        ExecutionLedgerEventType::FundingPayment => "Funding",
        ExecutionLedgerEventType::Slippage => "滑点",
        ExecutionLedgerEventType::OrderbookEvidence => "盘口证据",
        ExecutionLedgerEventType::Cancel => "撤单",
    }
}

fn side_label(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "买入",
        OrderSide::Sell => "卖出",
    }
}
