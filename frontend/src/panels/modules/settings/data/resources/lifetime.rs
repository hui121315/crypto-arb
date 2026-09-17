use leptos::prelude::on_cleanup;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct SettingsRequestLifetime(Arc<AtomicBool>);

impl SettingsRequestLifetime {
    fn active() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub(super) fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn expire(&self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) fn settings_request_lifetime() -> SettingsRequestLifetime {
    let lifetime = SettingsRequestLifetime::active();
    let cleanup = lifetime.clone();
    on_cleanup(move || cleanup.expire());
    lifetime
}

#[cfg(test)]
mod tests {
    use super::SettingsRequestLifetime;

    #[test]
    fn expired_lifetime_rejects_late_resource_updates() {
        let lifetime = SettingsRequestLifetime::active();
        assert!(lifetime.is_active());

        lifetime.expire();

        assert!(!lifetime.is_active());
    }
}
