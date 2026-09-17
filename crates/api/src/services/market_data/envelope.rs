use shared_types::{
    ApiProblem, FundingRateData, FundingRatesEnvelope, IndexCompositionListEnvelope,
    IndexCompositionSnapshot, MarketDataCoverage, MarketDataEnvelope, MarketDataFanoutOutcome,
    MarketDataHealth, MarketDataQuality as SharedQuality, MarketDataRowEvidence,
    MarketDataSnapshotOperation, MarketDataSourceKind, OrderBookInfo, RowCapEvidence,
};

use super::cache::{
    MARKET_AGGREGATE_VENUE, MARKET_OP_REST_FUNDING_RATES, MARKET_OP_REST_INDEX_COMPOSITIONS,
    MARKET_OP_REST_PERP_TICKERS, MARKET_OP_REST_SPOT_TICKS, MARKET_OP_WS_FUNDING,
    MARKET_OP_WS_ORDERBOOKS, MARKET_OP_WS_SPOT_SNAPSHOT, MARKET_OP_WS_TICKER,
};
use super::{MarketQuality, MarketRead, MarketRuntimeHealth, MarketSource};

mod health;

pub(crate) use health::{coverage, health_from_runtime, quality, source};
use health::{
    fallback_funding_health, fallback_spot_health, fanout_outcomes, health_from_read,
    is_funding_rate_health, is_spot_tick_health, orderbook_row_cap, problem,
};

pub(crate) fn orderbook_envelope(
    read: MarketRead<OrderBookInfo>,
    venue: &str,
    symbol: &str,
    max_rows: usize,
    now_ms: i64,
) -> MarketDataEnvelope<Option<OrderBookInfo>> {
    let received = u64::from(read.value.is_some());
    let health = health_from_read(&read, now_ms, Some(coverage(1, received)), || {
        format!("{venue} {symbol} orderbook unavailable")
    });
    MarketDataEnvelope {
        row_cap: Some(orderbook_row_cap(
            read.value.as_ref(),
            venue,
            symbol,
            max_rows,
        )),
        data: read.value,
        retry_after_ms: envelope_retry_after_ms(&health, &[], &[]),
        health,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    }
}

pub(crate) fn index_composition_envelope(
    read: MarketRead<IndexCompositionSnapshot>,
    venue: &str,
    symbol: &str,
    now_ms: i64,
) -> MarketDataEnvelope<Option<IndexCompositionSnapshot>> {
    let received = u64::from(read.value.is_some());
    let health = health_from_read(&read, now_ms, Some(coverage(1, received)), || {
        format!("{venue} {symbol} index composition unavailable")
    });
    MarketDataEnvelope {
        data: read.value,
        retry_after_ms: envelope_retry_after_ms(&health, &[], &[]),
        health,
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    }
}

pub(crate) fn index_composition_list_envelope(
    rows: Vec<IndexCompositionSnapshot>,
    now_ms: i64,
) -> IndexCompositionListEnvelope {
    let latest_at_ms = rows.iter().map(|row| row.received_at_ms).max();
    let quality = if rows.is_empty() {
        MarketQuality::Missing
    } else {
        MarketQuality::Fresh
    };
    let message = "index composition cache has no rows";
    let health = MarketDataHealth {
        quality: self::quality(quality),
        source: MarketDataSourceKind::LocalCache,
        freshness_ms: latest_at_ms.map(|latest| now_ms.saturating_sub(latest).max(0)),
        retry_after_ms: None,
        last_error: (quality != MarketQuality::Fresh).then(|| message.to_owned()),
        observed_at_ms: now_ms,
        coverage: Some(coverage(rows.len() as u64, rows.len() as u64)),
        problem: problem(quality, message, None, MarketSource::LocalCache),
    };
    IndexCompositionListEnvelope {
        data: rows,
        retry_after_ms: envelope_retry_after_ms(&health, &[], &[]),
        health,
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    }
}

pub(crate) fn spot_ticks_envelope<T>(
    data: T,
    row_count: usize,
    runtime_rows: &[MarketRuntimeHealth],
    row_evidence: Vec<MarketDataRowEvidence>,
    row_cap: Option<RowCapEvidence>,
    query_coverage: Option<MarketDataCoverage>,
) -> MarketDataEnvelope<T> {
    let fanout = fanout_outcomes(runtime_rows, MARKET_OP_REST_SPOT_TICKS);
    let mut health = runtime_rows
        .iter()
        .find(|row| is_spot_tick_health(row))
        .cloned()
        .map(health_from_runtime)
        .unwrap_or_else(|| fallback_spot_health(row_count));
    if let Some(coverage) = query_coverage {
        health.coverage = Some(coverage);
    }
    MarketDataEnvelope {
        data,
        retry_after_ms: envelope_retry_after_ms(&health, &row_evidence, &fanout),
        health,
        row_cap,
        row_evidence,
        fanout,
    }
}

pub(crate) fn funding_rates_envelope(
    rows: Vec<FundingRateData>,
    read_source: MarketSource,
    observed_at_ms: i64,
    runtime_rows: &[MarketRuntimeHealth],
    row_evidence: Vec<MarketDataRowEvidence>,
) -> FundingRatesEnvelope {
    let health = if read_source == MarketSource::LocalCache && !rows.is_empty() {
        fallback_funding_health(&rows, read_source, observed_at_ms)
    } else {
        runtime_rows
            .iter()
            .find(|row| is_funding_rate_health(row))
            .cloned()
            .map(health_from_runtime)
            .unwrap_or_else(|| fallback_funding_health(&rows, read_source, observed_at_ms))
    };
    let fanout = fanout_outcomes(runtime_rows, MARKET_OP_REST_FUNDING_RATES);
    let retry_after_ms = envelope_retry_after_ms(&health, &row_evidence, &fanout);
    FundingRatesEnvelope {
        data: rows,
        retry_after_ms,
        health,
        row_cap: None,
        row_evidence,
        fanout,
    }
}

fn envelope_retry_after_ms(
    health: &MarketDataHealth,
    row_evidence: &[MarketDataRowEvidence],
    fanout: &[MarketDataFanoutOutcome],
) -> Option<u64> {
    let mut retry_after_ms = None;
    push_health_retry_after(&mut retry_after_ms, health);
    for row in row_evidence {
        push_health_retry_after(&mut retry_after_ms, &row.health);
    }
    for outcome in fanout {
        push_health_retry_after(&mut retry_after_ms, &outcome.health);
    }
    retry_after_ms
}

fn push_health_retry_after(retry_after_ms: &mut Option<u64>, health: &MarketDataHealth) {
    *retry_after_ms = max_retry_after(*retry_after_ms, health.retry_after_ms);
    if let Some(problem) = health.problem.as_ref() {
        *retry_after_ms = max_retry_after(*retry_after_ms, problem.retry_after_ms);
    }
}

fn max_retry_after(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    left.into_iter().chain(right).max()
}

#[cfg(test)]
mod tests;
