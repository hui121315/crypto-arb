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
    submission_pending: Memo<bool>,
    seed_ready: Memo<bool>,
    details_reading: RwSignal<bool>,
    detail_problem: Memo<Option<ApiProblem>>,
    refresh_details: Callback<()>,
) -> impl IntoView {
    let known_count = Memo::new(move |_| run.with(|run| run.as_ref().map_or(0, |run| {
        run.long_leg.order_ids.iter().chain(&run.short_leg.order_ids)
            .filter(|id| !id.is_empty()).collect::<std::collections::BTreeSet<_>>().len()
    })));
    let missing = Memo::new(move |_| known_count.get().saturating_sub(orders.with(Vec::len)));
    let read_unconfirmed = Memo::new(move |_| !seed_ready.get() || seed_problem.get().is_some());
    view! {
        <div class="orders-list execution-order-queue">
            <QueueOverview
                run=run
                run_is_current=run_is_current
                orders=orders
                seed_problem=seed_problem
                stream_problem=stream_problem
                channel_state=channel_state
                submission_pending=submission_pending
                known_count=known_count
                read_unconfirmed=read_unconfirmed
            />
            <QueueTransport channel_state=channel_state/>
            <QueueProblems seed_problem=seed_problem stream_problem=stream_problem/>
            <OrderFeed run=run run_is_current=run_is_current orders=orders submission_pending=submission_pending
                known_count=known_count missing=missing read_unconfirmed=read_unconfirmed
                details_reading=details_reading detail_problem=detail_problem refresh_details=refresh_details/>
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
    submission_pending: Memo<bool>,
    known_count: Memo<usize>,
    read_unconfirmed: Memo<bool>,
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
        if submission_pending.get() { "原提交待核对".to_owned() }
        else { orders.with(|rows| active_state_label(run.get().as_ref(), rows, problem.get().as_ref())) }
    });
    let tone = Memo::new(move |_| {
        if submission_pending.get() { "warning" }
        else { orders.with(|rows| active_state_tone(run.get().as_ref(), rows, problem.get().as_ref())) }
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
                <Show when=move || run.get().is_some() && !submission_pending.get() fallback=move || view! {
                    <em class="queue-run-empty">{move || if submission_pending.get() {
                        "尚未取得原请求的交易记录，不代表未下单"
                    } else { "尚未读取到运行记录" }}</em>
                }>
                    <details class="queue-run-identity">
                        <summary title="展开运行标识">
                            <em>{move || run_scope_label(run.get().as_ref())}</em>
                            <span>"标识"</span>
                        </summary>
                        <code>{move || run.get().as_ref().map(run_identity_label).unwrap_or_default()}</code>
                    </details>
                </Show>
            </div>
            <span class="queue-count">
                {move || if submission_pending.get() && orders.with(Vec::is_empty) { "待确认".into() }
                    else if known_count.get() > 0 { format!("已知 {} 单", known_count.get()) }
                    else if orders.with(Vec::is_empty) && read_unconfirmed.get() { "待确认".into() }
                    else { orders.with(|rows| order_count_label(rows.len(), problem.get().as_ref())) }}
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
                        <details><summary>"查看错误"</summary>
                            <p>{move || stream_problem.get().as_ref().map(problem_text).unwrap_or_default()}</p>
                        </details>
                    </div>
                </div>
            </Show>
            <Show when=move || seed_problem.get().is_some()>
                <div class="queue-problem">
                    <span>"REST"</span>
                    <div>
                        <strong>"快照恢复失败"</strong>
                        <details><summary>"查看错误"</summary>
                            <p>{move || seed_problem.get().as_ref().map(problem_text).unwrap_or_default()}</p>
                        </details>
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
    submission_pending: Memo<bool>,
    known_count: Memo<usize>,
    missing: Memo<usize>,
    read_unconfirmed: Memo<bool>,
    details_reading: RwSignal<bool>,
    detail_problem: Memo<Option<ApiProblem>>,
    refresh_details: Callback<()>,
) -> impl IntoView {
    let visible_count = RwSignal::new(12_usize);
    let ids = Memo::new(move |_| {
        let mut rows = orders.get();
        // Progress updates must not move an order out from under the reader.
        rows.sort_by_key(|row| (Reverse(row.intent.created_at_ms), row.intent.id.clone()));
        rows.into_iter()
            .take(visible_count.get())
            .map(|row| row.intent.id)
            .collect::<Vec<_>>()
    });
    view! {
        <section class="queue-feed">
            <header class="queue-feed-head">
                <strong>{move || order_feed_label(run.get().is_some(), run_is_current.get())}</strong>
                <span>{move || orders.with(|rows| if known_count.get() > 0 {
                    format!("明细 {} / {}", rows.len(), known_count.get())
                } else { format!("显示 {} / {}", rows.len().min(visible_count.get()), rows.len()) })}</span>
            </header>
            <Show when=move || { missing.get() > 0 }>
                <section class="queue-detail-coverage" aria-label="订单明细补齐">
                    <div role="status">
                        <strong>{move || format!("{} 笔明细{}", missing.get(), if details_reading.get() { "读取中" } else { "待补齐" })}</strong>
                        <span>"运行记录已有订单编号；明细缺失不代表未下单。"</span>
                        <Show when=move || detail_problem.get().is_some()>
                            <details><summary>"查看读取原因"</summary>
                                <p>{move || detail_problem.get().as_ref().map(problem_text).unwrap_or_default()}</p>
                            </details>
                        </Show>
                    </div>
                    <button type="button" class="icon-button" title="补读缺失订单明细" aria-label="补读缺失订单明细"
                        disabled=move || details_reading.get()
                        on:click=move |_| refresh_details.run(())><span aria-hidden="true">"↻"</span></button>
                </section>
            </Show>
            <div class="queue-feed-columns" aria-hidden="true">
                <span>"更新时间"</span>
                <span>"环境 / 状态"</span>
                <span>"名义金额"</span>
                <span>"场所 / 标的"</span>
                <span>"订单明细"</span>
            </div>
            <Show when=move || ids.with(Vec::is_empty)>
                <div class="queue-empty">
                    <strong>{move || if submission_pending.get() { "等待原订单处理结果" }
                        else if missing.get() > 0 { "订单明细待补齐" }
                        else if read_unconfirmed.get() { "订单记录待确认" }
                        else { empty_orders_label(run.get().is_some(), run_is_current.get()) }}</strong>
                    <span>{move || if submission_pending.get() { "结果尚未确认，请勿重复提交" }
                        else if missing.get() > 0 { "尚未读取到这些订单的明细" }
                        else if read_unconfirmed.get() { "尚未完成有效读取，不能判断有无订单" }
                        else { "当前读取范围内暂无记录" }}</span>
                </div>
            </Show>
            <For each=move || ids.get() key=|id| id.clone() children=move |id| {
                let row_id = id.clone();
                let row = Memo::new(move |_| orders.with(|rows| rows.iter().find(|row| row.intent.id == row_id).cloned()));
                view! { <OrderRow id=id row=row/> }
            }/>
            <Show when=move || { orders.with(Vec::len) > visible_count.get() }>
                <button class="queue-show-more" on:click=move |_| visible_count.update(|count| *count = count.saturating_add(12))>
                    "显示更多订单"
                </button>
            </Show>
        </section>
    }
}

#[component]
fn OrderRow(id: String, row: Memo<Option<OrderRecord>>) -> impl IntoView {
    view! {
        <article class="queue-order-item" data-order-id=id
            data-state=move || row.with(|row| row.as_ref().map(|row| order_state_tone(row.state)))
            title=move || row.with(|row| row.as_ref().map(order_label))>
            <header>
                <time>{move || row.with(|row| row.as_ref().map(|row| history_time_label(row.updated_at_ms)))}</time>
                <div class="queue-order-state">
                    <span class="queue-order-environment"
                        data-environment=move || row.with(|row| row.as_ref().map(|row| order_environment_tone(row.intent.mode)))>
                        {move || row.with(|row| row.as_ref().map(|row| mode_label(row.intent.mode)))}
                    </span>
                    <span class="queue-order-status">{move || row.with(|row| row.as_ref().map(|row| state_label(row.state)))}</span>
                </div>
                <strong title="名义金额">{move || row.with(|row| row.as_ref().map(notional_label))}</strong>
            </header>
            <div class="queue-order-copy">
                <strong>{move || row.with(|row| row.as_ref().map(order_primary_label))}</strong>
                <details class="queue-order-detail">
                    <summary title="展开订单详情">
                        <span>{move || row.with(|row| row.as_ref().map(order_detail_label))}</span>
                        <b>"详情"</b>
                    </summary>
                    <p>{move || row.with(|row| row.as_ref().map(order_label))}</p>
                    <p>{move || row.with(|row| row.as_ref().and_then(|row| row.message.clone()))}</p>
                    <dl>
                        <dt>"委托数量"</dt><dd>{move || row.with(|row| labels::quantity_label(row.as_ref().map(|row| row.intent.quantity)))}</dd>
                        <dt>"已成交数量"</dt><dd>{move || row.with(|row| labels::quantity_label(row.as_ref().and_then(|row| row.filled_quantity)))}</dd>
                        <dt>"内部订单"</dt><dd>{move || row.with(|row| row.as_ref().map(|row| row.intent.id.clone()))}</dd>
                        <dt>"交易所订单"</dt><dd>{move || row.with(|row| row.as_ref().and_then(|row| row.exchange_order_id.clone()).unwrap_or_else(|| "待确认".into()))}</dd>
                    </dl>
                </details>
            </div>
        </article>
    }
}
