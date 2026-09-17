use super::*;

#[test]
fn action_problem_summary_keeps_request_context() {
    let problem = ApiProblem::new("RATE_LIMITED", "slow down")
        .with_request_id(Some("req-1".into()))
        .with_retry_after_ms(Some(2_000));

    let summary = problem_summary(Some(problem));

    assert!(summary.contains("slow down"));
    assert!(summary.contains("request_id req-1"));
    assert!(summary.contains("retry 2000ms"));
}

#[test]
fn action_problem_detail_is_conditional_and_verbose() {
    assert_eq!(problem_detail(None), None);
    let problem = ApiProblem::new("RATE_LIMITED", "slow down")
        .with_status(429)
        .with_source("kucoin")
        .with_request_id(Some("req-1".into()))
        .with_retry_after_ms(Some(2_000));

    let detail = problem_detail(Some(problem));

    assert!(detail
        .as_ref()
        .is_some_and(|text| text.contains("RATE_LIMITED：slow down")));
    assert!(detail
        .as_ref()
        .is_some_and(|text| text.contains("HTTP 429")));
    assert!(detail
        .as_ref()
        .is_some_and(|text| text.contains("source kucoin")));
    assert!(detail
        .as_ref()
        .is_some_and(|text| text.contains("request_id req-1")));
    assert!(detail
        .as_ref()
        .is_some_and(|text| text.contains("retry 2000ms")));
}

#[test]
fn action_status_labels_are_readable() {
    assert_eq!(status_label(ActionRunStatus::Accepted), "处理中");
    assert_eq!(status_label(ActionRunStatus::Succeeded), "成功");
    assert_eq!(status_label(ActionRunStatus::Failed), "失败");
}

#[test]
fn credential_maintenance_action_labels_are_readable() {
    assert_eq!(kind_label(ActionRunKind::VenueCredentialsClear), "清空凭证");
    assert_eq!(
        kind_label(ActionRunKind::VenueCredentialsMigrate),
        "迁移凭证"
    );
}

#[test]
fn close_action_status_uses_close_run_payload_status() {
    let run = action_run_with_result(
        ActionRunKind::PortfolioClosePair,
        serde_json::json!({ "status": "submitted" }),
    );

    assert_eq!(status_label_for_run(&run), "已提交");
}

#[test]
fn close_action_status_exposes_unwind_payload_on_failed_action_run() {
    let mut run = action_run_with_result(
        ActionRunKind::PortfolioClosePair,
        serde_json::json!({ "status": "unwind_required" }),
    );
    run.status = ActionRunStatus::Failed;

    assert_eq!(status_label_for_run(&run), "需补偿");
}

#[test]
fn close_action_status_rejects_unknown_payload_status() {
    let run = action_run_with_result(
        ActionRunKind::PortfolioClosePair,
        serde_json::json!({ "status": "unknown_success" }),
    );

    assert_eq!(status_label_for_run(&run), "结果状态异常");
}

#[test]
fn close_action_status_reports_missing_payload_status() {
    let run = action_run_with_result(
        ActionRunKind::PortfolioClosePair,
        serde_json::json!({ "message": "done" }),
    );

    assert_eq!(status_label_for_run(&run), "结果状态缺失");
}

#[test]
fn close_action_status_reports_missing_result_only_for_succeeded_close() {
    let mut run = action_run_with_result(
        ActionRunKind::PortfolioClosePair,
        serde_json::json!({ "status": "succeeded" }),
    );
    run.result = None;

    assert_eq!(status_label_for_run(&run), "结果状态缺失");
    run.status = ActionRunStatus::Failed;
    assert_eq!(status_label_for_run(&run), "失败");
}

#[test]
fn non_close_action_status_keeps_action_run_status() {
    let run = action_run_with_result(
        ActionRunKind::TradingOrderSubmit,
        serde_json::json!({ "status": "submitted" }),
    );

    assert_eq!(status_label_for_run(&run), "成功");
}

#[test]
fn action_result_json_is_pretty_and_safe() {
    let text = result_json(Some(serde_json::json!({ "orderId": "o-1" })));

    assert!(text.contains("orderId"));
    assert_eq!(result_json(None), "-");
}

#[test]
fn action_mutation_detail_exposes_old_new_and_effective_time() {
    let detail = mutation_detail(Some(shared_types::ActionMutationDiff {
        effective_at_ms: 1,
        changes: vec![
            shared_types::ActionMutationChange::KillSwitchActive {
                before: false,
                after: true,
            },
            shared_types::ActionMutationChange::AllowedSymbols {
                before: vec!["BTCUSDT".into()],
                after: vec!["BTCUSDT".into(), "ETHUSDT".into()],
            },
            shared_types::ActionMutationChange::ProtectedPositions {
                before: Vec::new(),
                after: vec![shared_types::ProtectedPositionFingerprint {
                    venue: "binance".into(),
                    canonical_symbol: "btc".into(),
                    native_symbol: "btcusdt".into(),
                    side: "long".into(),
                    quantity: 0.232,
                    entry_price: 64_456.2,
                    position_mode: Some("both".into()),
                    opening_identity: "preexisting-binance-btc-long".into(),
                    source: "account_position_runtime".into(),
                    captured_at_ms: 42,
                }],
            },
        ],
    }))
    .unwrap_or_default();

    assert!(detail.contains("Kill Switch false -> true"));
    assert!(detail.contains("BTCUSDT -> BTCUSDT,ETHUSDT"));
    assert!(detail.contains("受保护持仓 无 -> binance:btcusdt:long"));
    assert!(detail.contains("preexisting-binance-btc-long"));
    assert!(detail.contains("生效"));
}

fn action_run_with_result(kind: ActionRunKind, result: serde_json::Value) -> ActionRun {
    ActionRun {
        id: "act-1".to_owned(),
        kind,
        status: ActionRunStatus::Succeeded,
        actor: "127.0.0.1".to_owned(),
        target: None,
        request_id: None,
        idempotency_key: None,
        message: "done".to_owned(),
        problem: None,
        result: Some(result),
        mutation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    }
}
