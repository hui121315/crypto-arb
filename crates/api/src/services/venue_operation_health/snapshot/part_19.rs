pub(crate) fn watchlist_alert_storage_health_row(
    state: &AppState,
    now_ms: i64,
) -> VenueOperationHealth {
    let health = state.watchlist_alert_store().health();
    let status = match health.status {
        shared_types::WatchlistStorageStatus::Disabled => VenueOperationStatus::Unknown,
        shared_types::WatchlistStorageStatus::Ready => VenueOperationStatus::Ok,
        shared_types::WatchlistStorageStatus::Degraded => VenueOperationStatus::Blocked,
    };
    let message = match health.status {
        shared_types::WatchlistStorageStatus::Disabled => {
            "watchlist alert storage is inactive while the optional surface is disabled".to_owned()
        }
        shared_types::WatchlistStorageStatus::Ready => format!(
            "SQLite snapshot ready at revision {} with {} watchlist item(s) and {} alert rule(s)",
            health.revision, health.watchlist_item_count, health.alert_rule_count
        ),
        shared_types::WatchlistStorageStatus::Degraded => {
            "watchlist alert SQLite snapshot is degraded; mutations fail closed".to_owned()
        }
    };
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: shared_types::OP_STORAGE_WATCHLIST_ALERTS.to_owned(),
        status,
        source: "watchlist_alert_sqlite".to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(health.configured),
        requested: Some(health.persist_attempts),
        rows: Some(
            health
                .watchlist_item_count
                .saturating_add(health.alert_rule_count) as u64,
        ),
        freshness_ms: health
            .last_persisted_at_ms
            .map(|at_ms| freshness_since(at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(watchlist_storage_evidence(&health)),
        problem: health.problem.clone(),
        observed_at_ms: health.last_persisted_at_ms.unwrap_or(now_ms),
    }
}

pub(crate) fn watchlist_prewarm_health_row(
    state: &AppState,
    now_ms: i64,
) -> VenueOperationHealth {
    let configured = state.config().api_surface.watchlist_alerts;
    let Ok(watchlist) = state.watchlist().try_read() else {
        return watchlist_prewarm_busy_row(configured, now_ms);
    };
    let requested = watchlist
        .iter()
        .map(|item| item.runtime.requested_public_legs as u64)
        .sum::<u64>();
    let healthy = watchlist
        .iter()
        .filter(|item| {
            matches!(
                item.runtime.status,
                shared_types::WatchlistPrewarmStatus::Idle
                    | shared_types::WatchlistPrewarmStatus::Disabled
                    | shared_types::WatchlistPrewarmStatus::Planned
                    | shared_types::WatchlistPrewarmStatus::Fresh
            )
        })
        .count();
    let attention = watchlist.iter().find(|item| {
        matches!(
            item.runtime.status,
            shared_types::WatchlistPrewarmStatus::Degraded
                | shared_types::WatchlistPrewarmStatus::Capped
        )
    });
    let status = if !configured {
        VenueOperationStatus::Unknown
    } else if attention.is_some() {
        VenueOperationStatus::Warn
    } else {
        VenueOperationStatus::Ok
    };
    let message = if !configured {
        "watchlist public prewarm is inactive while the optional surface is disabled".to_owned()
    } else if let Some(item) = attention {
        format!(
            "watchlist public prewarm needs attention for item {} ({})",
            item.id, item.symbol
        )
    } else {
        format!(
            "watchlist public prewarm is bounded for {} item(s); private WS symbols remain zero",
            watchlist.len()
        )
    };
    let observed_at_ms = watchlist
        .iter()
        .filter_map(|item| item.runtime.last_prewarm_at_ms)
        .max()
        .unwrap_or(now_ms);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: shared_types::OP_WATCHLIST_PREWARM.to_owned(),
        status,
        source: "watchlist_public_prewarm".to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(configured),
        requested: Some(requested),
        rows: Some(healthy as u64),
        freshness_ms: Some(freshness_since(observed_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(watchlist_prewarm_evidence(&watchlist)),
        problem: attention.and_then(|item| item.runtime.problem.clone()),
        observed_at_ms,
    }
}

fn watchlist_prewarm_busy_row(configured: bool, now_ms: i64) -> VenueOperationHealth {
    let message = "watchlist public prewarm snapshot was busy; retry on the next health rebuild";
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: shared_types::OP_WATCHLIST_PREWARM.to_owned(),
        status: VenueOperationStatus::Warn,
        source: "watchlist_public_prewarm".to_owned(),
        message: message.to_owned(),
        supported: Some(true),
        configured: Some(configured),
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms: Some(5_000),
        latency_ms: None,
        latency_p95_ms: None,
        error: Some(message.to_owned()),
        evidence: None,
        problem: Some(
            ApiProblem::new("WATCHLIST_PREWARM_SNAPSHOT_BUSY", message)
                .with_retry_after_ms(Some(5_000))
                .with_source("watchlist_public_prewarm"),
        ),
        observed_at_ms: now_ms,
    }
}

fn watchlist_storage_evidence(
    health: &shared_types::WatchlistStorageHealth,
) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "sqlite_snapshot".to_owned(),
        path: "watchlist_alert_snapshots".to_owned(),
        checked_at: "2026-07-14".to_owned(),
        doc_version: realtime::alerts::WATCHLIST_ALERT_STORAGE_MIGRATION_ID.to_owned(),
        schema_hash: realtime::alerts::storage_schema_hash(),
        fixture_id: "watchlist-alert-sqlite-roundtrip".to_owned(),
        parser_test:
            "sqlite_snapshot_restores_config_delivery_and_cooldown_without_stale_prewarm"
                .to_owned(),
        request_builder_test: "app_state_restores_watchlist_alert_snapshot_and_active_cooldown"
            .to_owned(),
        auth_kind: "local_sqlite".to_owned(),
        request_id: None,
        request_context: vec![
            format!("configured={}", health.configured),
            format!("status={:?}", health.status),
            format!("revision={}", health.revision),
            format!("watchlist_items={}", health.watchlist_item_count),
            format!("alert_rules={}", health.alert_rule_count),
            format!("persist_attempts={}", health.persist_attempts),
            format!("persist_successes={}", health.persist_successes),
        ],
        doc_urls: Vec::new(),
        use_cases: vec![
            "watchlist_restart_restore".to_owned(),
            "alert_delivery_cooldown_restore".to_owned(),
        ],
        data_kinds: vec!["watchlist_item".to_owned(), "alert_rule".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn watchlist_prewarm_evidence(
    watchlist: &[shared_types::WatchlistItem],
) -> VenueOperationEvidence {
    let mut request_context = vec![
        format!("watchlist_items={}", watchlist.len()),
        "private_ws_symbols=0".to_owned(),
    ];
    request_context.extend(watchlist.iter().take(16).map(|item| {
        format!(
            "item={}:{}:status={:?}:requested={}:capped={}",
            item.id,
            item.symbol,
            item.runtime.status,
            item.runtime.requested_public_legs,
            item.runtime.capped_legs
        )
    }));
    VenueOperationEvidence {
        method: "bounded_public_prewarm".to_owned(),
        path: "watchlist".to_owned(),
        checked_at: "2026-07-14".to_owned(),
        doc_version: "PR-DP".to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: "watchlist-prewarm-runtime".to_owned(),
        parser_test: "watchlist_runtime_plan_is_bounded_visible_and_private_ws_free".to_owned(),
        request_builder_test: "ticker_plan_prevents_false_full_leg_cap".to_owned(),
        auth_kind: "public_market_data_only".to_owned(),
        request_id: None,
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec!["watchlist_public_prewarm".to_owned()],
        data_kinds: vec!["orderbook".to_owned(), "ticker".to_owned()],
        rate_scopes: vec!["watchlist_bounded".to_owned()],
        weight: 0,
    }
}
