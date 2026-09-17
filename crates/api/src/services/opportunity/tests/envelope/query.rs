use super::*;

#[test]
fn wide_limit_defaults_and_clamps_with_problem() {
    assert_eq!(wide_limit(None).value(), DEFAULT_WIDE_LIMIT);
    assert_eq!(wide_limit(Some(25)).value(), 25);

    let limit = wide_limit(Some(MAX_WIDE_LIMIT + 1));
    assert_eq!(limit.value(), MAX_WIDE_LIMIT);

    let problems = limit.into_query_problems();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, codes::LIST_LIMIT_CLAMPED);
    assert_eq!(
        problems[0].source.as_deref(),
        Some("arbitrage-opportunities")
    );
    assert_eq!(
        problems[0]
            .details
            .as_ref()
            .and_then(|details| details.get("applied"))
            .and_then(serde_json::Value::as_u64),
        Some(MAX_WIDE_LIMIT as u64)
    );
}

#[test]
fn envelope_includes_query_problem_and_degrades() {
    let cached_at = Utc::now();
    let query_problems = wide_limit(Some(MAX_WIDE_LIMIT + 1)).into_query_problems();

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta: OpportunityScanMeta::default(),
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems,
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Degraded);
    assert!(env
        .partial_failures
        .iter()
        .any(|problem| problem.code == codes::LIST_LIMIT_CLAMPED));
}
