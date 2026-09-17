use leptos::prelude::*;
use shared_types::HedgeConfirmContext;

use super::super::super::data::confirm_context_detail;

pub(super) fn confirm_context(context: RwSignal<Option<HedgeConfirmContext>>) -> impl IntoView {
    view! {
        <Show when=move || {
            context
                .get()
                .as_ref()
                .and_then(confirm_context_detail)
                .is_some()
        }>
            <details class="execution-disclosure confirm-context-detail">
                <summary>"执行标识与腿级上下文"</summary>
                <span>
                    {move || {
                        context
                            .get()
                            .as_ref()
                            .and_then(confirm_context_detail)
                            .unwrap_or_default()
                    }}
                </span>
            </details>
        </Show>
    }
}
