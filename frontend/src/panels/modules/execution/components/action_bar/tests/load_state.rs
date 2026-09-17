use super::*;

#[test]
fn action_detail_keeps_request_context() {
    let state = ActionState::failed(
        "提交失败",
        ApiProblem::new("RATE_LIMITED", "slow")
            .with_source("trade-rest")
            .with_status(429)
            .with_request_id(Some("req-1".into()))
            .with_retry_after_ms(Some(2_000)),
    );

    let detail = action_detail(&state, "BTC", None);

    assert!(detail.contains("HTTP 429"));
    assert!(detail.contains("code RATE_LIMITED"));
    assert!(detail.contains("source trade-rest"));
    assert!(detail.contains("request_id req-1"));
    assert!(detail.contains("retry 2000ms"));
}

#[test]
fn successful_action_detail_keeps_machine_evidence_out_of_primary_copy() {
    let state = ActionState::succeeded("双腿提交完成").with_evidence(
        shared_types::ActionEvidence::client_request("req-machine", Some("idem-machine".into()))
            .with_run_id(Some("run-machine".into())),
    );

    let detail = action_detail(&state, "BTC", None);

    assert_eq!(detail, "双腿提交完成 · BTC");
    assert!(!detail.contains("machine"));
}

#[test]
fn pending_action_detail_keeps_request_and_order_evidence_visible() {
    let state = ActionState::pending("模拟提交中").with_evidence(
        shared_types::ActionEvidence::client_request("req-pending", Some("idem-pending".into()))
            .with_client_order_ids(["client-order-1".into()]),
    );

    let detail = action_detail(&state, "BTC", None);

    assert!(detail.contains("request_id req-pending"));
    assert!(detail.contains("idempotency idem-pending"));
    assert!(detail.contains("client_order_id client-order-1"));
}

#[test]
fn accepted_action_detail_keeps_run_evidence_visible_until_finality() {
    let state = ActionState::accepted("等待终态").with_evidence(
        shared_types::ActionEvidence::default().with_run_id(Some("run-pending".into())),
    );

    let detail = action_detail(&state, "BTC", None);

    assert!(detail.contains("run_id run-pending"));
}

#[test]
fn idle_action_detail_surfaces_preview_problem_context() {
    let problem = ApiProblem::new("RATE_LIMITED", "preview slow")
        .with_source("preview-rest")
        .with_status(429)
        .with_request_id(Some("preview-req-1".into()))
        .with_retry_after_ms(Some(2_000));

    let detail = action_detail(&ActionState::Idle, "BTC", Some(&problem));

    assert!(detail.contains("预览阻断：preview slow"));
    assert!(detail.contains("code RATE_LIMITED"));
    assert!(detail.contains("source preview-rest"));
    assert!(detail.contains("HTTP 429"));
    assert!(detail.contains("request_id preview-req-1"));
    assert!(detail.contains("retry 2000ms"));
}

#[test]
fn preview_blocked_state_prefers_typed_preview_problem() {
    let state = preview_blocked_state(Some(
        ApiProblem::new("RATE_LIMITED", "preview slow")
            .with_status(429)
            .with_request_id(Some("preview-req-1".into()))
            .with_retry_after_ms(Some(2_000)),
    ));

    assert_eq!(
        state.problem().map(|problem| problem.code.as_str()),
        Some("RATE_LIMITED")
    );
    let detail = action_detail(&state, "BTC", None);
    assert!(detail.contains("预览阻断：preview slow"));
    assert!(detail.contains("request_id preview-req-1"));
    assert!(!detail.contains("HEDGE_PREVIEW_NOT_READY"));
}

#[test]
fn can_submit_requires_ready_preview_load_state() {
    let ready = ready_preview();
    let stale = LoadState::Stale {
        value: ready.clone(),
        problem: ApiProblem::new("TIMEOUT", "preview timeout"),
    };
    let error = LoadState::Error(ApiProblem::new("RATE_LIMITED", "preview slow"));

    assert!(ready_preview_can_submit(&LoadState::Ready(ready), 1));
    assert!(!ready_preview_can_submit(&stale, 1));
    assert!(!ready_preview_can_submit(&error, 1));
}
