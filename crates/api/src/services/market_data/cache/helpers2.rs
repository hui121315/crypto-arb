use super::*;

#[derive(Default)]
pub(super) struct FeedMeta {
    pub(super) requested: u64,
    pub(super) received: u64,
    pub(super) latest_received_at_ms: Option<i64>,
    pub(super) latest_source: Option<MarketSource>,
}

pub(super) fn runtime_status_row(row: MarketRuntimeHealth) -> Option<MarketDataSnapshotStatusRow> {
    Some(MarketDataSnapshotStatusRow {
        venue: row.venue.clone(),
        operation: snapshot_operation(row.operation)?,
        health: health_from_runtime(row),
    })
}

pub(super) fn snapshot_operation(operation: &str) -> Option<MarketDataSnapshotOperation> {
    match operation {
        MARKET_OP_REST_FUNDING_RATES => Some(MarketDataSnapshotOperation::FundingRates),
        MARKET_OP_REST_INDEX_COMPOSITIONS => Some(MarketDataSnapshotOperation::IndexCompositions),
        MARKET_OP_REST_METADATA => Some(MarketDataSnapshotOperation::Metadata),
        MARKET_OP_WS_ORDERBOOKS => Some(MarketDataSnapshotOperation::Orderbooks),
        MARKET_OP_REST_PERP_TICKERS => Some(MarketDataSnapshotOperation::PerpTickers),
        MARKET_OP_REST_SPOT_TICKS => Some(MarketDataSnapshotOperation::SpotTicks),
        MARKET_OP_WS_FUNDING | MARKET_OP_WS_FUNDING_SNAPSHOT | MARKET_OP_REST_FUNDING_FALLBACK => {
            Some(MarketDataSnapshotOperation::WsFunding)
        }
        MARKET_OP_WS_TICKER | MARKET_OP_WS_TICKER_SNAPSHOT | MARKET_OP_REST_TICKER_FALLBACK => {
            Some(MarketDataSnapshotOperation::WsTicker)
        }
        // Subscription confirmation is transport health, not data coverage. Folding it into the
        // snapshot operation creates conflicting duplicate rows beside the ingest outcome.
        MARKET_OP_WS_FUNDING_SUBSCRIBE | MARKET_OP_WS_TICKER_SUBSCRIBE => None,
        MARKET_OP_WS_SPOT_SNAPSHOT => Some(MarketDataSnapshotOperation::WsSpotTicks),
        _ => None,
    }
}

pub(super) fn push_cache_status_row<T>(
    rows: &mut Vec<MarketDataSnapshotStatusRow>,
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    spec: CacheStatusSpec,
) {
    let meta = feed_meta(entries, now_ms, spec.fresh_ms);
    let existing_index = rows
        .iter()
        .position(|row| row.venue == MARKET_AGGREGATE_VENUE && row.operation == spec.operation);
    if spec.operation == MarketDataSnapshotOperation::Orderbooks && meta.requested == 0 {
        let runtime_requested = existing_index
            .and_then(|index| rows[index].health.coverage.as_ref())
            .map(|coverage| coverage.requested)
            .unwrap_or(0);
        if runtime_requested == 0 {
            if let Some(index) = existing_index {
                rows.remove(index);
            }
            return;
        }
    }
    if let Some(index) = existing_index {
        if should_keep_runtime_status(&rows[index], &meta, spec) {
            return;
        }
        rows.remove(index);
    }
    let quality = if meta.received == 0 {
        MarketQuality::Missing
    } else {
        MarketQuality::Fresh
    };
    let source = meta.latest_source.unwrap_or(MarketSource::LocalCache);
    let mut health = health_from_runtime(MarketRuntimeHealth {
        venue: MARKET_AGGREGATE_VENUE.to_owned(),
        operation: spec.runtime_operation,
        quality,
        source,
        requested: meta.requested,
        rows: meta.received,
        retry_after_ms: None,
        last_error: (quality != MarketQuality::Fresh).then(|| spec.missing_message.to_owned()),
        problem: None,
        observed_at_ms: now_ms,
    });
    health.coverage = Some(coverage(meta.requested, meta.received));
    health.freshness_ms = meta
        .latest_received_at_ms
        .map(|received_at| now_ms.saturating_sub(received_at).max(0));
    rows.push(MarketDataSnapshotStatusRow {
        venue: MARKET_AGGREGATE_VENUE.to_owned(),
        operation: spec.operation,
        health,
    });
}

pub(super) fn remove_scan_rows_superseded_by_fresh_ws(rows: &mut Vec<MarketDataSnapshotStatusRow>) {
    let fresh_ws = rows
        .iter()
        .filter(|row| {
            row.health.quality == SharedMarketDataQuality::Fresh
                && row
                    .health
                    .coverage
                    .as_ref()
                    .is_some_and(|coverage| coverage.received > 0)
        })
        .filter_map(|row| {
            let operation = match row.operation {
                MarketDataSnapshotOperation::WsFunding => MarketDataSnapshotOperation::FundingRates,
                MarketDataSnapshotOperation::WsTicker => MarketDataSnapshotOperation::PerpTickers,
                MarketDataSnapshotOperation::WsSpotTicks => MarketDataSnapshotOperation::SpotTicks,
                _ => return None,
            };
            Some((shared_types::normalized_venue_name(&row.venue), operation))
        })
        .collect::<Vec<_>>();
    rows.retain(|row| {
        row.health.quality == SharedMarketDataQuality::Fresh
            || !fresh_ws.iter().any(|(venue, operation)| {
                *operation == row.operation
                    && *venue == shared_types::normalized_venue_name(&row.venue)
            })
    });
}

pub(super) fn should_keep_runtime_status(
    row: &MarketDataSnapshotStatusRow,
    meta: &FeedMeta,
    spec: CacheStatusSpec,
) -> bool {
    if meta.received == 0 {
        return true;
    }
    if spec.operation != MarketDataSnapshotOperation::Orderbooks {
        return false;
    }
    let requested = row
        .health
        .coverage
        .as_ref()
        .map(|coverage| coverage.requested)
        .unwrap_or(0);
    row.health.quality != SharedMarketDataQuality::Fresh && meta.received < requested
}

pub(super) fn feed_meta<T>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    fresh_ms: i64,
) -> FeedMeta {
    let mut meta = FeedMeta::default();
    for entry in entries.iter() {
        meta.requested += 1;
        let age_ms = now_ms.saturating_sub(entry.received_at_ms);
        if age_ms > fresh_ms {
            continue;
        }
        meta.received += 1;
        if latest_is_before(meta.latest_received_at_ms, entry.received_at_ms) {
            meta.latest_received_at_ms = Some(entry.received_at_ms);
            meta.latest_source = Some(entry.source);
        }
    }
    meta
}

pub(super) fn latest_is_before(latest: Option<i64>, candidate: i64) -> bool {
    match latest {
        Some(value) => candidate > value,
        None => true,
    }
}

pub(super) fn fetch_backoff_wait(
    backoffs: &DashMap<MarketKey, MarketFetchBackoff>,
    key: &MarketKey,
    now_ms: i64,
) -> Option<ActiveFetchBackoff> {
    let backoff = backoffs.get(key).map(|entry| entry.value().clone())?;
    if backoff.until_ms > now_ms {
        Some(ActiveFetchBackoff {
            wait_ms: backoff.until_ms - now_ms,
            quality: backoff.quality,
            last_error: backoff.last_error,
        })
    } else {
        backoffs.remove(key);
        None
    }
}

pub(super) fn index_composition_guard_key(venue: &str) -> MarketKey {
    MarketKey::new("index-composition", venue)
}

pub(super) fn retry_after_ms(error: &ExchangeError) -> Option<i64> {
    match error {
        ExchangeError::RateLimited { retry_after_secs } => Some(
            (*retry_after_secs as i64)
                .saturating_mul(1_000)
                .max(RETRY_AFTER_FLOOR_MS),
        ),
        _ => None,
    }
}

pub(super) fn error_backoff_ms(error: &ExchangeError) -> i64 {
    if is_unsupported_symbol_error(error) {
        ORDERBOOK_UNSUPPORTED_CACHE_MS
    } else {
        retry_after_ms(error).unwrap_or(ORDERBOOK_NEGATIVE_CACHE_MS)
    }
}

pub(super) fn index_composition_backoff_ms(error: &ExchangeError) -> i64 {
    retry_after_ms(error)
        .unwrap_or(INDEX_COMPOSITION_NEGATIVE_CACHE_MS)
        .max(INDEX_COMPOSITION_NEGATIVE_CACHE_MS)
}

pub(super) fn error_quality(error: &ExchangeError) -> MarketQuality {
    if is_unsupported_symbol_error(error) {
        return MarketQuality::Unsupported;
    }
    match error {
        ExchangeError::RateLimited { .. } => MarketQuality::RateLimited,
        ExchangeError::CircuitBreaker { .. } => MarketQuality::CircuitOpen,
        ExchangeError::NotImplemented(_)
        | ExchangeError::UnsupportedCapability(_)
        | ExchangeError::UnsupportedSymbol(_) => MarketQuality::Unsupported,
        _ => MarketQuality::Missing,
    }
}

fn is_unsupported_symbol_error(error: &ExchangeError) -> bool {
    match error {
        ExchangeError::UnsupportedSymbol(_) => true,
        ExchangeError::Api { code, message, .. } => {
            matches!(code.as_str(), "25100" | "40034" | "10001" | "200003")
                || unsupported_symbol_message(message)
        }
        ExchangeError::Http { body, .. } => unsupported_symbol_message(body),
        _ => false,
    }
}

fn unsupported_symbol_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("invalid symbol")
        || message.contains("symbol invalid")
        || message.contains("symbol does not exist")
        || message.contains("trading pair") && message.contains("does not exist")
}
