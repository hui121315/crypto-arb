use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "connection_tests/account_rotation.rs"]
mod account_rotation;

#[test]
fn reconnect_rebuilds_time_sensitive_messages() {
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let factory: Arc<PrivateWsMessageFactory> = Arc::new(move || {
        let generation = factory_calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(vec![format!("auth-generation-{generation}")])
    });

    assert!(matches!(
        build_connect_messages(&factory),
        Ok(messages) if messages == ["auth-generation-1"]
    ));
    assert!(matches!(
        build_connect_messages(&factory),
        Ok(messages) if messages == ["auth-generation-2"]
    ));
}
