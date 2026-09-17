use shared_types::{ApiProblem, ReviewEnvelope, VenueOperationHealth, VenueQualityEnvelope};

pub(super) trait ReviewRetryAfterSource {
    fn review_retry_after_ms(&self) -> Option<u64>;
}

impl<T> ReviewRetryAfterSource for ReviewEnvelope<T> {
    fn review_retry_after_ms(&self) -> Option<u64> {
        max_retry_after(
            self.problems
                .iter()
                .filter_map(|problem| problem.retry_after_ms)
                .max(),
            self.storage_health
                .as_ref()
                .and_then(venue_operation_retry_after_ms),
        )
    }
}

impl ReviewRetryAfterSource for VenueQualityEnvelope {
    fn review_retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
    }
}

pub(super) fn review_poll_allowed(retry_until_ms: Option<u64>, now_ms: u64) -> bool {
    retry_until_ms.is_none_or(|until_ms| now_ms >= until_ms)
}

pub(super) fn review_retry_deadline_for_result<T: ReviewRetryAfterSource>(
    result: &Result<T, ApiProblem>,
    now_ms: u64,
) -> Option<u64> {
    let retry_after_ms = match result {
        Ok(value) => value.review_retry_after_ms(),
        Err(problem) => problem.retry_after_ms,
    };
    review_retry_deadline_ms(retry_after_ms, now_ms)
}

fn venue_operation_retry_after_ms(health: &VenueOperationHealth) -> Option<u64> {
    max_retry_after(
        health.retry_after_ms,
        health
            .problem
            .as_ref()
            .and_then(|problem| problem.retry_after_ms),
    )
}

fn max_retry_after(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    left.into_iter().chain(right).max()
}

pub(super) fn review_retry_deadline_ms(retry_after_ms: Option<u64>, now_ms: u64) -> Option<u64> {
    retry_after_ms
        .filter(|retry_after_ms| *retry_after_ms > 0)
        .map(|retry_after_ms| now_ms.saturating_add(retry_after_ms))
}
