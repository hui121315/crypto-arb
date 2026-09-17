//! 订单投影：把队列按当前 run 的 leg `order_ids` 过滤，并暴露 seed/stream problem memo。
//!
//! 从 `orders.rs` 拆出。队列状态机见 `queue.rs`，feed 装配见 `orders.rs`。

use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionRun, OrderRecord};
use std::collections::BTreeSet;

use super::queue::OrderQueue;

pub(crate) fn orders_for_run_memo(
    queue: RwSignal<OrderQueue>,
    run: RwSignal<Option<ExecutionRun>>,
) -> Memo<Vec<OrderRecord>> {
    Memo::new(move |_| {
        run.with(|run| queue.with(|queue| orders_for_run(run.as_ref(), &queue.rows)))
    })
}

pub(crate) fn all_orders_memo(queue: RwSignal<OrderQueue>) -> Memo<Vec<OrderRecord>> {
    Memo::new(move |_| queue.with(|queue| queue.rows.clone()))
}

pub(crate) fn order_seed_problem_memo(queue: RwSignal<OrderQueue>) -> Memo<Option<ApiProblem>> {
    Memo::new(move |_| queue.with(|queue| queue.seed_problem.clone()))
}

pub(crate) fn order_stream_problem_memo(queue: RwSignal<OrderQueue>) -> Memo<Option<ApiProblem>> {
    Memo::new(move |_| queue.with(|queue| queue.stream_problem.clone()))
}

fn orders_for_run(run: Option<&ExecutionRun>, rows: &[OrderRecord]) -> Vec<OrderRecord> {
    let Some(run) = run else {
        return rows.to_vec();
    };
    let ids = run_order_ids(run);
    if ids.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .filter(|order| ids.contains(order.intent.id.as_str()))
        .cloned()
        .collect()
}

fn run_order_ids(run: &ExecutionRun) -> BTreeSet<&str> {
    run.long_leg
        .order_ids
        .iter()
        .chain(run.short_leg.order_ids.iter())
        .map(String::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{order, run_with_orders};
    use super::*;

    #[test]
    fn orders_for_run_filters_to_leg_order_ids() {
        let run = run_with_orders(&["long-1"], &["short-1"]);
        let rows = orders_for_run(
            Some(&run),
            &[order("noise"), order("short-1"), order("long-1")],
        );

        let ids = rows
            .iter()
            .map(|order| order.intent.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["short-1", "long-1"]);
    }

    #[test]
    fn orders_for_run_keeps_global_rows_before_run_exists() {
        let rows = orders_for_run(None, &[order("a"), order("b")]);

        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn all_orders_projection_keeps_rows_outside_latest_run() {
        Owner::new().with(|| {
            let queue = RwSignal::new(OrderQueue::default());
            queue.update(|queue| {
                queue.seed(Ok(super::super::fixtures::envelope(vec![
                    order("run-order"),
                    order("older-order"),
                ])));
            });

            let rows = all_orders_memo(queue).get_untracked();

            assert_eq!(rows.len(), 2);
        });
    }
}
