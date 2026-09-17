fn overlay_idle_private_ws_readiness(
    rows: &mut [VenueOperationHealth],
    balance_cache: &[AccountCacheSnapshot],
    position_cache: &[AccountCacheSnapshot],
) {
    let idle_streams = rows
        .iter()
        .filter(|row| {
            row.status == VenueOperationStatus::Unknown
                && row.supported == Some(true)
                && row.configured == Some(true)
                && matches!(
                    row.operation.as_str(),
                    OP_PRIVATE_WS_ORDER_STREAM | OP_PRIVATE_WS_ACCOUNT_STREAM
                )
        })
        .map(|row| (normalized_venue_name(&row.venue), row.operation.clone()))
        .collect::<Vec<_>>();

    for (venue, operation) in idle_streams {
        let Some((observed_at_ms, freshness_ms)) = current_private_ws_readiness(rows, &venue)
        else {
            continue;
        };
        let account_readiness = (operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
            .then(|| account_cache_readiness(&venue, balance_cache, position_cache))
            .flatten();
        if operation == OP_PRIVATE_WS_ACCOUNT_STREAM && account_readiness.is_none() {
            continue;
        }
        let Some(row) = rows.iter_mut().find(|row| {
            normalized_venue_name(&row.venue) == venue
                && row.operation == operation
                && row.status == VenueOperationStatus::Unknown
        }) else {
            continue;
        };

        if let Some(account) = account_readiness {
            let message = if account.has_bounded_refresh {
                "私有 WS 当前会话已连接并确认订阅；余额与持仓快照可用，后台刷新中，空闲等待账户事件"
            } else {
                "私有 WS 当前会话已连接并确认订阅；余额与持仓缓存新鲜，空闲等待账户事件"
            };
            mark_idle_stream_ready(
                row,
                observed_at_ms.max(account.observed_at_ms),
                Some(
                    freshness_ms
                        .unwrap_or_default()
                        .max(account.freshness_ms),
                ),
                message,
                &[
                    "stream_readiness=current_session_subscription_idle".to_owned(),
                    "event_sample=current_session_none_expected".to_owned(),
                    format!("account_cache_rows={}", account.rows),
                    format!(
                        "account_cache_readiness={}",
                        if account.has_bounded_refresh {
                            "bounded_refresh"
                        } else {
                            "fresh"
                        }
                    ),
                ],
            );
        } else {
            mark_idle_stream_ready(
                row,
                observed_at_ms,
                freshness_ms,
                "私有 WS 当前会话已连接并确认订阅；当前无未决实盘订单，空闲等待订单事件",
                &[
                    "stream_readiness=current_session_subscription_idle".to_owned(),
                    "event_sample=current_session_none_expected".to_owned(),
                ],
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccountCacheReadiness {
    observed_at_ms: i64,
    freshness_ms: i64,
    rows: u64,
    has_bounded_refresh: bool,
}

fn account_cache_readiness(
    venue: &str,
    balance_cache: &[AccountCacheSnapshot],
    position_cache: &[AccountCacheSnapshot],
) -> Option<AccountCacheReadiness> {
    let balance = ready_account_cache_for_venue(balance_cache, venue, OP_BALANCE)?;
    let positions = ready_account_cache_for_venue(position_cache, venue, OP_POSITIONS)?;
    Some(AccountCacheReadiness {
        observed_at_ms: balance.observed_at_ms.max(positions.observed_at_ms),
        freshness_ms: balance.freshness_ms.max(positions.freshness_ms),
        rows: balance.rows.saturating_add(positions.rows),
        has_bounded_refresh: balance.quality == AccountCacheQuality::Stale
            || positions.quality == AccountCacheQuality::Stale,
    })
}

fn ready_account_cache_for_venue<'a>(
    snapshots: &'a [AccountCacheSnapshot],
    venue: &str,
    operation: &str,
) -> Option<&'a AccountCacheSnapshot> {
    snapshots
        .iter()
        .filter(|snapshot| {
            private_read_cache_matches(venue, &snapshot.venue)
                && account_cache_status(operation, snapshot.quality, snapshot.freshness_ms)
                    == VenueOperationStatus::Ok
        })
        .min_by_key(|snapshot| snapshot.freshness_ms)
}

fn mark_idle_stream_ready(
    row: &mut VenueOperationHealth,
    observed_at_ms: i64,
    freshness_ms: Option<i64>,
    message: &str,
    evidence_context: &[String],
) {
    row.status = VenueOperationStatus::Ok;
    row.message = message.to_owned();
    row.rows = Some(0);
    row.freshness_ms = freshness_ms;
    row.retry_after_ms = None;
    row.error = None;
    row.problem = None;
    row.observed_at_ms = observed_at_ms;
    if let Some(evidence) = row.evidence.as_mut() {
        evidence.request_context.extend_from_slice(evidence_context);
    }
}
