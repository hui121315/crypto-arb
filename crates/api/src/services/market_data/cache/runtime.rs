use super::*;

impl MarketDataCache {
    pub(crate) fn record_runtime_success(
        &self,
        venue: &str,
        operation: &'static str,
        source: MarketSource,
        requested: usize,
        rows: usize,
    ) {
        self.clear_ws_warmup(venue, operation);
        let (quality, last_error) = runtime_success_fields(operation, requested, rows);
        self.upsert_runtime_health(MarketRuntimeHealth {
            venue: venue.to_owned(),
            operation,
            quality,
            source,
            requested: requested as u64,
            rows: rows as u64,
            retry_after_ms: None,
            last_error,
            problem: None,
            observed_at_ms: common::time::now_ms(),
        });
    }

    pub(crate) fn record_runtime_error(
        &self,
        venue: &str,
        operation: &'static str,
        source: MarketSource,
        requested: usize,
        error: &ExchangeError,
    ) {
        self.clear_ws_warmup(venue, operation);
        self.upsert_runtime_health(runtime_error_health(
            venue,
            operation,
            source,
            requested,
            error,
            error.to_problem(venue, operation),
        ));
    }

    pub(crate) fn record_runtime_symbol_error(
        &self,
        venue: &str,
        operation: &'static str,
        source: MarketSource,
        symbol: &str,
        error: &ExchangeError,
    ) {
        self.upsert_runtime_health(runtime_error_health(
            venue,
            operation,
            source,
            1,
            error,
            error.to_problem(venue, operation).with_symbol(symbol),
        ));
    }

    pub(crate) fn record_fanout_outcomes(
        &self,
        source: MarketSource,
        outcomes: Vec<FanoutVenueResult>,
    ) {
        let observed_at_ms = common::time::now_ms();
        for mut outcome in outcomes {
            let (quality, retry_after_ms, last_error) = fanout_health_fields(&outcome);
            let problem = outcome
                .problem
                .take()
                .or_else(|| {
                    outcome
                        .error
                        .as_ref()
                        .map(|error| error.to_problem(&outcome.venue, outcome.operation))
                })
                .map(|problem| {
                    let latency_ms = problem.latency_ms.or(Some(outcome.latency_ms));
                    problem.with_latency_ms(latency_ms)
                });
            self.upsert_runtime_health(MarketRuntimeHealth {
                venue: outcome.venue,
                operation: outcome.operation,
                quality,
                source,
                requested: 1,
                rows: outcome.rows as u64,
                retry_after_ms,
                last_error,
                problem,
                observed_at_ms,
            });
        }
    }

    pub(crate) fn record_aggregate_fanout_outcome(
        &self,
        operation: &'static str,
        source: MarketSource,
        outcomes: &[FanoutVenueResult],
    ) {
        self.upsert_runtime_health(aggregate_fanout_health(operation, source, outcomes));
    }

    pub(crate) fn record_runtime_unsupported(
        &self,
        venue: &str,
        operation: &'static str,
        source: MarketSource,
        requested: usize,
        reason: impl Into<String>,
    ) {
        self.clear_ws_warmup(venue, operation);
        self.upsert_runtime_health(MarketRuntimeHealth {
            venue: venue.to_owned(),
            operation,
            quality: MarketQuality::Unsupported,
            source,
            requested: requested as u64,
            rows: 0,
            retry_after_ms: None,
            last_error: Some(reason.into()),
            problem: None,
            observed_at_ms: common::time::now_ms(),
        });
    }
}

fn runtime_error_health(
    venue: &str,
    operation: &'static str,
    source: MarketSource,
    requested: usize,
    error: &ExchangeError,
    problem: ExchangeProblem,
) -> MarketRuntimeHealth {
    MarketRuntimeHealth {
        venue: venue.to_owned(),
        operation,
        quality: error_quality(error),
        source,
        requested: requested as u64,
        rows: 0,
        retry_after_ms: retry_after_ms(error).map(|ms| ms.max(0) as u64),
        last_error: Some(error.to_string()),
        problem: Some(problem),
        observed_at_ms: common::time::now_ms(),
    }
}
