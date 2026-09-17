use leptos::prelude::*;

#[component]
pub fn ModuleHeader(#[prop(into)] title: String) -> impl IntoView {
    view! {
        <header class="module-header">
            <h1>{title}</h1>
        </header>
    }
}
