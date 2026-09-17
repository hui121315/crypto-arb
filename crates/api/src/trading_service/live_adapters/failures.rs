use super::*;

/// A single venue's failure during a tolerant multi-venue fanout read.
///
/// The fanout (`get_positions`/`get_balances`) keeps serving the venues that
/// succeeded, but records the ones that failed here so callers can surface a
/// per-venue `RuntimeProblem` instead of silently dropping the failure into a
/// log line.
pub(crate) struct RouteFailure {
    pub(crate) venue: String,
    pub(crate) operation: &'static str,
    pub(crate) error: common::AppError,
    cached_error: super::super::CachedExchangeError,
}

impl RouteFailure {
    pub(crate) fn new(venue: String, operation: &'static str, error: ExchangeError) -> Self {
        let cached_error = super::super::CachedExchangeError::from(&error);
        Self {
            venue,
            operation,
            error: common::AppError::from(error),
            cached_error,
        }
    }

    fn exchange_error(&self) -> ExchangeError {
        self.cached_error.to_exchange_error()
    }

    pub(super) fn from_account_read_issue(issue: exchange::VenueAccountReadIssue) -> Self {
        Self::new(issue.venue, issue.operation, issue.error)
    }

    pub(crate) fn to_api_problem(
        &self,
        api_source: &str,
        api_path: &str,
    ) -> shared_types::ApiProblem {
        let endpoint = route_failure_endpoint(&self.venue, self.operation);
        let mut problem = self.error.to_api_problem();
        problem.source = Some(endpoint.as_ref().map_or_else(
            || api_source.to_owned(),
            |row| {
                format!(
                    "{}.{} {}",
                    shared_types::normalized_venue_name(shared_types::venue_family(&self.venue)),
                    row.method,
                    row.path
                )
            },
        ));
        merge_route_context(&mut problem, self, api_source, api_path, endpoint.as_ref());
        problem
    }
}

/// Per-operation sink shared between the dispatch router and `TradingService`.
///
/// Each fanout read replaces its operation's bucket (an all-success read clears
/// it), so a `take` reflects the most recent read rather than accumulating.
#[derive(Default)]
pub(crate) struct RouteFailureSink {
    by_operation: Mutex<BTreeMap<&'static str, Vec<RouteFailure>>>,
}

impl RouteFailureSink {
    pub(crate) fn record(&self, operation: &'static str, failures: Vec<RouteFailure>) {
        let mut guard = self.by_operation.lock();
        if failures.is_empty() {
            guard.remove(operation);
        } else {
            guard.insert(operation, failures);
        }
    }

    pub(crate) fn take(&self, operation: &str) -> Vec<RouteFailure> {
        self.by_operation
            .lock()
            .remove(operation)
            .unwrap_or_default()
    }

    pub(crate) fn take_for_venues(&self, operation: &str, venues: &[String]) -> Vec<RouteFailure> {
        let requested = venues
            .iter()
            .map(|venue| shared_types::normalized_venue_name(shared_types::venue_family(venue)))
            .collect::<BTreeSet<_>>();
        if requested.is_empty() {
            return Vec::new();
        }

        let mut guard = self.by_operation.lock();
        let Some(failures) = guard.remove(operation) else {
            return Vec::new();
        };
        let key = failures.first().map(|failure| failure.operation);
        let (matched, retained): (Vec<_>, Vec<_>) = failures.into_iter().partition(|failure| {
            requested.contains(&shared_types::normalized_venue_name(
                shared_types::venue_family(&failure.venue),
            ))
        });
        if !retained.is_empty() {
            if let Some(key) = key {
                guard.insert(key, retained);
            }
        }
        matched
    }

    #[cfg(test)]
    pub(crate) fn venues(&self, operation: &str) -> Vec<String> {
        let mut venues = self
            .by_operation
            .lock()
            .get(operation)
            .into_iter()
            .flatten()
            .map(|failure| failure.venue.clone())
            .collect::<Vec<_>>();
        venues.sort_unstable();
        venues.dedup();
        venues
    }

    pub(crate) fn exchange_errors(&self, operation: &str) -> Vec<(String, ExchangeError)> {
        self.by_operation
            .lock()
            .get(operation)
            .into_iter()
            .flatten()
            .map(|failure| (failure.venue.clone(), failure.exchange_error()))
            .collect()
    }
}

fn route_failure_endpoint(venue: &str, operation: &str) -> Option<shared_types::RestEndpointRow> {
    let (use_case, data_kind) = match operation {
        "positions" => ("private_read", "account_position"),
        "balances" => ("private_read", "account_balance"),
        "open_orders" => ("private_read", "order_status"),
        _ => return None,
    };
    let family = shared_types::normalized_venue_name(shared_types::venue_family(venue));
    exchange::rest_endpoint_registry()
        .venues
        .into_iter()
        .find(|row| shared_types::normalized_venue_name(&row.venue) == family)?
        .endpoints
        .into_iter()
        .find(|row| {
            row.use_cases.iter().any(|value| value == use_case)
                && row.data_kinds.iter().any(|value| value == data_kind)
        })
}

fn merge_route_context(
    problem: &mut shared_types::ApiProblem,
    failure: &RouteFailure,
    api_source: &str,
    api_path: &str,
    endpoint: Option<&shared_types::RestEndpointRow>,
) {
    let mut details = match problem.details.take() {
        Some(serde_json::Value::Object(details)) => details,
        Some(upstream) => {
            let mut details = serde_json::Map::new();
            details.insert("upstreamDetails".to_owned(), upstream);
            details
        }
        None => serde_json::Map::new(),
    };
    details.insert("venue".to_owned(), serde_json::json!(failure.venue));
    details.insert("operation".to_owned(), serde_json::json!(failure.operation));
    details.insert("apiPath".to_owned(), serde_json::json!(api_path));
    details.insert("status".to_owned(), serde_json::json!(problem.status));
    details.insert("source".to_owned(), serde_json::json!(api_source));
    if let Some(endpoint) = endpoint {
        details.insert("method".to_owned(), serde_json::json!(endpoint.method));
        details.insert("path".to_owned(), serde_json::json!(endpoint.path));
        details.insert(
            "fixtureId".to_owned(),
            serde_json::json!(endpoint.fixture_id),
        );
        details.insert(
            "schemaHash".to_owned(),
            serde_json::json!(endpoint.schema_hash),
        );
        if let Some(doc_url) = endpoint.doc_urls.first() {
            details.insert("docUrl".to_owned(), serde_json::json!(doc_url));
        }
    } else {
        details.insert("path".to_owned(), serde_json::json!(api_path));
    }
    problem.details = Some(serde_json::Value::Object(details));
}
