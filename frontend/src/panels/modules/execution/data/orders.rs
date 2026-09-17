//! 执行订单 feed 装配：REST seed + WS 流 + 兜底轮询写入 workstation-owned [`OrderQueue`]。
//! 队列状态机见 `orders/queue.rs`，按 run 投影 + problem memo 见 `orders/projection.rs`。

#[cfg(test)]
#[path = "orders/fixtures.rs"]
mod fixtures;
#[path = "orders/projection.rs"]
mod projection;
#[path = "orders/queue.rs"]
mod queue;

use crate::api::ws::{start_order_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::polling::{use_ws_channel_snapshot_fallback, SnapshotFallbackTiming};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::time::Duration;

pub(crate) use projection::{
    all_orders_memo, order_seed_problem_memo, order_stream_problem_memo, orders_for_run_memo,
};
pub(crate) use queue::OrderQueue;

pub(super) const ORDERS_CHANNEL: &str = "orders";
const ORDERS_FALLBACK_TIMING: SnapshotFallbackTiming = SnapshotFallbackTiming {
    period: Duration::from_secs(5),
    grace: Duration::from_secs(8),
    stale_after: Duration::from_secs(10),
};

pub(crate) fn use_order_queue(
    rows: RwSignal<OrderQueue>,
    channel_state: RwSignal<WsChannelState>,
    refresh_nonce: RwSignal<u64>,
) -> RwSignal<OrderQueue> {
    let client = use_global().client;
    let seed_client = client.clone();
    Effect::new(move |_| {
        refresh_nonce.get();
        let client = seed_client.clone();
        spawn_local(async move {
            let result = client.trading_orders().await.map_err(|error| error.problem);
            rows.update(|queue| queue.seed(result));
        });
    });
    let handle = start_order_stream_with_state(
        channel_state,
        move |event| rows.update(|queue| queue.apply_stream_payload(event)),
        move |problem| rows.update(|queue| queue.note_stream_problem(problem)),
    );
    on_cleanup(move || handle.cancel());
    use_ws_channel_snapshot_fallback(
        channel_state,
        ORDERS_FALLBACK_TIMING,
        || true,
        move || {
            let client = client.clone();
            async move { client.trading_orders().await.map_err(|error| error.problem) }
        },
        move |result| rows.update(|queue| queue.seed(result)),
    );
    rows
}
