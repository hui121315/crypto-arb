use crate::api::{
    base::{normalize_api_auth_token, normalize_api_base},
    rest::ApiClient,
};
use crate::state::{module_runtime, AppContext};
use leptos::prelude::*;
use sha2::{Digest, Sha256};

/// Execution requests retain their original login, including after an A -> B -> A switch.
#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct ExecutionConnection {
    base: RwSignal<String>,
    token: RwSignal<String>,
    identity: StoredValue<String>,
    client: StoredValue<ApiClient>,
    active: RwSignal<bool>,
}

impl ExecutionConnection {
    pub(super) fn new() -> Self {
        let app = expect_context::<AppContext>();
        Self::from_signals(app.api_base, app.api_auth_token)
    }

    pub(super) fn from_signals(base: RwSignal<String>, token: RwSignal<String>) -> Self {
        let connection = Self {
            base,
            token,
            identity: StoredValue::new(identity(&base.get_untracked(), &token.get_untracked())),
            client: StoredValue::new(ApiClient::with_base_and_auth(
                &base.get_untracked(),
                &token.get_untracked(),
            )),
            active: RwSignal::new(true),
        };
        Effect::new(move |_| {
            base.track();
            token.track();
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

    pub(in crate::panels::modules::execution) fn current(self) -> bool {
        self.active.try_get_untracked() == Some(true)
            && self.identity.try_get_value().is_some_and(|expected| {
                self.base
                    .try_get_untracked()
                    .zip(self.token.try_get_untracked())
                    .is_some_and(|(base, token)| identity(&base, &token) == expected)
            })
    }

    pub(in crate::panels::modules::execution) fn available(self) -> bool {
        self.base.track();
        self.token.track();
        self.active.track();
        self.current()
    }

    pub(in crate::panels::modules::execution) fn client(self) -> ApiClient {
        self.client.get_value()
    }

    pub(in crate::panels::modules::execution) fn key(self, prefix: &str) -> String {
        format!("{prefix}:{}", self.identity.get_value())
    }

    pub(in crate::panels::modules::execution) fn read<T>(
        self,
        key: &str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Option<T> {
        self.current()
            .then(|| module_runtime::stored_choice(&self.key(key), parse))
            .flatten()
    }

    pub(in crate::panels::modules::execution) fn store(self, key: &str, value: &str) {
        if self.current() {
            module_runtime::store_choice(&self.key(key), value);
        }
    }

    pub(in crate::panels::modules::execution) fn clear(self, key: &str) {
        if self.current() {
            module_runtime::clear_choice(&self.key(key));
        }
    }
}

fn identity(base: &str, token: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(normalize_api_base(base));
    hash.update([0]);
    hash.update(normalize_api_auth_token(token));
    format!("{:x}", hash.finalize())
}
