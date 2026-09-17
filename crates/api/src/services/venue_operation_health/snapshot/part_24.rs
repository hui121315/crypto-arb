fn append_account_runtime_rows(
    rows: &mut Vec<VenueOperationHealth>,
    credentials: &[VenueCredentialStatus],
    state: &AppState,
    observed_at_ms: i64,
) -> (Vec<AccountCacheSnapshot>, Vec<AccountCacheSnapshot>) {
    let balance_cache = state.trading_service().balance_cache_health();
    let position_cache = state.trading_service().position_cache_health();
    overlay_private_read_runtime_rows(
        rows,
        private_read_runtime_rows(
            credentials,
            &balance_cache,
            &position_cache,
            observed_at_ms,
        ),
    );
    rows.extend(account_cache_rows(
        credentials,
        balance_cache.clone(),
        OP_BALANCE,
        observed_at_ms,
    ));
    rows.extend(account_cache_rows(
        credentials,
        position_cache.clone(),
        OP_POSITIONS,
        observed_at_ms,
    ));
    (balance_cache, position_cache)
}

fn private_read_runtime_rows(
    credentials: &[VenueCredentialStatus],
    balance_cache: &[AccountCacheSnapshot],
    position_cache: &[AccountCacheSnapshot],
    observed_at_ms: i64,
) -> Vec<VenueOperationHealth> {
    credentials
        .iter()
        .map(|credential| {
            if !credential.private_read || !credentials_configured(credential) {
                return credential_row(
                    credential,
                    OP_PRIVATE_READ,
                    credential.private_read,
                    observed_at_ms,
                );
            }
            best_private_read_cache_row(credential, balance_cache, position_cache)
                .unwrap_or_else(|| {
                    credential_row(credential, OP_PRIVATE_READ, true, observed_at_ms)
                })
        })
        .collect()
}

fn best_private_read_cache_row(
    credential: &VenueCredentialStatus,
    balance_cache: &[AccountCacheSnapshot],
    position_cache: &[AccountCacheSnapshot],
) -> Option<VenueOperationHealth> {
    balance_cache
        .iter()
        .map(|snapshot| (OP_BALANCE, snapshot))
        .chain(
            position_cache
                .iter()
                .map(|snapshot| (OP_POSITIONS, snapshot)),
        )
        .filter(|(_, snapshot)| private_read_cache_matches(&credential.venue, &snapshot.venue))
        .map(|(operation, snapshot)| {
            let status = account_cache_status(operation, snapshot.quality, snapshot.freshness_ms);
            (
                private_read_status_rank(status),
                snapshot.freshness_ms,
                operation,
                snapshot,
                status,
            )
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
        .map(|(_, _, operation, snapshot, status)| {
            let message = format!("私有账户读取已有运行态样本：{operation}");
            VenueOperationHealth {
                venue: credential.venue.clone(),
                operation: OP_PRIVATE_READ.to_owned(),
                status,
                source: SOURCE_ACCOUNT_CACHE.to_owned(),
                message: message.clone(),
                supported: Some(true),
                configured: Some(true),
                requested: Some(1),
                rows: Some(snapshot.rows),
                freshness_ms: Some(snapshot.freshness_ms),
                retry_after_ms: None,
                latency_ms: None,
                latency_p95_ms: None,
                error: account_cache_error(status, &message),
                evidence: None,
                problem: None,
                observed_at_ms: snapshot.observed_at_ms,
            }
        })
}

fn private_read_cache_matches(credential_venue: &str, snapshot_venue: &str) -> bool {
    let credential = normalized_venue_name(credential_venue);
    let snapshot = normalized_venue_name(snapshot_venue);
    snapshot == credential || normalized_venue_name(venue_family(&snapshot)) == credential
}

fn private_read_status_rank(status: VenueOperationStatus) -> u8 {
    match status {
        VenueOperationStatus::Ok => 4,
        VenueOperationStatus::Warn => 3,
        VenueOperationStatus::Blocked => 2,
        VenueOperationStatus::Unknown => 1,
        VenueOperationStatus::Unsupported => 0,
    }
}

fn overlay_private_read_runtime_rows(
    rows: &mut Vec<VenueOperationHealth>,
    runtime_rows: Vec<VenueOperationHealth>,
) {
    for runtime_row in runtime_rows {
        let venue_key = normalized_venue_name(&runtime_row.venue);
        if let Some(row) = rows.iter_mut().find(|row| {
            normalized_venue_name(&row.venue) == venue_key && row.operation == OP_PRIVATE_READ
        }) {
            *row = runtime_row;
        } else {
            rows.push(runtime_row);
        }
    }
}
