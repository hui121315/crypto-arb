use super::*;

#[test]
fn failed_subscribe_releases_the_symbol_for_one_bounded_retry() {
    let subscriptions = DashMap::new();
    let symbol = "BTC_USDT".to_owned();
    subscriptions.insert(
        symbol.clone(),
        SubscriptionState {
            last_touched_ms: 1,
            sent_on_current_connection: true,
        },
    );

    release_subscriptions(&subscriptions, std::slice::from_ref(&symbol), "subscribe");
    let mut state = subscriptions.get_mut(&symbol).expect("subscription exists");
    assert!(claim_subscription(&mut state));
    assert!(!claim_subscription(&mut state));
}

#[test]
fn book_bootstrap_claim_is_bounded_until_refresh_deadline() {
    let claims = DashMap::new();
    assert!(claim_refresh(
        &claims,
        "IONQ_USDT",
        1_000,
        BOOK_SNAPSHOT_REFRESH_MS
    ));
    assert!(!claim_refresh(
        &claims,
        "IONQ_USDT",
        1_000 + BOOK_SNAPSHOT_REFRESH_MS - 1,
        BOOK_SNAPSHOT_REFRESH_MS
    ));
    assert!(claim_refresh(
        &claims,
        "IONQ_USDT",
        1_000 + BOOK_SNAPSHOT_REFRESH_MS,
        BOOK_SNAPSHOT_REFRESH_MS
    ));
}

#[test]
fn snapshot_reconnect_claim_has_a_global_cooldown() {
    let last_reconnect_ms = AtomicI64::new(0);
    assert!(claim_snapshot_reconnect(&last_reconnect_ms, 1_000));
    assert!(!claim_snapshot_reconnect(
        &last_reconnect_ms,
        1_000 + SNAPSHOT_RECONNECT_COOLDOWN_MS - 1
    ));
    assert!(claim_snapshot_reconnect(
        &last_reconnect_ms,
        1_000 + SNAPSHOT_RECONNECT_COOLDOWN_MS
    ));
}
