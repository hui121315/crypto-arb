use super::*;

pub(super) fn read_entry<T: Clone>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    key: &MarketKey,
    now_ms: i64,
    fresh_ms: i64,
) -> Option<MarketRead<T>> {
    let entry = entries.get(key)?;
    let age_ms = now_ms.saturating_sub(entry.received_at_ms);
    (age_ms <= fresh_ms).then(|| MarketRead::fresh(entry.data.clone(), age_ms, entry.source))
}

pub(super) fn fresh_values<T: Clone>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    fresh_ms: i64,
) -> Vec<T> {
    entries
        .iter()
        .filter_map(|entry| {
            let age_ms = now_ms.saturating_sub(entry.received_at_ms);
            (age_ms <= fresh_ms).then(|| entry.data.clone())
        })
        .collect()
}

pub(super) fn fresh_rows_snapshot<T: Clone>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    fresh_ms: i64,
    operation: MarketDataSnapshotOperation,
    row_key: impl Fn(&T) -> (String, String),
) -> MarketRowsSnapshot<T> {
    discovery_rows_snapshot(entries, now_ms, fresh_ms, fresh_ms, operation, row_key)
}

pub(super) fn discovery_rows_snapshot<T: Clone>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    fresh_ms: i64,
    max_age_ms: i64,
    operation: MarketDataSnapshotOperation,
    row_key: impl Fn(&T) -> (String, String),
) -> MarketRowsSnapshot<T> {
    let mut pairs = Vec::new();
    for entry in entries.iter() {
        let age_ms = now_ms.saturating_sub(entry.received_at_ms);
        if age_ms > max_age_ms {
            continue;
        }
        let row = entry.data.clone();
        let (venue, symbol) = row_key(&row);
        let quality = if age_ms <= fresh_ms {
            SharedMarketDataQuality::Fresh
        } else {
            SharedMarketDataQuality::StaleAllowed
        };
        let evidence =
            cached_row_evidence(venue, symbol, operation, entry.value(), now_ms, quality);
        pairs.push((row, evidence));
    }
    pairs.sort_by(|left, right| compare_row_evidence(&left.1, &right.1));
    let mut rows = Vec::with_capacity(pairs.len());
    let mut row_evidence = Vec::with_capacity(pairs.len());
    for (row, evidence) in pairs {
        rows.push(row);
        row_evidence.push(evidence);
    }
    MarketRowsSnapshot { rows, row_evidence }
}

pub(super) fn cached_row_evidence_for<T>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    rows: &[T],
    now_ms: i64,
    fresh_ms: i64,
    operation: MarketDataSnapshotOperation,
    row_identity: impl Fn(&T) -> (String, String, MarketKey),
) -> Vec<MarketDataRowEvidence> {
    let mut evidence = Vec::with_capacity(rows.len());
    for row in rows {
        let (venue, symbol, key) = row_identity(row);
        let Some(entry) = entries.get(&key) else {
            continue;
        };
        let age_ms = now_ms.saturating_sub(entry.received_at_ms);
        if age_ms <= fresh_ms {
            evidence.push(cached_row_evidence(
                venue,
                symbol,
                operation,
                entry.value(),
                now_ms,
                SharedMarketDataQuality::Fresh,
            ));
        }
    }
    evidence.sort_by(compare_row_evidence);
    evidence
}

pub(super) fn projected_row_evidence<T>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    keys: &[ProjectedRowKey],
    now_ms: i64,
    fresh_ms: i64,
    max_age_ms: i64,
    operation: MarketDataSnapshotOperation,
) -> Vec<MarketDataRowEvidence> {
    let mut evidence = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(entry) = entries.get(&key.cache_key) else {
            continue;
        };
        let age_ms = now_ms.saturating_sub(entry.received_at_ms);
        if age_ms > max_age_ms {
            continue;
        }
        let quality = if age_ms <= fresh_ms {
            SharedMarketDataQuality::Fresh
        } else {
            SharedMarketDataQuality::StaleAllowed
        };
        evidence.push(cached_row_evidence(
            key.venue.clone(),
            key.symbol.clone(),
            operation,
            entry.value(),
            now_ms,
            quality,
        ));
    }
    evidence
}

pub(super) fn cached_row_evidence<T>(
    venue: String,
    symbol: String,
    operation: MarketDataSnapshotOperation,
    entry: &CachedEntry<T>,
    now_ms: i64,
    quality: SharedMarketDataQuality,
) -> MarketDataRowEvidence {
    MarketDataRowEvidence {
        venue,
        symbol,
        operation,
        health: MarketDataHealth {
            quality,
            source: super::super::envelope::source(entry.source),
            freshness_ms: Some(now_ms.saturating_sub(entry.received_at_ms).max(0)),
            retry_after_ms: cached_retry_after_ms(entry, now_ms)
                .and_then(|ms| u64::try_from(ms.max(0)).ok()),
            last_error: entry.last_error.clone(),
            observed_at_ms: entry.received_at_ms,
            coverage: Some(coverage(1, 1)),
            problem: None,
        },
    }
}

pub(super) fn compare_row_evidence(
    left: &MarketDataRowEvidence,
    right: &MarketDataRowEvidence,
) -> std::cmp::Ordering {
    left.venue
        .cmp(&right.venue)
        .then_with(|| left.symbol.cmp(&right.symbol))
        .then_with(|| left.operation.as_str().cmp(right.operation.as_str()))
}

pub(super) fn stale_adjusted_runtime_health(
    mut row: MarketRuntimeHealth,
    now_ms: i64,
) -> MarketRuntimeHealth {
    let freshness_ms = now_ms.saturating_sub(row.observed_at_ms);
    let stale_ms = runtime_health_fresh_ms(row.operation);
    if row.quality == MarketQuality::Fresh && freshness_ms > stale_ms {
        row.quality = MarketQuality::StaleAllowed;
        row.last_error = Some(format!(
            "market runtime sample stale: freshness_ms={freshness_ms}, limit_ms={stale_ms}"
        ));
    }
    row
}

pub(super) fn runtime_health_fresh_ms(operation: &str) -> i64 {
    match operation {
        MARKET_OP_REST_FUNDING_RATES
        | MARKET_OP_REST_FUNDING_FALLBACK
        | MARKET_OP_WS_FUNDING
        | MARKET_OP_WS_FUNDING_SUBSCRIBE
        | MARKET_OP_WS_FUNDING_SNAPSHOT => FUNDING_FRESH_MS,
        MARKET_OP_REST_INDEX_COMPOSITIONS => INDEX_COMPOSITION_FRESH_MS,
        MARKET_OP_REST_METADATA => METADATA_FRESH_MS,
        MARKET_OP_WS_ORDERBOOKS => ORDERBOOK_FRESH_MS,
        MARKET_OP_REST_PERP_TICKERS
        | MARKET_OP_REST_SPOT_TICKS
        | MARKET_OP_REST_TICKER_FALLBACK
        | MARKET_OP_WS_TICKER
        | MARKET_OP_WS_TICKER_SUBSCRIBE
        | MARKET_OP_WS_TICKER_SNAPSHOT => TICKER_FRESH_MS,
        _ => MARKET_RUNTIME_HEALTH_TTL_MS,
    }
}

pub(super) fn runtime_health_ttl_ms(operation: &str) -> i64 {
    MARKET_RUNTIME_HEALTH_TTL_MS.max(runtime_health_fresh_ms(operation))
}
