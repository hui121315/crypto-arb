#[test]
fn fresh_account_cache_replaces_private_read_credential_unknown() {
    let credential = credential(true, true);
    let balance = AccountCacheSnapshot {
        venue: "binance".to_owned(),
        rows: 1,
        freshness_ms: 50,
        observed_at_ms: 20,
        quality: AccountCacheQuality::Fresh,
    };
    let mut rows = credential_rows(std::slice::from_ref(&credential), 10);

    overlay_private_read_runtime_rows(
        &mut rows,
        private_read_runtime_rows(std::slice::from_ref(&credential), &[balance], &[], 20),
    );
    let private_read = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_READ)
        .expect("private read row");

    assert_eq!(private_read.status, VenueOperationStatus::Ok);
    assert_eq!(private_read.source, SOURCE_ACCOUNT_CACHE);
    assert_eq!(private_read.rows, Some(1));
    assert_eq!(private_read.freshness_ms, Some(50));
}

#[test]
fn missing_account_cache_keeps_private_read_unknown() {
    let credential = credential(true, true);

    let rows = private_read_runtime_rows(std::slice::from_ref(&credential), &[], &[], 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Unknown);
    assert_eq!(rows[0].source, SOURCE_CREDENTIAL_CONFIG);
}

#[test]
fn hyperliquid_child_cache_proves_family_private_read() {
    let mut credential = credential(true, true);
    credential.venue = "hyperliquid".to_owned();
    let balance = AccountCacheSnapshot {
        venue: "hyperliquid:spot".to_owned(),
        rows: 3,
        freshness_ms: 10,
        observed_at_ms: 20,
        quality: AccountCacheQuality::Fresh,
    };

    let rows = private_read_runtime_rows(&[credential], &[balance], &[], 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].rows, Some(3));
}
