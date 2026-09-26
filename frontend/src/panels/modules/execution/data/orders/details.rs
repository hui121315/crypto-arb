use super::OrderQueue;
use crate::panels::modules::execution_orders::{leg_contains_order, run_order_ids};
use crate::panels::modules::execution::data::connection::ExecutionConnection;
use crate::state::read_scope::bounded_read;
use futures::{
    future::{AbortHandle, Abortable},
    stream, StreamExt,
};
use leptos::{prelude::*, task::spawn_local};
use shared_types::{ApiProblem, ExecutionRun};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(crate) struct OrderDetails {
    pub reading: RwSignal<bool>,
    pub problem: Memo<Option<ApiProblem>>,
    pub refresh: Callback<()>,
}

pub(crate) fn use_run_order_details(
    run: RwSignal<Option<ExecutionRun>>,
    queue: RwSignal<OrderQueue>,
    seed_ready: Memo<bool>,
) -> OrderDetails {
    let connection = expect_context::<ExecutionConnection>();
    let reading = RwSignal::new(false);
    let errors = RwSignal::new(BTreeMap::<String, ApiProblem>::new());
    let retry = RwSignal::new(0_u64);
    let generation = RwSignal::new(0_u64);
    let active = StoredValue::new(None::<AbortHandle>);
    let identity = Memo::new(move |_| {
        run.with(|run| {
            run.as_ref().map(|run| {
                (
                    run.run_id.clone(),
                    run_order_ids(run, &[])
                        .into_iter()
                        .collect::<Vec<_>>(),
                )
            })
        })
    });
    on_cleanup(move || {
        active.update_value(|active| {
            if let Some(abort) = active.take() {
                abort.abort();
            }
        })
    });
    let problem = Memo::new(move |_| {
        errors.with(|errors| {
            queue.with(|queue| {
                let expected = run.with(|run| run.as_ref().map(|run| run_order_ids(run, &queue.rows))).unwrap_or_default();
                errors
                    .iter()
                    .find(|(id, _)| expected.contains(*id) && queue.order(id).is_none())
                    .map(|(_, problem)| problem.clone())
            })
        })
    });
    Effect::new(move |_| {
        let identity = identity.get();
        let ready = seed_ready.get();
        retry.get();
        let available = connection.available();
        active.update_value(|active| {
            if let Some(abort) = active.take() {
                abort.abort();
            }
        });
        generation.update(|value| *value = value.wrapping_add(1));
        let version = generation.get_untracked();
        reading.set(false);
        errors.set(BTreeMap::new());
        let Some(_) = identity else {
            return;
        };
        if !ready || !available {
            return;
        }
        let missing = queue.with_untracked(|queue| {
            run.with_untracked(|run| run.as_ref().map(|run| run_order_ids(run, &queue.rows)))
                .unwrap_or_default().into_iter()
                .filter(|id| queue.order(id).is_none())
                .collect::<Vec<_>>()
        });
        if missing.is_empty() {
            return;
        }
        let expected_run = run.get_untracked();
        reading.set(true);
        let client = connection.client().cancelable_reads();
        let (abort, registration) = AbortHandle::new_pair();
        active.set_value(Some(abort));
        spawn_local(async move {
            // Only missing IDs in this run, two reads at a time, one bound for the entire batch.
            let batch_ids = missing.clone();
            let fetch = async {
                let mut reads = stream::iter(batch_ids.into_iter().map(|id| {
                    let client = client.clone();
                    async move {
                        let result = client.trading_order(&id).await;
                        (id, result)
                    }
                }))
                .buffer_unordered(2);
                while let Some((id, result)) = reads.next().await {
                    if generation.try_get_untracked() != Some(version) || !connection.current() {
                        return Ok(());
                    }
                    match result {
                        Ok(record) if record.intent.id == id && expected_run.as_ref().is_some_and(|run|
                            leg_contains_order(&run.long_leg, &record) || leg_contains_order(&run.short_leg, &record)) => {
                            queue.update(|queue| queue.apply_receipt(record))
                        }
                        Ok(_) => errors.update(|errors| {
                            errors.insert(
                                id,
                                ApiProblem::new(
                                    "ORDER_DETAIL_ID_MISMATCH",
                                    "订单明细与当前交易不匹配，未采用该记录",
                                )
                                .with_source("execution.order_details"),
                            );
                        }),
                        Err(error) => errors.update(|errors| {
                            errors.insert(id, error.problem);
                        }),
                    }
                }
                Ok(())
            };
            let Ok(result) = Abortable::new(bounded_read(fetch), registration).await else {
                return;
            };
            if generation.try_get_untracked() != Some(version) || !connection.current() {
                return;
            }
            active.update_value(|active| {
                active.take();
            });
            reading.set(false);
            if let Err(mut problem) = result {
                problem.code = "ORDER_DETAILS_READ_TIMEOUT".into();
                problem.message =
                    "订单明细读取超过 15 秒，已停止本次查询；不代表订单未提交或成交失败".into();
                problem.source = Some("execution.order_details".into());
                errors.update(|errors| {
                    for id in missing {
                        if queue.with_untracked(|queue| queue.order(&id).is_none()) {
                            errors.entry(id).or_insert_with(|| problem.clone());
                        }
                    }
                });
            }
        });
    });
    OrderDetails {
        reading,
        problem,
        refresh: Callback::new(move |_| retry.update(|value| *value = value.wrapping_add(1))),
    }
}
