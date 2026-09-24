use leptos::prelude::*;
use shared_types::CloseRun;

use super::{
    derive::{close_run_rows, close_run_status_text},
    CloseRunCompensationAction, SectionData,
};

pub(in crate::panels::modules::positions) fn close_runs_panel(
    runs: Memo<SectionData<Vec<CloseRun>>>,
    action: CloseRunCompensationAction,
) -> impl IntoView {
    let rows = Memo::new(move |_| close_run_rows(runs.get()));
    let fresh = Memo::new(move |_| rows.get().has_fresh_value());
    view! {
        <section class="close-runs-panel" aria-label="待处理平仓">
            <header><h3>"待处理平仓"</h3><span>{move || close_run_status_text(&rows.get())}</span></header>
            {move || rows.get().status.stale_note("记录已过期，暂停提交").map(|note| view! {
                <p class="position-history-warning" role="status">{note}</p>
            })}
            <For
                each=move || { rows.get().value.into_iter().map(|run| run.id).collect::<Vec<_>>() }
                key=|id| id.clone()
                children=move |id| {
                    let record = Memo::new(move |_| rows.with(|rows| rows.value.iter().find(|run| run.id == id).cloned()));
                    super::close_run_record(record, fresh, action)
                }
            />
        </section>
    }
}
