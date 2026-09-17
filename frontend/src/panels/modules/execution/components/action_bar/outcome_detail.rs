use leptos::prelude::*;
use shared_types::HedgeConfirmResponse;

use super::super::super::data::{confirm_outcome_detail, confirm_outcome_summary};

pub(super) fn confirm_outcome(outcome: RwSignal<Option<HedgeConfirmResponse>>) -> impl IntoView {
    view! {
        <Show when=move || {
            outcome
                .get()
                .as_ref()
                .and_then(confirm_outcome_detail)
                .is_some()
        }>
            <details class="execution-disclosure confirm-outcome-detail">
                <summary>
                    {move || {
                        outcome
                            .get()
                            .as_ref()
                            .and_then(confirm_outcome_summary)
                            .unwrap_or_else(|| "提交结果".into())
                    }}
                </summary>
                <span>
                    {move || {
                        outcome
                            .get()
                            .as_ref()
                            .and_then(confirm_outcome_detail)
                            .unwrap_or_default()
                    }}
                </span>
            </details>
        </Show>
    }
}
