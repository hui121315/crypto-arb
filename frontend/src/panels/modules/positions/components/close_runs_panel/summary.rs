//! `CloseRun` 面板的筛选摘要与确认输入。

use leptos::prelude::*;
use shared_types::{
    CloseRun, CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE,
    CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
};

use super::super::section_state::SectionData;
use super::derive::{close_run_rows, close_run_status_text};
use crate::panels::modules::positions::data::CloseRunCompensationAction;

#[derive(Clone)]
pub(super) struct ManualTerminalDraft {
    pub(super) phrase: String,
    pub(super) reason: String,
    pub(super) evidence: String,
    pub(super) cost_usd: String,
}

pub(in crate::panels::modules::positions) fn close_runs_panel(
    runs: Memo<SectionData<Vec<CloseRun>>>,
    action: CloseRunCompensationAction,
) -> impl IntoView {
    let phrase = RwSignal::new(String::new());
    let manual_phrase = RwSignal::new(String::new());
    let manual_reason = RwSignal::new(String::new());
    let manual_evidence = RwSignal::new(String::new());
    let manual_cost = RwSignal::new(String::new());
    let rows = Memo::new(move |_| close_run_rows(runs.get()));

    view! {
        <div class="close-runs-panel">
            <div class="close-runs-tools">
                <input
                    type="text"
                    autocomplete="off"
                    aria-label="补偿确认短语"
                    placeholder=CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE
                    value=move || phrase.get()
                    on:input=move |ev| phrase.set(event_target_value(&ev))
                />
                <span>{move || close_run_status_text(&rows.get())}</span>
                <input
                    type="text"
                    autocomplete="off"
                    aria-label="人工终结确认短语"
                    placeholder=CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE
                    value=move || manual_phrase.get()
                    on:input=move |ev| manual_phrase.set(event_target_value(&ev))
                />
                <input
                    type="text"
                    placeholder="人工终结原因"
                    value=move || manual_reason.get()
                    on:input=move |ev| manual_reason.set(event_target_value(&ev))
                />
                <input
                    type="text"
                    placeholder="证据编号"
                    value=move || manual_evidence.get()
                    on:input=move |ev| manual_evidence.set(event_target_value(&ev))
                />
                <input
                    type="number"
                    min="0"
                    step="0.01"
                    placeholder="人工成本 USD"
                    value=move || manual_cost.get()
                    on:input=move |ev| manual_cost.set(event_target_value(&ev))
                />
            </div>
            <div class="table-wrap close-runs-table-wrap">
                <table class="clean-table close-runs-table">
                    <thead>
                        <tr>
                            <th>"状态"</th>
                            <th>"CloseRun"</th>
                            <th>"裸露"</th>
                            <th>"成本"</th>
                            <th>"候选"</th>
                            <th>"操作"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                            let phrase_value = phrase.get();
                            let manual = ManualTerminalDraft {
                                phrase: manual_phrase.get(),
                                reason: manual_reason.get(),
                                evidence: manual_evidence.get(),
                                cost_usd: manual_cost.get(),
                            };
                            super::render_close_run_body(rows.get(), action, &phrase_value, &manual)
                        }}
                    </tbody>
                </table>
            </div>
            <em class="positions-action-message table-message">
                {move || action.state.get().message("")}
            </em>
        </div>
    }
}
