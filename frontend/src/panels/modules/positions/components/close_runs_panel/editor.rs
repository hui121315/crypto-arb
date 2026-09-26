use leptos::prelude::*;
use shared_types::{
    CloseRun, CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE,
    CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
};

use super::derive::*;
use crate::panels::modules::positions::data::{
    compensation_key, CloseRunCompensationAction, CloseRunCompensationCancelInput,
    CloseRunCompensationInput, CloseRunManualTerminalInput,
};

pub(super) fn incident_editor(
    record: Memo<Option<CloseRun>>,
    fresh: Memo<bool>,
    action: CloseRunCompensationAction,
) -> impl IntoView {
    let phrase = RwSignal::new(String::new());
    let manual_phrase = RwSignal::new(String::new());
    let reason = RwSignal::new(String::new());
    let evidence = RwSignal::new(String::new());
    let cost = RwSignal::new(String::new());
    let revision = Memo::new(move |_| record.get().map(|run| confirmation_revision(&run)));
    // A changed execution plan needs a new confirmation; ordinary WS updates
    // retain the draft and keyboard focus.
    Effect::new(move |_| {
        revision.get();
        phrase.set(String::new());
        manual_phrase.set(String::new());
    });
    let blocked = Memo::new(move |_| !fresh.get() || action.locked());
    let cancel_blocked = Memo::new(move |_| !fresh.get() || action.cancel_locked());
    view! {
        <Show when=move || record.get().is_some_and(|run| !submittable_compensation_candidates(&run).is_empty())>
            <div class="close-incident-form">
                <label>"补偿确认短语"
                    <input type="text" autocomplete="off" spellcheck="false"
                        placeholder=CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE
                        prop:value=move || phrase.get()
                        on:input=move |ev| phrase.set(event_target_value(&ev)) />
                </label>
                <div class="close-run-actions">
                    {move || record.get().map(|run| submittable_compensation_candidates(&run).into_iter().map(|(index, candidate)| {
                        let key = compensation_key(&run, index);
                        let retry = run.status == shared_types::CloseRunStatus::CompensationFailed;
                        view! {
                        <button type="button" class="danger-action"
                            disabled=move || blocked.get() || !record.get().is_some_and(|run| can_submit_compensation(&run, &phrase.get()))
                            on:click=move |_| {
                                if blocked.get_untracked() { return; }
                                if let Some(run) = record.get_untracked() {
                                    if can_submit_compensation(&run, &phrase.get_untracked()) && submittable_compensation_candidates(&run).iter().any(|(current, _)| *current == index) {
                                        action.submit.run(CloseRunCompensationInput { run, candidate_index: index, confirmation_phrase: phrase.get_untracked() });
                                    }
                                }
                            }>
                            {move || if action.active_key.get().as_deref() == Some(key.as_str()) { "处理中".to_owned() } else { format!("{}{} #{}", if retry { "重试" } else { "" }, compensation_button_label(&candidate), index + 1) }}
                        </button>
                    }}).collect_view())}
                </div>
            </div>
        </Show>
        <div class="close-run-actions">
            {move || record.get().map(|run| cancellable_compensation_attempts(&run).into_iter().filter_map(|attempt| {
                let order_id = compensation_cancel_order_id(&attempt)?;
                let disabled_id = order_id.clone();
                Some(view! {
                    <button type="button" class="danger-action" title=compensation_cancel_title(&attempt)
                        disabled=move || cancel_blocked.get() || action.cancel_finished(&disabled_id)
                        on:click=move |_| {
                            if cancel_blocked.get_untracked() { return; }
                            if let Some(run) = record.get_untracked() {
                                action.cancel.run(CloseRunCompensationCancelInput { run, order_id: order_id.clone() });
                            }
                        }>{compensation_cancel_button_label(&attempt)}</button>
                })
            }).collect_view())}
        </div>
        <Show when=move || record.get().is_some_and(|run| has_manual_terminal_action(&run))>
            <div class="close-incident-form close-incident-manual">
                <p>"人工终结仅记录处理结果，不会提交平仓单。"</p>
                <label>"处理原因"<input type="text" prop:value=move || reason.get() on:input=move |ev| reason.set(event_target_value(&ev)) /></label>
                <label>"数据依据编号"<input type="text" prop:value=move || evidence.get() on:input=move |ev| evidence.set(event_target_value(&ev)) /></label>
                <label>"人工成本 USD（选填）"<input type="number" min="0" step="0.01" prop:value=move || cost.get() on:input=move |ev| cost.set(event_target_value(&ev)) /></label>
                <label>"人工终结确认短语"
                    <input type="text" autocomplete="off" spellcheck="false" placeholder=CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE
                        prop:value=move || manual_phrase.get() on:input=move |ev| manual_phrase.set(event_target_value(&ev)) />
                </label>
                <button type="button" class="danger-action"
                    disabled=move || blocked.get() || !manual_cost_valid(&cost.get()) || !record.get().is_some_and(|run| can_submit_manual_terminal(&run, &manual_phrase.get(), &reason.get()))
                    on:click=move |_| {
                        if blocked.get_untracked() || !manual_cost_valid(&cost.get_untracked()) { return; }
                        if let Some(run) = record.get_untracked() {
                            if can_submit_manual_terminal(&run, &manual_phrase.get_untracked(), &reason.get_untracked()) {
                                action.manual_terminal.run(CloseRunManualTerminalInput { run, confirmation_phrase: manual_phrase.get_untracked(), reason: reason.get_untracked(), evidence: evidence.get_untracked(), manual_handling_cost_usd: cost.get_untracked() });
                            }
                        }
                    }>"记录人工终结"</button>
            </div>
        </Show>
    }
}

fn manual_cost_valid(value: &str) -> bool {
    value.trim().is_empty()
        || value
            .trim()
            .parse::<f64>()
            .is_ok_and(|n| n.is_finite() && n >= 0.0)
}

fn confirmation_revision(run: &CloseRun) -> String {
    format!(
        "{:?}:{}:{:?}",
        run.status,
        run.snapshot_version,
        run.unwind_plan
            .as_ref()
            .map(|plan| &plan.compensation_candidates)
    )
}

#[cfg(test)]
mod tests {
    use super::manual_cost_valid;
    #[test]
    fn manual_cost_allows_optional_decimal_but_rejects_invalid_values() {
        for valid in ["", "0", "0.25"] {
            assert!(manual_cost_valid(valid));
        }
        for invalid in ["-1", "NaN", "inf", "bad"] {
            assert!(!manual_cost_valid(invalid));
        }
    }
}
