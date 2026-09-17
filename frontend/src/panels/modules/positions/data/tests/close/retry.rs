//! `CloseRun` 补偿撤销与人工恢复请求的行为测试。

use super::{attach_compensation_attempt, attach_manual_action, close_run};
use crate::panels::modules::positions::data::requests::{
    close_run_compensation_cancel_order_id, close_run_manual_terminal_request,
};
use crate::panels::modules::positions::data::{
    CloseRunCompensationCancelInput, CloseRunManualTerminalInput,
};
use shared_types::CloseRunStatus;

#[test]
fn compensation_cancel_order_id_must_belong_to_run() {
    let mut run = close_run(CloseRunStatus::CompensationSubmitted, 2, 1, 1250.0);
    attach_compensation_attempt(&mut run, "comp-order-1");
    let input = CloseRunCompensationCancelInput {
        run: run.clone(),
        order_id: "comp-order-1".to_owned(),
    };

    assert_eq!(
        close_run_compensation_cancel_order_id(&input).as_deref(),
        Ok("comp-order-1")
    );

    let input = CloseRunCompensationCancelInput {
        run,
        order_id: "external-order".to_owned(),
    };
    let result = close_run_compensation_cancel_order_id(&input);
    assert!(
        result.is_err(),
        "external compensation order must fail closed"
    );
    let error_source = result.err().and_then(|error| error.source);

    assert_eq!(
        error_source.as_deref(),
        Some("positions.close_run_compensation_cancel")
    );
}

#[test]
fn manual_terminal_request_requires_manual_action_and_reason() {
    let mut run = close_run(CloseRunStatus::CompensationFailed, 2, 1, 1250.0);
    attach_manual_action(&mut run);
    let input = CloseRunManualTerminalInput {
        run: run.clone(),
        confirmation_phrase: shared_types::CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE.to_owned(),
        reason: "account flat".to_owned(),
        evidence: "ticket-1, ticket-2\n ".to_owned(),
        manual_handling_cost_usd: "12.5".to_owned(),
    };

    let request_result = close_run_manual_terminal_request(&input);
    assert!(request_result.is_ok(), "{request_result:?}");
    let Ok(request) = request_result else {
        return;
    };

    assert_eq!(request.snapshot_version.as_deref(), Some("pos-1"));
    assert_eq!(request.reason, "account flat");
    assert_eq!(request.manual_handling_cost_usd, Some(12.5));
    assert_eq!(request.evidence, vec!["ticket-1", "ticket-2"]);

    let input = CloseRunManualTerminalInput {
        reason: " ".to_owned(),
        ..input
    };
    let result = close_run_manual_terminal_request(&input);
    assert!(result.is_err());
}

#[test]
fn manual_terminal_request_rejects_bad_cost_and_missing_action() {
    let mut run = close_run(CloseRunStatus::CompensationFailed, 2, 1, 1250.0);
    attach_manual_action(&mut run);
    let input = CloseRunManualTerminalInput {
        run: run.clone(),
        confirmation_phrase: shared_types::CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE.to_owned(),
        reason: "account flat".to_owned(),
        evidence: String::new(),
        manual_handling_cost_usd: "-1".to_owned(),
    };

    let result = close_run_manual_terminal_request(&input);

    assert!(result.is_err());

    let input = CloseRunManualTerminalInput {
        run: close_run(CloseRunStatus::CompensationFailed, 2, 1, 1250.0),
        manual_handling_cost_usd: "1".to_owned(),
        ..input
    };
    let result = close_run_manual_terminal_request(&input);
    assert!(result.is_err());
}
