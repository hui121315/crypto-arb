//! CROSSLINE Omni workstation shell.

use crate::components::ui::ToastContainer;
use crate::panels::workstation::OmniWorkstation;
use crate::state::context::provide_global;
use crate::state::watchlist_alerts::provide_watchlist_alert_runtime;
use crate::state::{persist_lang_on_change, provide_app_context, provide_toasts};
use leptos::prelude::*;

#[component]
pub fn App() -> impl IntoView {
    let ctx = provide_app_context();
    provide_toasts();
    provide_global();
    persist_lang_on_change(ctx);
    provide_watchlist_alert_runtime();

    view! {
        <OmniWorkstation/>
        <ToastContainer/>
    }
}
