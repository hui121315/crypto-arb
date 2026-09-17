use super::*;

#[test]
fn mutation_context_reuses_exact_request_identity_in_evidence() {
    let context = MutationRequestContext::with_idempotency_key("idem-transport");
    let evidence = context.evidence();

    assert_eq!(evidence.request_id.as_deref(), Some(context.request_id()));
    assert_eq!(context.idempotency_key(), Some("idem-transport"));
    assert_eq!(evidence.idempotency_key.as_deref(), Some("idem-transport"));
}

#[test]
fn idempotent_attempt_is_stable_within_request_and_unique_across_retries() {
    let first = MutationRequestContext::new_idempotent_attempt("execution-cancel:run-1");
    let second = MutationRequestContext::new_idempotent_attempt("execution-cancel:run-1");

    assert_ne!(first.request_id(), second.request_id());
    assert_ne!(first.idempotency_key(), second.idempotency_key());
    assert!(first
        .idempotency_key()
        .is_some_and(|key| key.starts_with("execution-cancel:run-1:web-")));
    assert_eq!(
        first.evidence().idempotency_key.as_deref(),
        first.idempotency_key()
    );
}
