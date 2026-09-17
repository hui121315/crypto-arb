use super::*;

pub(super) fn fallback_funding_health(
    rows: &[FundingRateData],
    read_source: MarketSource,
    observed_at_ms: i64,
) -> MarketDataHealth {
    let latest_at_ms = rows.iter().map(|row| row.timestamp).max();
    let quality = if rows.is_empty() {
        MarketQuality::Missing
    } else {
        MarketQuality::Fresh
    };
    let message = "funding rates unavailable";
    MarketDataHealth {
        quality: self::quality(quality),
        source: source(read_source),
        freshness_ms: latest_at_ms.map(|latest| observed_at_ms.saturating_sub(latest)),
        retry_after_ms: None,
        last_error: (quality != MarketQuality::Fresh).then(|| message.to_owned()),
        observed_at_ms,
        coverage: Some(coverage(rows.len() as u64, rows.len() as u64)),
        problem: problem(quality, message, None, read_source),
    }
}

pub(super) fn orderbook_row_cap(
    book: Option<&OrderBookInfo>,
    venue: &str,
    symbol: &str,
    max_rows: usize,
) -> RowCapEvidence {
    let returned_count = book
        .map(|book| book.bids.len().max(book.asks.len()))
        .unwrap_or_default();
    RowCapEvidence::exact(
        max_rows,
        returned_count,
        returned_count,
        format!("orderbook:{venue}:{symbol}"),
    )
}

pub(super) fn health_from_read<T>(
    read: &MarketRead<T>,
    now_ms: i64,
    coverage: Option<MarketDataCoverage>,
    fallback_message: impl FnOnce() -> String,
) -> MarketDataHealth {
    let message = read.last_error.clone().unwrap_or_else(fallback_message);
    MarketDataHealth {
        quality: quality(read.quality),
        source: source(read.source),
        freshness_ms: read.freshness_ms,
        retry_after_ms: retry_after(read.retry_after_ms),
        last_error: read.last_error.clone(),
        observed_at_ms: now_ms,
        coverage,
        problem: problem(read.quality, &message, read.retry_after_ms, read.source),
    }
}

pub(crate) fn health_from_runtime(row: MarketRuntimeHealth) -> MarketDataHealth {
    let message = row
        .last_error
        .clone()
        .unwrap_or_else(|| format!("{} {} unavailable", row.venue, row.operation));
    let problem = row.problem.as_ref().map(|problem| {
        problem
            .to_api_problem(row.quality.problem_code())
            .with_retry_after_ms(row.retry_after_ms)
    });
    MarketDataHealth {
        quality: quality(row.quality),
        source: source(row.source),
        freshness_ms: None,
        retry_after_ms: row.retry_after_ms,
        last_error: row.last_error,
        observed_at_ms: row.observed_at_ms,
        coverage: Some(coverage(row.requested, row.rows)),
        problem: problem.or_else(|| {
            self::problem(
                row.quality,
                &message,
                row.retry_after_ms.map(|ms| ms as i64),
                row.source,
            )
        }),
    }
}

pub(super) fn fallback_spot_health(row_count: usize) -> MarketDataHealth {
    let quality = if row_count == 0 {
        MarketQuality::Missing
    } else {
        MarketQuality::Fresh
    };
    let message = "spot tick baseline has no runtime health";
    MarketDataHealth {
        quality: self::quality(quality),
        source: MarketDataSourceKind::LocalCache,
        freshness_ms: None,
        retry_after_ms: None,
        last_error: (quality != MarketQuality::Fresh).then(|| message.to_owned()),
        observed_at_ms: common::time::now_ms(),
        coverage: Some(coverage(0, row_count as u64)),
        problem: problem(quality, message, None, MarketSource::LocalCache),
    }
}

pub(super) fn is_spot_tick_health(row: &MarketRuntimeHealth) -> bool {
    row.venue == MARKET_AGGREGATE_VENUE && row.operation == MARKET_OP_REST_SPOT_TICKS
}

pub(super) fn is_funding_rate_health(row: &MarketRuntimeHealth) -> bool {
    row.venue == MARKET_AGGREGATE_VENUE && row.operation == MARKET_OP_REST_FUNDING_RATES
}

pub(super) fn fanout_outcomes(
    runtime_rows: &[MarketRuntimeHealth],
    runtime_operation: &str,
) -> Vec<MarketDataFanoutOutcome> {
    let Some(operation) = snapshot_operation(runtime_operation) else {
        return Vec::new();
    };
    runtime_rows
        .iter()
        .filter(|row| row.operation == runtime_operation)
        .filter(|row| row.venue != MARKET_AGGREGATE_VENUE)
        .cloned()
        .map(|row| MarketDataFanoutOutcome {
            venue: row.venue.clone(),
            operation,
            health: health_from_runtime(row),
        })
        .collect()
}

fn snapshot_operation(operation: &str) -> Option<MarketDataSnapshotOperation> {
    match operation {
        MARKET_OP_REST_FUNDING_RATES => Some(MarketDataSnapshotOperation::FundingRates),
        MARKET_OP_REST_INDEX_COMPOSITIONS => Some(MarketDataSnapshotOperation::IndexCompositions),
        MARKET_OP_WS_ORDERBOOKS => Some(MarketDataSnapshotOperation::Orderbooks),
        MARKET_OP_REST_PERP_TICKERS => Some(MarketDataSnapshotOperation::PerpTickers),
        MARKET_OP_REST_SPOT_TICKS => Some(MarketDataSnapshotOperation::SpotTicks),
        MARKET_OP_WS_FUNDING => Some(MarketDataSnapshotOperation::WsFunding),
        MARKET_OP_WS_TICKER => Some(MarketDataSnapshotOperation::WsTicker),
        MARKET_OP_WS_SPOT_SNAPSHOT => Some(MarketDataSnapshotOperation::WsSpotTicks),
        _ => None,
    }
}

pub(crate) fn coverage(requested: u64, received: u64) -> MarketDataCoverage {
    MarketDataCoverage::new(requested, received)
}

pub(super) fn problem(
    quality: MarketQuality,
    message: &str,
    retry_after_ms: Option<i64>,
    source: MarketSource,
) -> Option<ApiProblem> {
    (quality != MarketQuality::Fresh).then(|| {
        ApiProblem::new(quality.problem_code(), message)
            .with_retry_after_ms(retry_after(retry_after_ms))
            .with_source(source.as_str())
    })
}

fn retry_after(value: Option<i64>) -> Option<u64> {
    value.and_then(|ms| u64::try_from(ms.max(0)).ok())
}

pub(crate) fn quality(value: MarketQuality) -> SharedQuality {
    match value {
        MarketQuality::Fresh => SharedQuality::Fresh,
        MarketQuality::Warming => SharedQuality::Unverified,
        MarketQuality::StaleAllowed => SharedQuality::StaleAllowed,
        MarketQuality::Missing => SharedQuality::Missing,
        MarketQuality::RateLimited => SharedQuality::RateLimited,
        MarketQuality::CircuitOpen => SharedQuality::CircuitOpen,
        MarketQuality::Unsupported => SharedQuality::Unsupported,
    }
}

pub(crate) fn source(value: MarketSource) -> MarketDataSourceKind {
    match value {
        MarketSource::WsPush => MarketDataSourceKind::WsPush,
        MarketSource::RestColdStart => MarketDataSourceKind::RestColdStart,
        MarketSource::RestBaseline => MarketDataSourceKind::RestBaseline,
        MarketSource::LocalCache => MarketDataSourceKind::LocalCache,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_operation_maps_spot_ws_and_rejects_unknown_operations() {
        assert_eq!(
            snapshot_operation(MARKET_OP_WS_SPOT_SNAPSHOT),
            Some(MarketDataSnapshotOperation::WsSpotTicks)
        );
        assert_eq!(snapshot_operation("unknown_operation"), None);
    }
}
