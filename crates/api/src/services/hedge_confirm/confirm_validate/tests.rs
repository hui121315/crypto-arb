use super::test_fixtures::{attach_sizing_contract, hyperliquid_plan};
use super::*;
use shared_types::hedge::HedgeTicketOrderPlans;
use shared_types::{HedgeLegRole, HedgePreflightStatus, MarginPreflightOutcome, VenueOrderKind};

#[test]
fn confirm_requires_explicit_non_empty_ticket_identity() -> Result<(), String> {
    for ticket_id in [None, Some(String::new()), Some("  ".to_owned())] {
        let request = HedgeConfirmRequest {
            idempotency_key: "confirm-required-ticket".to_owned(),
            ticket_id,
        };
        let error = required_confirm_ticket_id(&request)
            .err()
            .ok_or_else(|| "missing ticket identity must fail closed".to_owned())?;

        assert_eq!(error.code(), codes::HEDGE_TICKET_REQUIRED);
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    let request = HedgeConfirmRequest {
        idempotency_key: "confirm-required-ticket".to_owned(),
        ticket_id: Some(" ticket-1 ".to_owned()),
    };
    let ticket_id = required_confirm_ticket_id(&request).map_err(|error| error.to_string())?;
    assert_eq!(ticket_id, "ticket-1");
    Ok(())
}

#[test]
fn confirm_rejects_legacy_preview_without_ticket_order_plan_evidence() -> Result<(), String> {
    let error = validate_ticket_order_plans("ticket-legacy", None)
        .err()
        .ok_or_else(|| "legacy preview must fail closed".to_owned())?;

    assert_eq!(error.code(), codes::HEDGE_TICKET_BLOCKED);
    assert!(error
        .to_string()
        .contains("missing ticket-bound order plan evidence"));
    Ok(())
}

#[test]
fn confirm_uses_ticket_bound_hyperliquid_builder_plans() -> Result<(), String> {
    let plans = HedgeTicketOrderPlans::from_compile_plans(
        "ticket-hyperliquid",
        hyperliquid_plan(HedgeLegRole::Long),
        hyperliquid_plan(HedgeLegRole::Short),
    )
    .map_err(|error| format!("ticket plans must be valid: {error}"))?;
    let [long, short] = validate_ticket_order_plans("ticket-hyperliquid", Some(&plans))
        .map_err(|error| format!("ticket plans must pass confirmation: {error}"))?;

    assert_eq!(long.compile_plan.role, HedgeLegRole::Long);
    assert_eq!(short.compile_plan.role, HedgeLegRole::Short);
    assert_eq!(long.compile_plan.exchange, "hyperliquid:xyz");
    assert_eq!(
        long.compile_plan.venue_order_kind,
        VenueOrderKind::ProtectedIoc
    );
    assert_eq!(
        long.identity_plan
            .client_order_id_policy
            .venue_client_order_id
            .as_deref(),
        Some("0x00000000000000000000000000000001")
    );
    Ok(())
}

#[test]
fn live_confirm_requires_ticket_bound_instrument_sizing_contracts() -> Result<(), String> {
    let missing = HedgeTicketOrderPlans::from_compile_plans(
        "ticket-missing-sizing",
        hyperliquid_plan(HedgeLegRole::Long),
        hyperliquid_plan(HedgeLegRole::Short),
    )
    .map_err(|error| error.to_string())?;
    let missing_plans = missing
        .plans_for_ticket("ticket-missing-sizing")
        .map_err(|error| error.to_string())?;
    let error = validate_live_sizing_contracts(missing_plans)
        .err()
        .ok_or_else(|| "missing sizing must fail closed".to_owned())?;
    assert!(error.to_string().contains("INSTRUMENT_SPEC_MISSING"));

    let mut long = hyperliquid_plan(HedgeLegRole::Long);
    let mut short = hyperliquid_plan(HedgeLegRole::Short);
    attach_sizing_contract(&mut long)?;
    attach_sizing_contract(&mut short)?;
    let complete = HedgeTicketOrderPlans::from_compile_plans("ticket-complete-sizing", long, short)
        .map_err(|error| error.to_string())?;
    let complete_plans = complete
        .plans_for_ticket("ticket-complete-sizing")
        .map_err(|error| error.to_string())?;
    validate_live_sizing_contracts(complete_plans)
        .map_err(|error| format!("complete sizing must pass: {error}"))
}

#[test]
fn confirm_scoped_preflight_returns_every_blocked_guard() -> Result<(), String> {
    let guards = vec![
        ExecutionGuard {
            key: "order_capability".to_owned(),
            label: "交易所下单能力".to_owned(),
            passed: true,
            detail: "通过".to_owned(),
            preflight_outcome: None,
        },
        blocked_guard("account_mode", "账户模式不可读"),
        blocked_guard("live_operation_health", "订单终态回查缺证据"),
    ];

    let error = validate_confirm_preflight_guards(guards)
        .err()
        .ok_or_else(|| "blocked confirm preflight must fail".to_owned())?;
    assert_eq!(error.code(), codes::HEDGE_PRE_TRADE_REJECTED);
    assert!(error.to_string().contains("账户模式不可读"));
    assert!(error.to_string().contains("订单终态回查缺证据"));
    match error {
        AppError::Domain { details, .. } => {
            let count = details
                .and_then(|value| value.get("guards").cloned())
                .and_then(|value| value.as_array().map(Vec::len));
            assert_eq!(count, Some(2));
        }
        _ => return Err("expected a typed domain error".to_owned()),
    }
    Ok(())
}

fn blocked_guard(key: &str, detail: &str) -> ExecutionGuard {
    ExecutionGuard {
        key: key.to_owned(),
        label: key.to_owned(),
        passed: false,
        detail: detail.to_owned(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: HedgePreflightStatus::Blocked,
            ..MarginPreflightOutcome::default()
        }),
    }
}
