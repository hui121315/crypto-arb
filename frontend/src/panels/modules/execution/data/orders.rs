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
use crate::state::polling::{use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming};
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
    let reading = RwSignal::new(false);
    let seed_finished = RwSignal::new(false);
    let read_again = RwSignal::new(false);
    let request_version = RwSignal::new(0_u64);
    let retry_nonce = RwSignal::new(0_u64);
    Effect::new(move |_| {
        refresh_nonce.get();
        retry_nonce.get();
        request_version.update(|version| *version = version.wrapping_add(1));
        if reading.get_untracked() {
            read_again.set(true);
            return;
        }
        reading.set(true);
        let base = seed_client.base_url();
        let client = seed_client.clone();
        spawn_local(async move {
            let result = client.trading_orders().await.map_err(|error| error.problem);
            if reading.try_get_untracked().is_none() {
                return;
            }
            reading.set(false);
            if read_again.get_untracked() {
                read_again.set(false);
                retry_nonce.update(|value| *value = value.wrapping_add(1));
                return;
            }
            if base != client.base_url() {
                return;
            }
            rows.update(|queue| queue.seed(result));
            seed_finished.set(true);
        });
    });
    let handle = start_order_stream_with_state(
        channel_state,
        move |event| rows.update(|queue| queue.apply_stream_payload(event)),
        move |problem| rows.update(|queue| queue.note_stream_problem(problem)),
    );
    on_cleanup(move || handle.cancel());
    let result_client = client.clone();
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        ORDERS_FALLBACK_TIMING,
        // The fallback resource can start before the initial read effect runs.
        move || seed_finished.get_untracked() && !reading.get_untracked(),
        move || {
            let client = client.clone();
            let context = (client.base_url(), request_version.get_untracked());
            reading.set(true);
            async move {
                (
                    context,
                    client.trading_orders().await.map_err(|error| error.problem),
                )
            }
        },
        move |(base, version), result| {
            reading.set(false);
            if read_again.get_untracked() {
                read_again.set(false);
                retry_nonce.update(|value| *value = value.wrapping_add(1));
                return;
            }
            if base == result_client.base_url() && version == request_version.get_untracked() {
                rows.update(|queue| queue.seed(result));
            }
        },
    );
    rows
}
