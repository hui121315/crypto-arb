//! 订单列表共享原子组件：装配活动行 + WS/REST problem 行 + 排序后的最近订单行。
//! 文案标签见 `orders_list/labels.rs`，名义金额推导见 `orders_list/notional.rs`。

#[path = "orders_list/labels.rs"]
mod labels;
#[path = "orders_list/notional.rs"]
mod notional;
#[cfg(test)]
#[path = "orders_list/testing.rs"]
mod testing;

use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionRun, OrderRecord};
use std::cmp::Reverse;

use crate::api::ws::WsChannelState;

use labels::{
    active_problem, active_state_label, active_state_tone, channel_meta_label, channel_problem,
    channel_state_tone, empty_orders_label, history_time_label, mode_label, order_count_label,
    order_detail_label, order_environment_tone, order_feed_label, order_label, order_primary_label,
    order_state_tone, problem_text, run_identity_label, run_scope_label, state_label,
};
use notional::notional_label;

#[component]
pub fn OrdersList(
    run: RwSignal<Option<ExecutionRun>>,
    run_is_current: Memo<bool>,
    orders: Memo<Vec<OrderRecord>>,
    seed_problem: Memo<Option<ApiProblem>>,
    stream_problem: Memo<Option<ApiProblem>>,
    channel_state: RwSignal<WsChannelState>,
) -> impl IntoView {
    view! {
        <div class="orders-list execution-order-queue">
            <QueueOverview
                run=run
                run_is_current=run_is_current
                orders=orders
                seed_problem=seed_problem
                stream_problem=stream_problem
                channel_state=channel_state
            />
            <QueueTransport channel_state=channel_state/>
            <QueueProblems seed_problem=seed_problem stream_problem=stream_problem/>
            <OrderFeed run=run run_is_current=run_is_current orders=orders/>
        </div>
    }
}

#[component]
fn QueueOverview(
    run: RwSignal<Option<ExecutionRun>>,
    run_is_current: Memo<bool>,
    orders: Memo<Vec<OrderRecord>>,
    seed_problem: Memo<Option<ApiProblem>>,
    stream_problem: Memo<Option<ApiProblem>>,
    channel_state: RwSignal<WsChannelState>,
) -> impl IntoView {
    let problem = Memo::new(move |_| {
        let channel = channel_state.get();
        active_problem(
            seed_problem.get(),
            stream_problem.get(),
            channel_problem(&channel),
        )
    });
    let state = Memo::new(move |_| {
        orders.with(|rows| active_state_label(run.get().as_ref(), rows, problem.get().as_ref()))
    });
    let tone = Memo::new(move |_| {
        orders.with(|rows| active_state_tone(run.get().as_ref(), rows, problem.get().as_ref()))
    });

    view! {
        <section class="queue-overview" data-state=move || tone.get()>
            <div class="queue-overview-copy">
                <span>{move || match (run.get().is_some(), run_is_current.get()) {
                    (true, true) => "当前执行",
                    (true, false) => "上一笔执行",
                    (false, _) => "最近订单",
                }}</span>
                <strong>{move || state.get()}</strong>
                {move || run.get().map_or_else(
                    || view! {
                        <em class="queue-run-empty">"尚未创建 ExecutionRun"</em>
                    }.into_any(),
                    |run| view! {
                        <details class="queue-run-identity">
                            <summary title="展开运行标识">
                                <em>{run_scope_label(Some(&run))}</em>
                                <span>"标识"</span>
                            </summary>
                            <code>{run_identity_label(&run)}</code>
                        </details>
                    }.into_any(),
                )}
            </div>
            <span class="queue-count">
                {move || orders.with(|rows| order_count_label(rows.len(), problem.get().as_ref()))}
            </span>
        </section>
    }
}

#[component]
fn QueueTransport(channel_state: RwSignal<WsChannelState>) -> impl IntoView {
    view! {
        <section class="queue-transport" data-state=move || channel_state_tone(&channel_state.get())>
            <span class="queue-transport-dot" aria-hidden="true"></span>
            <div>
                <strong>"订单通道"</strong>
                <em title=move || channel_meta_label(&channel_state.get())>
                    {move || channel_meta_label(&channel_state.get())}
                </em>
            </div>
            <i>{move || channel_state.get().channel}</i>
        </section>
    }
}

#[component]
fn QueueProblems(
    seed_problem: Memo<Option<ApiProblem>>,
    stream_problem: Memo<Option<ApiProblem>>,
) -> impl IntoView {
    view! {
        <div class="queue-problems">
            <Show when=move || stream_problem.get().is_some()>
                <div class="queue-problem">
                    <span>"WS"</span>
                    <div>
                        <strong>"订单流异常"</strong>
                        <em title=move || stream_problem.get().as_ref().map(problem_text).unwrap_or_default()>
                            {move || stream_problem.get().as_ref().map(problem_text).unwrap_or_default()}
                        </em>
                    </div>
                </div>
            </Show>
            <Show when=move || seed_problem.get().is_some()>
                <div class="queue-problem">
                    <span>"REST"</span>
                    <div>
                        <strong>"快照恢复失败"</strong>
                        <em title=move || seed_problem.get().as_ref().map(problem_text).unwrap_or_default()>
                            {move || seed_problem.get().as_ref().map(problem_text).unwrap_or_default()}
                        </em>
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn OrderFeed(
    run: RwSignal<Option<ExecutionRun>>,
    run_is_current: Memo<bool>,
    orders: Memo<Vec<OrderRecord>>,
) -> impl IntoView {
    view! {
        <section class="queue-feed">
            <header class="queue-feed-head">
                <strong>{move || order_feed_label(run.get().is_some(), run_is_current.get())}</strong>
                <span>{move || orders.with(|rows| format!("显示 {} / {}", rows.len().min(12), rows.len()))}</span>
            </header>
            <div class="queue-feed-columns" aria-hidden="true">
                <span>"更新时间"</span>
                <span>"环境 / 状态"</span>
                <span>"名义金额"</span>
                <span>"场所 / 标的"</span>
                <span>"订单明细"</span>
            </div>
            {move || {
                let mut rows = orders.get();
                let has_run = run.get().is_some();
                rows.sort_by_key(|order| Reverse(order.updated_at_ms));
                if rows.is_empty() {
                    return view! {
                        <div class="queue-empty">
                            <strong>{empty_orders_label(has_run, run_is_current.get())}</strong>
                            <span>{if has_run && run_is_current.get() {
                                "当前 ExecutionRun"
                            } else if has_run {
                                "上一笔 ExecutionRun"
                            } else {
                                "订单历史"
                            }}</span>
                        </div>
                    }.into_any();
                }
                rows.into_iter().take(12).map(|order| {
                    let title = order_label(&order);
                    let primary = order_primary_label(&order);
                    let detail = order_detail_label(&order);
                    let has_message = order
                        .message
                        .as_deref()
                        .is_some_and(|message| !message.trim().is_empty());
                    let detail_title = detail.clone();
                    let detail_body = detail.clone();
                    let state = state_label(order.state);
                    let tone = order_state_tone(order.state);
                    let environment = mode_label(order.intent.mode);
                    let environment_tone = order_environment_tone(order.intent.mode);
                    let time = history_time_label(order.updated_at_ms);
                    let notional = notional_label(&order);
                    view! {
                        <article class="queue-order-item" data-state=tone title=title>
                            <header>
                                <time>{time}</time>
                                <div class="queue-order-state">
                                    <span class="queue-order-environment" data-environment=environment_tone>
                                        {environment}
                                    </span>
                                    <span class="queue-order-status">{state}</span>
                                </div>
                                <strong>{notional}</strong>
                            </header>
                            <div class="queue-order-copy">
                                <strong>{primary}</strong>
                                {if has_message {
                                    view! {
                                        <details class="queue-order-detail">
                                            <summary title=detail_title>
                                                <span>{detail}</span>
                                                <b>"详情"</b>
                                            </summary>
                                            <p>{detail_body}</p>
                                        </details>
                                    }.into_any()
                                } else {
                                    view! { <em>{detail}</em> }.into_any()
                                }}
                            </div>
                        </article>
                    }
                }).collect_view().into_any()
            }}
        </section>
    }
}
