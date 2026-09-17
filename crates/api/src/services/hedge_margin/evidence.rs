use super::*;

#[derive(Debug, Clone)]
pub(super) struct MarginEvidence {
    pub(super) outcome: MarginPreflightOutcome,
    pub(super) account_state: AccountStateSnapshot,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MarginBalanceEvidence {
    pub(super) freshness_ms: Option<u64>,
    pub(super) retry_after_ms: Option<u64>,
    pub(super) request_id: Option<String>,
    pub(super) operation_health: Vec<VenueOperationHealth>,
    pub(super) account_summaries: Vec<VenueAccountSummary>,
    pub(super) account_bindings: Vec<AccountBindingEvidence>,
    pub(super) collateral_rows: Vec<VenueBalanceInfo>,
}

pub(super) fn margin_evidence(
    status: HedgePreflightStatus,
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: MarginBalanceEvidence,
    error: Option<String>,
) -> MarginEvidence {
    let account_summaries = evidence.account_summaries.clone();
    let account_bindings = evidence.account_bindings.clone();
    let collateral_rows = evidence.collateral_rows.clone();
    let outcome = margin_outcome(status, intents, balances, evidence, error);
    let account_state = scoped_margin_account_state(
        balances,
        &collateral_rows,
        account_summaries,
        account_bindings,
        &outcome,
    );
    MarginEvidence {
        outcome,
        account_state,
    }
}

pub(super) fn margin_balance_evidence(
    state: &AppState,
    venues: &[String],
    collateral_rows: &[VenueBalanceInfo],
    error: Option<&AppError>,
) -> MarginBalanceEvidence {
    let mut evidence = margin_problem_evidence(error);
    evidence.freshness_ms = max_scoped_balance_freshness(state, venues);
    let operation_health =
        scoped_margin_operation_health(venues, &venue_operation_health::snapshot(state).rows);
    merge_operation_health_evidence(&mut evidence, venues, &operation_health);
    evidence.operation_health = operation_health;
    evidence.account_summaries = state
        .trading_service()
        .account_summaries()
        .into_iter()
        .filter(|row| {
            venues
                .iter()
                .any(|venue| margin_balance_venue_matches(&row.venue, venue))
        })
        .collect();
    evidence.account_bindings =
        account_binding::evidence_for_venues(venues.iter().cloned(), common::time::now_ms());
    evidence.collateral_rows = scoped_collateral_rows(collateral_rows, venues);
    evidence
}

pub(super) fn margin_problem_evidence(error: Option<&AppError>) -> MarginBalanceEvidence {
    let Some(error) = error else {
        return MarginBalanceEvidence::default();
    };
    MarginBalanceEvidence {
        freshness_ms: None,
        retry_after_ms: error.retry_after_ms(),
        request_id: common::request_id::current(),
        operation_health: Vec::new(),
        account_summaries: Vec::new(),
        account_bindings: Vec::new(),
        collateral_rows: Vec::new(),
    }
}

pub(super) fn with_margin_evidence(error: AppError, evidence: &MarginEvidence) -> AppError {
    let outcome = json!(&evidence.outcome);
    let account_state = json!(&evidence.account_state);
    match error {
        AppError::Domain {
            status,
            code,
            message,
            details,
        } => AppError::Domain {
            status,
            code,
            message,
            details: Some(merge_margin_details(details, outcome, account_state)),
        },
        other => {
            let status = other.status();
            let code = other.code();
            let message = other.to_string();
            AppError::domain(status, code, message).with_details(json!({
                "preflightOutcome": outcome,
                "accountState": account_state,
            }))
        }
    }
}

pub(super) fn merge_margin_details(
    details: Option<Value>,
    outcome: Value,
    account_state: Value,
) -> Value {
    match details {
        Some(Value::Object(mut map)) => {
            map.insert("preflightOutcome".to_owned(), outcome);
            map.insert("accountState".to_owned(), account_state);
            Value::Object(map)
        }
        Some(value) => json!({
            "details": value,
            "preflightOutcome": outcome,
            "accountState": account_state,
        }),
        None => json!({
            "preflightOutcome": outcome,
            "accountState": account_state,
        }),
    }
}
