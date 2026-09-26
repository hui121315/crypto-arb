use crate::panels::modules::settings::data::{create_webhook_data, WebhookData};
use leptos::prelude::*;
use shared_types::{WebhookConfig, WebhookEventKind, WebhookProvider};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct WebhookRuntime {
    pub(super) data: WebhookData,
    pub(super) provider: RwSignal<String>,
    pub(super) timeout: RwSignal<String>,
    pub(super) attempts: RwSignal<String>,
    pub(super) backoff: RwSignal<String>,
    pub(super) capacity: RwSignal<String>,
    pub(super) event_kinds: RwSignal<Vec<WebhookEventKind>>,
    pub(super) hydrated: RwSignal<bool>,
    pub(super) credentials_cleared: RwSignal<bool>,
    dirty: RwSignal<bool>,
    baseline: RwSignal<Option<WebhookConfig>>,
}

pub(in crate::panels::modules::settings) fn create_webhook_runtime() -> WebhookRuntime {
    let runtime = WebhookRuntime {
        data: create_webhook_data(),
        provider: RwSignal::new("generic".into()),
        timeout: RwSignal::new("15000".into()),
        attempts: RwSignal::new("3".into()),
        backoff: RwSignal::new("500".into()),
        capacity: RwSignal::new("128".into()),
        event_kinds: RwSignal::new(Vec::new()),
        hydrated: RwSignal::new(false),
        credentials_cleared: RwSignal::new(false),
        dirty: RwSignal::new(false),
        baseline: RwSignal::new(None),
    };
    Effect::new(move |_| {
        let state = runtime.data.state.get();
        if !runtime.data.pending.get()
            && (!runtime.hydrated.get() || (!runtime.dirty.get() && !runtime.data.journal.locked()))
        {
            if let Some(status) = state.value() {
                if runtime.baseline.get_untracked().as_ref() != Some(&status.config) {
                    runtime.apply(&status.config);
                }
            }
        }
    });
    Effect::new(move |_| {
        if let Some(saved) = runtime.data.saved.get() {
            runtime.apply(&saved.config);
            runtime.credentials_cleared.set(false);
        }
    });
    Effect::new(move |_| {
        runtime.data.recovered.track();
        runtime.data.journal.connection.track();
        runtime.dirty.set(false);
        runtime.baseline.set(None);
        runtime.hydrated.set(false);
    });
    runtime
}

impl WebhookRuntime {
    pub(super) fn edit(self) {
        if self.data.pending.get_untracked() || self.data.journal.locked() {
            return;
        }
        self.dirty.set(true);
        self.data.message.set(None);
        self.data.action_problem.set(None);
    }

    fn apply(self, config: &WebhookConfig) {
        self.provider.set(
            match config.provider {
                WebhookProvider::Generic => "generic",
                WebhookProvider::Bark => "bark",
            }
            .into(),
        );
        self.timeout.set(config.timeout_ms.to_string());
        self.attempts.set(config.max_attempts.to_string());
        self.backoff.set(config.base_backoff_ms.to_string());
        self.capacity.set(config.queue_capacity.to_string());
        self.event_kinds.set(config.event_kinds.clone());
        self.baseline.set(Some(config.clone()));
        self.hydrated.set(true);
        if self.dirty.get_untracked() {
            self.dirty.set(false);
        }
    }
}
