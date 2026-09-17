use super::*;

#[test]
fn preview_backoff_suppresses_identical_query_until_expiry() {
    let query = preview_query_fixture();
    let problem = ApiProblem::new("RATE_LIMITED", "preview rate limited")
        .with_status(429)
        .with_retry_after_ms(Some(2_000));
    let backoff = Some(PreviewBackoff::new(&query, problem.clone(), 3_000.0));

    let active = active_backoff_problem(&backoff, &query, 2_999.0);
    assert_eq!(active, Some(problem));
    assert_eq!(active_backoff_problem(&backoff, &query, 3_000.0), None);
}

#[test]
fn preview_backoff_does_not_suppress_changed_query() {
    let query = preview_query_fixture();
    let mut changed = query.clone();
    changed.input.leverage = 3.0;
    let backoff = Some(PreviewBackoff::new(
        &query,
        ApiProblem::new("RATE_LIMITED", "preview rate limited")
            .with_status(429)
            .with_retry_after_ms(Some(2_000)),
        3_000.0,
    ));

    assert_eq!(active_backoff_problem(&backoff, &changed, 2_000.0), None);
}

#[test]
fn preview_backoff_uses_retry_after_or_short_429_fallback() {
    let explicit = ApiProblem::new("RATE_LIMITED", "slow").with_retry_after_ms(Some(2_500));
    let fallback = ApiProblem::new("RATE_LIMITED", "slow").with_status(429);
    let no_backoff = ApiProblem::new("PREVIEW_FAILED", "slow").with_status(503);
    let zero_retry = ApiProblem::new("RATE_LIMITED", "slow").with_retry_after_ms(Some(0));

    assert_eq!(preview_backoff_until_ms(&explicit, 10.0), Some(2_510.0));
    assert_eq!(preview_backoff_until_ms(&fallback, 10.0), Some(1_010.0));
    assert_eq!(preview_backoff_until_ms(&no_backoff, 10.0), None);
    assert_eq!(preview_backoff_until_ms(&zero_retry, 10.0), None);
}
