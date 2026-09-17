use super::*;

pub(super) fn fanout_health_fields(
    outcome: &FanoutVenueResult,
) -> (MarketQuality, Option<u64>, Option<String>) {
    if let Some(error) = outcome.error.as_ref() {
        return (
            error_quality(error),
            retry_after_ms(error).map(|ms| ms.max(0) as u64),
            Some(error.to_string()),
        );
    }

    if outcome.rows == 0 {
        (
            MarketQuality::Missing,
            None,
            Some("fanout produced no rows".to_owned()),
        )
    } else {
        (MarketQuality::Fresh, None, None)
    }
}

pub(super) fn aggregate_fanout_health(
    operation: &'static str,
    source: MarketSource,
    outcomes: &[FanoutVenueResult],
) -> MarketRuntimeHealth {
    let requested = outcomes.len() as u64;
    let rows = outcomes.iter().filter(|outcome| outcome.rows > 0).count() as u64;
    let issue = aggregate_fanout_issue(outcomes);
    let (quality, retry_after_ms, last_error, problem) =
        aggregate_fanout_fields(operation, requested, rows, issue);
    MarketRuntimeHealth {
        venue: MARKET_AGGREGATE_VENUE.to_owned(),
        operation,
        quality,
        source,
        requested,
        rows,
        retry_after_ms,
        last_error,
        problem,
        observed_at_ms: common::time::now_ms(),
    }
}

pub(super) fn aggregate_fanout_fields(
    operation: &'static str,
    requested: u64,
    rows: u64,
    issue: Option<&FanoutVenueResult>,
) -> (
    MarketQuality,
    Option<u64>,
    Option<String>,
    Option<ExchangeProblem>,
) {
    if requested == 0 {
        return (
            MarketQuality::Missing,
            None,
            Some(format!("{operation} fanout has no registered venues")),
            None,
        );
    }
    if rows >= requested {
        return (MarketQuality::Fresh, None, None, None);
    }
    let quality = issue
        .and_then(|outcome| outcome.error.as_ref())
        .map(error_quality)
        .unwrap_or(MarketQuality::Missing);
    let retry_after_ms = issue
        .and_then(|outcome| outcome.error.as_ref())
        .and_then(retry_after_ms)
        .map(|ms| ms.max(0) as u64);
    let last_error = issue
        .and_then(|outcome| outcome.error.as_ref())
        .map(ToString::to_string)
        .or_else(|| {
            Some(format!(
                "{operation} partial fanout: {rows}/{requested} venues"
            ))
        });
    let problem = issue.and_then(|outcome| outcome.problem.clone());
    (quality, retry_after_ms, last_error, problem)
}

pub(super) fn aggregate_fanout_issue(outcomes: &[FanoutVenueResult]) -> Option<&FanoutVenueResult> {
    outcomes
        .iter()
        .find(|outcome| matches!(outcome.error, Some(ExchangeError::RateLimited { .. })))
        .or_else(|| {
            outcomes
                .iter()
                .find(|outcome| matches!(outcome.error, Some(ExchangeError::CircuitBreaker { .. })))
        })
        .or_else(|| outcomes.iter().find(|outcome| outcome.error.is_some()))
        .or_else(|| outcomes.iter().find(|outcome| outcome.rows == 0))
}

pub(super) fn runtime_success_fields(
    operation: &'static str,
    requested: usize,
    rows: usize,
) -> (MarketQuality, Option<String>) {
    if requested > 0 && rows < requested {
        (
            MarketQuality::Missing,
            Some(format!(
                "{operation} produced partial rows: {rows}/{requested}"
            )),
        )
    } else {
        (MarketQuality::Fresh, None)
    }
}

pub(super) fn cached_retry_after_ms<T>(entry: &CachedEntry<T>, now_ms: i64) -> Option<i64> {
    let until = entry.retry_after_until_ms?;
    (until > now_ms).then_some(until - now_ms)
}

pub(super) fn hit_ratio(hits: u64, misses: u64) -> f64 {
    let total = hits.saturating_add(misses);
    if total == 0 {
        0.0
    } else {
        hits as f64 / total as f64
    }
}
