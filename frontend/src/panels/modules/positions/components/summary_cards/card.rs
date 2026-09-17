use leptos::prelude::*;

use super::Tone;

pub(super) fn card(label: &'static str, value: String, sub: String, tone: Tone) -> AnyView {
    view! {
        <div class=tone.class()>
            <span>{label}</span>
            <strong class="num">{value}</strong>
            <em>{sub}</em>
        </div>
    }
    .into_any()
}

pub(super) fn state_card(message: String) -> AnyView {
    view! {
        <div class="summary-card neutral summary-state-card">
            <span>"概览状态"</span>
            <strong>{message}</strong>
            <em>"等待后端 portfolio snapshot 恢复"</em>
        </div>
    }
    .into_any()
}
