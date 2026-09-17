use super::super::audit_log::audit_outcome;
use super::*;
use axum::http::StatusCode;

const CORE_HIGH_RISK_KINDS: [ActionRunKind; 22] = [
    ActionRunKind::TradingRiskConfigUpdate,
    ActionRunKind::TradingAdapterSelect,
    ActionRunKind::TradingKillSwitch,
    ActionRunKind::TradingFeeSnapshotUpsert,
    ActionRunKind::TradingOrderSubmit,
    ActionRunKind::TradingOrderCancel,
    ActionRunKind::TradingOrderReconcile,
    ActionRunKind::AutomationConfigUpdate,
    ActionRunKind::AutomationControl,
    ActionRunKind::AutomationLiveUnlock,
    ActionRunKind::HedgeConfirm,
    ActionRunKind::WebhookConfigUpdate,
    ActionRunKind::VenueCredentialsUpdate,
    ActionRunKind::VenueCredentialsClear,
    ActionRunKind::VenueCredentialsMigrate,
    ActionRunKind::OnchainProviderCredentialsUpdate,
    ActionRunKind::OnchainProviderCredentialsClear,
    ActionRunKind::PortfolioClosePosition,
    ActionRunKind::PortfolioClosePair,
    ActionRunKind::PortfolioCloseAll,
    ActionRunKind::PortfolioCloseCompensation,
    ActionRunKind::PortfolioCloseManualTerminal,
];

#[derive(Clone, Copy)]
enum TerminalCase {
    Success,
    Denied,
    Error,
}

impl TerminalCase {
    const ALL: [Self; 3] = [Self::Success, Self::Denied, Self::Error];

    fn expected(self) -> (ActionRunStatus, &'static str) {
        match self {
            Self::Success => (ActionRunStatus::Succeeded, "success"),
            Self::Denied => (ActionRunStatus::Failed, "denied"),
            Self::Error => (ActionRunStatus::Failed, "error"),
        }
    }
}

#[tokio::test]
async fn core_high_risk_mutations_preserve_identity_across_terminal_outcomes() {
    let state = test_state().await;

    for (kind_index, kind) in CORE_HIGH_RISK_KINDS.into_iter().enumerate() {
        for (case_index, case) in TerminalCase::ALL.into_iter().enumerate() {
            let request_id = format!("req-{kind_index}-{case_index}");
            common::request_id::scope(request_id.clone(), async {
                assert_terminal_case(&state, kind, case, &request_id, kind_index, case_index);
            })
            .await;
        }
    }
}

fn assert_terminal_case(
    state: &AppState,
    kind: ActionRunKind,
    case: TerminalCase,
    request_id: &str,
    kind_index: usize,
    case_index: usize,
) {
    let idempotency_key = format!("idem-{kind_index}-{case_index}");
    let actor = format!("api-token:{kind_index:08x}{case_index:08x}");
    let claim = begin_idempotent(
        state,
        ActionRunStart {
            kind,
            actor: actor.clone(),
            target: Some(format!("target-{kind_index}")),
            idempotency_key: Some(idempotency_key.clone()),
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("initial idempotent begin failed: {error}"));
    assert!(!claim.is_replayed());
    finish_case(state, claim.run(), case);

    let stored = get(state, &claim.run().id).unwrap_or_else(|| panic!("missing run for {kind:?}"));
    let (expected_status, expected_outcome) = case.expected();
    assert_eq!(stored.status, expected_status, "kind={kind:?}");
    assert_ne!(stored.status, ActionRunStatus::Accepted, "kind={kind:?}");
    assert_eq!(audit_outcome(&stored), expected_outcome, "kind={kind:?}");
    assert_eq!(stored.request_id.as_deref(), Some(request_id));
    assert_eq!(
        stored.idempotency_key.as_deref(),
        Some(idempotency_key.as_str())
    );
    assert_eq!(stored.actor, actor);

    let replay = begin_idempotent(
        state,
        ActionRunStart {
            kind,
            actor: "replacement-actor".to_owned(),
            target: Some("replacement-target".to_owned()),
            idempotency_key: Some(idempotency_key),
            message: "replacement".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("replay idempotent begin failed: {error}"));
    assert!(replay.is_replayed());
    assert_eq!(replay.run().id, stored.id);
    assert_eq!(replay.run().request_id, stored.request_id);
    assert_eq!(replay.run().actor, stored.actor);
}

fn finish_case(state: &AppState, run: &ActionRun, case: TerminalCase) {
    match case {
        TerminalCase::Success => {
            finish_result_with_payload(state, &run.id, Ok(serde_json::json!({"ok": true})), "done")
                .unwrap_or_else(|error| panic!("success finalization failed: {error}"));
        }
        TerminalCase::Denied => {
            let result: Result<(), AppError> = fail_response(
                state,
                &run.id,
                AppError::domain(StatusCode::CONFLICT, "TEST_DENIED", "denied"),
            );
            assert!(result.is_err());
        }
        TerminalCase::Error => {
            let result: Result<(), AppError> = fail_response(
                state,
                &run.id,
                AppError::domain(StatusCode::BAD_GATEWAY, "TEST_ERROR", "error"),
            );
            assert!(result.is_err());
        }
    }
}
