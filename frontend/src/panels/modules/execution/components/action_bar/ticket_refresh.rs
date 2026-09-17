use crate::state::action_state::ActionState;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use shared_types::ExecutionRun;

use super::super::super::data::ExecutionPreview;
use super::labels::run_blocks_new_submission;

const PREVIEW_RETRY_DELAY_MS: i64 = 3_000;
const MAX_AUTO_PREVIEW_RETRIES: u8 = 2;

pub(super) fn use_ticket_refresh(
    preview: Memo<ExecutionPreview>,
    execution_run: RwSignal<Option<ExecutionRun>>,
    action_state: RwSignal<ActionState>,
    preview_refresh_nonce: RwSignal<u64>,
) -> RwSignal<i64> {
    let ticket_clock_ms = RwSignal::new(crate::state::polling::now_ms() as i64);
    let ticket_clock = StoredValue::new_local(Some(Interval::new(1_000, move || {
        ticket_clock_ms.set(crate::state::polling::now_ms() as i64);
    })));
    on_cleanup(move || {
        ticket_clock.update_value(|slot| {
            if let Some(interval) = slot.take() {
                interval.cancel();
            }
        });
    });
    let last_refreshed_ticket = RwSignal::new(None::<String>);
    let retry_scope = RwSignal::new((String::new(), 0_u8));
    let retry_ticket = RwSignal::new(None::<(String, i64)>);
    Effect::new(move |_| {
        let now_ms = ticket_clock_ms.get();
        let current_preview = preview.get();
        let current_run = execution_run.get();
        if action_state.get().is_pending()
            || run_blocks_new_submission(current_run.as_ref(), &current_preview)
        {
            return;
        }
        if maybe_retry_market_preview(
            &current_preview,
            now_ms,
            retry_scope,
            retry_ticket,
            preview_refresh_nonce,
        ) {
            return;
        }
        if !current_preview.ticket_needs_refresh_at(now_ms) {
            return;
        }
        let Some(ticket_id) = current_preview.ticket_id.as_deref() else {
            return;
        };
        if last_refreshed_ticket.get_untracked().as_deref() == Some(ticket_id) {
            return;
        }
        last_refreshed_ticket.set(Some(ticket_id.to_owned()));
        preview_refresh_nonce.update(|value| *value = value.wrapping_add(1));
    });
    ticket_clock_ms
}

fn maybe_retry_market_preview(
    preview: &ExecutionPreview,
    now_ms: i64,
    retry_scope: RwSignal<(String, u8)>,
    retry_ticket: RwSignal<Option<(String, i64)>>,
    preview_refresh_nonce: RwSignal<u64>,
) -> bool {
    if !preview.has_retryable_market_blocker() {
        retry_scope.set((String::new(), 0));
        retry_ticket.set(None);
        return false;
    }
    let Some(ticket_id) = preview.ticket_id.as_deref() else {
        return false;
    };
    let (scope, attempts) = retry_scope.get_untracked();
    if scope != preview.opportunity_id {
        retry_scope.set((preview.opportunity_id.clone(), 0));
        retry_ticket.set(Some((ticket_id.to_owned(), now_ms)));
        return true;
    }
    if attempts >= MAX_AUTO_PREVIEW_RETRIES {
        return false;
    }
    let Some((seen_ticket, seen_at_ms)) = retry_ticket.get_untracked() else {
        retry_ticket.set(Some((ticket_id.to_owned(), now_ms)));
        return true;
    };
    if seen_ticket != ticket_id {
        retry_ticket.set(Some((ticket_id.to_owned(), now_ms)));
        return true;
    }
    if now_ms.saturating_sub(seen_at_ms) < PREVIEW_RETRY_DELAY_MS {
        return true;
    }
    retry_scope.set((scope, attempts.saturating_add(1)));
    retry_ticket.set(None);
    preview_refresh_nonce.update(|value| *value = value.wrapping_add(1));
    true
}
