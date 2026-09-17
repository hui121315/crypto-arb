use super::emit_value;
use crate::state::AppState;
use shared_types::{ExecutionRunState, WebhookEventKind};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ExecutionAlertCursor {
    state: ExecutionRunState,
    observed_at_ms: i64,
}

pub(super) fn baseline_execution_updates(
    state: &AppState,
) -> HashMap<String, ExecutionAlertCursor> {
    state
        .execution_runs()
        .iter()
        .filter_map(|entry| {
            let alert_state = webhook::execution_result_alert_state(entry.value())?;
            let event_id = webhook::execution_result_event_id(entry.value())?;
            if state.webhook().durable_outbox() && !state.webhook().event_known(&event_id) {
                return None;
            }
            Some((
                entry.run_id.clone(),
                ExecutionAlertCursor {
                    state: alert_state,
                    observed_at_ms: entry.updated_at_ms,
                },
            ))
        })
        .collect()
}

pub(super) async fn emit_execution_results(
    state: &AppState,
    cursor: &mut HashMap<String, ExecutionAlertCursor>,
) {
    let runs = state
        .execution_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect::<Vec<_>>();
    for run in runs {
        let Some(alert_state) = webhook::execution_result_alert_state(&run) else {
            continue;
        };
        if let Some(previous) = cursor.get_mut(&run.run_id) {
            if previous.state == alert_state {
                previous.observed_at_ms = previous.observed_at_ms.max(run.updated_at_ms);
                continue;
            }
        }
        let Some(event_id) = webhook::execution_result_event_id(&run) else {
            continue;
        };
        if emit_value(state, WebhookEventKind::ExecutionResult, event_id, &run).await {
            cursor.insert(
                run.run_id.clone(),
                ExecutionAlertCursor {
                    state: alert_state,
                    observed_at_ms: run.updated_at_ms,
                },
            );
        }
    }
    trim_cursor(cursor);
}

fn trim_cursor(values: &mut HashMap<String, ExecutionAlertCursor>) {
    if values.len() <= 256 {
        return;
    }
    let mut rows = values
        .iter()
        .map(|(id, cursor)| (id.clone(), cursor.observed_at_ms))
        .collect::<Vec<_>>();
    rows.sort_by_key(|(_, observed_at_ms)| *observed_at_ms);
    for (id, _) in rows.into_iter().take(values.len().saturating_sub(256)) {
        values.remove(&id);
    }
}
