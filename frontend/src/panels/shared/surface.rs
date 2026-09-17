use leptos::prelude::*;

#[component]
pub(in crate::panels) fn Surface(
    title: &'static str,
    meta: &'static str,
    class_name: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <section class=format!("surface {}", class_name)>
            <header class="surface-header">
                <h2>{title}</h2>
                <span>{meta}</span>
            </header>
            <div class="surface-body">{children()}</div>
        </section>
    }
}
