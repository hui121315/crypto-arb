use crate::api::base::{normalize_api_auth_token, normalize_api_base};
use crate::state::AppContext;
use leptos::prelude::*;

/// Retained history belongs to the original login, even after an A -> B -> A switch.
#[derive(Clone, Copy)]
pub(in crate::panels::modules::review) struct ReviewConnection {
    app: AppContext,
    identity: StoredValue<(String, String)>,
    active: RwSignal<bool>,
}

impl ReviewConnection {
    pub(super) fn new() -> Self {
        let app = expect_context::<AppContext>();
        let base = normalize_api_base(&app.api_base.get_untracked());
        let token = normalize_api_auth_token(&app.api_auth_token.get_untracked());
        let connection = Self {
            app,
            identity: StoredValue::new((base, token)),
            active: RwSignal::new(true),
        };
        Effect::new(move |_| {
            app.api_base.track();
            app.api_auth_token.track();
            if !connection.current() {
                connection.active.set(false);
            }
        });
        #[cfg(target_arch = "wasm32")]
        {
            let listener = window_event_listener_untyped("beforeunload", move |_| {
                connection.active.set(false);
            });
            on_cleanup(move || listener.remove());
        }
        connection
    }

    pub(in crate::panels::modules::review) fn current(self) -> bool {
        self.active.try_get_untracked() == Some(true)
            && self.identity.try_get_value().is_some_and(|expected| {
                self.app.api_base.try_get_untracked()
                    .zip(self.app.api_auth_token.try_get_untracked())
                    .is_some_and(|(base, token)| {
                        (normalize_api_base(&base), normalize_api_auth_token(&token)) == expected
                    })
            })
    }

    pub(in crate::panels::modules::review) fn available(self) -> bool {
        self.app.api_base.track();
        self.app.api_auth_token.track();
        self.active.track();
        self.current()
    }

}
