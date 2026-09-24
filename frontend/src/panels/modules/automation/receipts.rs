use crate::api::ws::{start_execution_stream_with_state, WsChannelState};
use crate::state::{context::use_global, load_state::LoadState};
use gloo_timers::callback::Interval;
use leptos::{prelude::*, task::spawn_local};
use shared_types::{AutomationExecutionReceipt, AutomationRuntimeStatus, ExecutionRunEvent};

#[derive(Clone, Copy)]
pub(super) struct ReceiptData {
    pub choice: RwSignal<String>,
    pub run_id: Memo<Option<String>>,
    pub options: Memo<Vec<(String, String)>>,
    pub state: RwSignal<LoadState<AutomationExecutionReceipt>>,
    pub reading: RwSignal<bool>,
    pub refresh: Callback<()>,
}

pub(super) fn use_receipts(status: RwSignal<LoadState<AutomationRuntimeStatus>>) -> ReceiptData {
    let choice = RwSignal::new(String::new());
    let options = Memo::new(move |_| {
        status.with(|state| {
            let mut rows = Vec::new();
            if let Some(status) = state.value() {
                for decision in status
                    .last_decision
                    .iter()
                    .chain(status.recent_decisions.iter())
                {
                    if let Some(id) = &decision.execution_run_id {
                        if !rows.iter().any(|(existing, _)| existing == id) {
                            rows.push((
                                id.clone(),
                                format!(
                                    "{} · {}",
                                    decision.symbol.as_deref().unwrap_or("运行"),
                                    id
                                ),
                            ));
                        }
                    }
                }
            }
            rows
        })
    });
    let run_id = Memo::new(move |_| {
        let choice = choice.get();
        if choice.is_empty() {
            options.with(|rows| rows.first().map(|(id, _)| id.clone()))
        } else {
            Some(choice)
        }
    });
    let state = RwSignal::new(LoadState::<AutomationExecutionReceipt>::Loading);
    let reading = RwSignal::new(false);
    let generation = RwSignal::new(0_u64);
    let last_confirmed = RwSignal::new(0_i64);
    let queued = RwSignal::new(Vec::<ExecutionRunEvent>::new());
    let client = use_global().client;
    let refresh = Callback::new(move |_| {
        let Some(id) = run_id.get_untracked() else {
            return;
        };
        if reading.get_untracked() {
            return;
        }
        reading.set(true);
        let generation_at_start = generation.get_untracked();
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .automation_execution_receipt(&id)
                .await
                .map_err(|error| error.problem);
            if generation.try_get_untracked() != Some(generation_at_start) {
                return;
            }
            match result {
                Ok(mut next) if next.run.run_id == id => {
                    if let Some(old) = state.get_untracked().value() {
                        merge_receipt(&mut next, old);
                    }
                    for event in queued.get_untracked() {
                        apply_event(&mut next, &event);
                    }
                    state.set(LoadState::Ready(next));
                    last_confirmed.set(super::super::timestamp::now_ms());
                }
                Ok(_) => state.update(|state| {
                    state.apply_result(Err(shared_types::ApiProblem::new(
                        "AUTOMATION_RECEIPT_MISMATCH",
                        "回执运行编号不匹配",
                    )))
                }),
                Err(problem) => state.update(|state| state.apply_result(Err(problem))),
            }
            queued.set(Vec::new());
            reading.set(false);
        });
    });
    Effect::new(move |_| {
        run_id.get();
        generation.update(|value| *value = value.wrapping_add(1));
        state.set(LoadState::Loading);
        queued.set(Vec::new());
        reading.set(false);
        last_confirmed.set(0);
        refresh.run(());
    });
    let channel = RwSignal::new(WsChannelState::new("execution"));
    let stream = start_execution_stream_with_state(
        channel,
        move |event| {
            if reading.try_get_untracked().is_none() {
                return;
            }
            let Some(id) = run_id.get_untracked() else {
                return;
            };
            let relevant = event
                .execution_run
                .as_ref()
                .is_some_and(|run| run.run_id == id)
                || event.close_run.as_ref().is_some_and(|close| {
                    close
                        .legs
                        .iter()
                        .filter_map(|leg| leg.pair_evidence.as_ref())
                        .any(|pair| pair.run_id == id)
                });
            if !relevant {
                return;
            }
            if state.with_untracked(|state| state.value().is_none()) {
                queued.update(|rows| {
                    rows.push(event);
                    if rows.len() > 64 {
                        rows.remove(0);
                    }
                });
                return;
            }
            let mut accepted = false;
            state.update(|state| {
                if let LoadState::Ready(value) | LoadState::Stale { value, .. } = state {
                    accepted = apply_event(value, &event);
                }
            });
            if accepted && state.with_untracked(|state| matches!(state, LoadState::Ready(_))) {
                last_confirmed.set(super::super::timestamp::now_ms());
            }
        },
        move |problem| {
            state.try_update(|state| state.apply_result(Err(problem)));
            last_confirmed.try_set(0);
        },
    );
    on_cleanup(move || stream.cancel());
    let interval = StoredValue::new_local(None::<Interval>);
    Effect::new(move |_| {
        interval.set_value(Some(Interval::new(5_000, move || {
            let Some(last) = last_confirmed.try_get_untracked() else {
                return;
            };
            if run_id.get_untracked().is_none() {
                return;
            }
            let age = super::super::timestamp::now_ms().saturating_sub(last);
            if age > 15_000 && last > 0 {
                state.update(|state| {
                    if matches!(state, LoadState::Ready(_)) {
                        state.apply_result(Err(shared_types::ApiProblem::new(
                            "AUTOMATION_RECEIPT_STALE",
                            "运行回执超过 15 秒未确认",
                        )));
                    }
                });
            }
            if age > 10_000 {
                refresh.run(());
            }
        })));
    });
    on_cleanup(move || {
        interval.update_value(|value| {
            value.take();
        })
    });
    ReceiptData {
        choice,
        run_id,
        options,
        state,
        reading,
        refresh,
    }
}

fn merge_receipt(next: &mut AutomationExecutionReceipt, old: &AutomationExecutionReceipt) {
    if next.run.run_id != old.run.run_id {
        return;
    }
    if next.run.updated_at_ms <= old.run.updated_at_ms {
        if next.run.long_leg.identity != old.run.long_leg.identity
            || next.run.short_leg.identity != old.run.short_leg.identity
        {
            next.mode = old.mode;
        }
        next.run = old.run.clone();
    }
    for close in &old.close_runs {
        merge_close(next, close);
    }
    next.close_run_total = next.close_run_total.max(old.close_run_total);
}

fn apply_event(receipt: &mut AutomationExecutionReceipt, event: &ExecutionRunEvent) -> bool {
    let mut accepted = false;
    if let Some(run) = &event.execution_run {
        if run.run_id == receipt.run.run_id
            && run.ticket_id == receipt.run.ticket_id
            && run.opportunity_id == receipt.run.opportunity_id
            && run.updated_at_ms >= receipt.run.updated_at_ms
        {
            if run.long_leg.identity != receipt.run.long_leg.identity
                || run.short_leg.identity != receipt.run.short_leg.identity
            {
                receipt.mode = None;
            }
            receipt.run = run.clone();
            accepted = true;
        }
    }
    if let Some(close) = &event.close_run {
        accepted |= merge_close(receipt, close);
    }
    accepted
}

fn merge_close(receipt: &mut AutomationExecutionReceipt, next: &shared_types::CloseRun) -> bool {
    if !AutomationExecutionReceipt::matches_close(&receipt.run, next) {
        return false;
    }
    if let Some(current) = receipt.close_runs.iter_mut().find(|run| run.id == next.id) {
        if next.updated_at_ms < current.updated_at_ms {
            return false;
        }
        *current = next.clone();
    } else {
        receipt.close_runs.push(next.clone());
        receipt.close_run_total = receipt.close_run_total.max(receipt.close_runs.len());
    }
    receipt.close_runs.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    receipt.close_runs.truncate(32);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn receipt() -> AutomationExecutionReceipt {
        let leg = |role| {
            json!({"role":role,"exchange":"fixture","symbol":"SOL","orderIds":[],
            "state":"accepted","targetQuantity":1.0,"targetNotionalUsd":10.0,"finalitySource":"adapter_ack"})
        };
        serde_json::from_value(json!({"run":{"runId":"run-a","ticketId":"ticket-a","opportunityId":"opp-a",
            "state":"second_leg_submitted","longLeg":leg("long"),"shortLeg":leg("short"),"netExposureUsd":0.0,
            "statusReason":"fixture","createdAtMs":1,"updatedAtMs":2},"closeRuns":[],"closeRunTotal":0,"observedAtMs":2})).unwrap()
    }

    #[test]
    fn receipt_rejects_other_bindings_and_older_versions() {
        let mut current = receipt();
        let mut event = ExecutionRunEvent {
            event: "execution_run_updated".into(),
            execution_run: Some(current.run.clone()),
            close_run: None,
            timestamp_ms: 9,
        };
        event.execution_run.as_mut().unwrap().ticket_id = "wrong-ticket".into();
        assert!(!apply_event(&mut current, &event));
        event.execution_run.as_mut().unwrap().ticket_id = "ticket-a".into();
        event.execution_run.as_mut().unwrap().updated_at_ms = 1;
        assert!(!apply_event(&mut current, &event));
        event.execution_run.as_mut().unwrap().updated_at_ms = 3;
        event.execution_run.as_mut().unwrap().net_exposure_usd = 4.0;
        assert!(apply_event(&mut current, &event));
        let mut late_http = receipt();
        merge_receipt(&mut late_http, &current);
        assert_eq!(late_http.run.net_exposure_usd, 4.0);
        assert_eq!(late_http.run.updated_at_ms, 3);
        let mut same_millisecond = receipt();
        same_millisecond.run.updated_at_ms = 3;
        merge_receipt(&mut same_millisecond, &current);
        assert_eq!(same_millisecond.run.net_exposure_usd, 4.0);
    }

    #[test]
    fn legacy_execution_frames_still_decode_without_close_payload() {
        let event: ExecutionRunEvent = serde_json::from_value(
            json!({"event":"execution_run_updated","executionRun":receipt().run,"timestampMs":2}),
        )
        .unwrap();
        assert!(event.close_run.is_none());
        assert!(event.execution_run.is_some());
    }
}
