use leptos::prelude::*;

pub(in crate::panels::modules::execution) fn mini_stat<V>(
    label: &'static str,
    value: V,
    tone: &'static str,
) -> impl IntoView
where
    V: IntoView + 'static,
{
    view! {
        <div class=format!("ticket-stat {tone}")>
            <span>{label}</span>
            <strong>{value}</strong>
        </div>
    }
}

pub(in crate::panels::modules::execution) fn read_only_field<V>(
    label: &'static str,
    value: V,
) -> impl IntoView
where
    V: Fn() -> String + Copy + Send + 'static,
{
    view! {
        <label class="leg-field readonly">
            <span>{label}</span>
            <strong>{move || value()}</strong>
        </label>
    }
}
