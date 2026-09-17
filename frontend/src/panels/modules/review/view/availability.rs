use leptos::prelude::*;

use super::ReviewTab;
use crate::panels::modules::review::components::ReviewSectionRows;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReviewTaskAvailability {
    tab: ReviewTab,
    rows: usize,
    loaded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReviewResultSuggestion {
    tab: ReviewTab,
    rows: usize,
}

pub(super) fn review_task_availability<T>(
    tab: ReviewTab,
    section: &ReviewSectionRows<T>,
) -> ReviewTaskAvailability {
    ReviewTaskAvailability {
        tab,
        rows: section.rows.len(),
        loaded: section.has_loaded_context(),
    }
}

pub(super) fn review_result_suggestion(
    active: ReviewTab,
    tasks: [ReviewTaskAvailability; 4],
) -> Option<ReviewResultSuggestion> {
    let current = tasks.iter().find(|task| task.tab == active)?;
    if !current.loaded || current.rows > 0 {
        return None;
    }
    tasks
        .into_iter()
        .find(|task| task.tab != active && task.loaded && task.rows > 0)
        .map(|task| ReviewResultSuggestion {
            tab: task.tab,
            rows: task.rows,
        })
}

pub(super) fn review_available_result(
    suggestion: Memo<Option<ReviewResultSuggestion>>,
    on_open: Callback<ReviewTab>,
) -> impl IntoView {
    view! {
        {move || suggestion.get().map(|suggestion| {
            let tab = suggestion.tab;
            view! {
                <aside class="review-available-result" aria-live="polite">
                    <strong>{format!(
                        "{} · {} {}可查看",
                        tab.label(),
                        suggestion.rows,
                        tab.unit(),
                    )}</strong>
                    <button
                        type="button"
                        class="row-action review-available-action"
                        on:click=move |_| on_open.run(tab)
                    >
                        {format!("查看{}", tab.label())}
                    </button>
                </aside>
            }
        })}
    }
}
