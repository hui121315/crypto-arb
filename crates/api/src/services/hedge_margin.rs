use crate::services::{account_binding, account_state, venue_operation_health};
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use serde_json::{json, Value};
use shared_types::{
    normalized_venue_name, problem::codes, AccountBindingEvidence, AccountDataHealth,
    AccountEquityScope, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubject,
    AccountStateSnapshot, ApiProblem, ExecutionGuard, ExecutionMode, HedgePreflightOperation,
    HedgePreflightScope, HedgePreflightStatus, HedgePreviewResponse, ListStatus, MarginMode,
    MarginPreflightOutcome, OrderIntent, VenueAccountSummary, VenueBalanceEnvelope,
    VenueBalanceInfo, VenueOperationHealth, VenueOperationKind, VenueOperationStatus,
    VenuePositionEnvelope,
};
use trading::TradingError;

const MARGIN_BALANCE_SOURCE: &str = "account_state.margin_facts";
const MARGIN_BALANCE_OPERATION: &str = "margin_balance_read";

mod collateral;
mod evidence;
mod health;
mod problems;
#[cfg(test)]
mod tests;
mod venues;

use collateral::*;
use evidence::*;
use health::*;
use problems::*;
use venues::*;

pub(crate) async fn preview_margin_guard(
    state: &AppState,
    intents: &[&OrderIntent],
) -> Result<ExecutionGuard, AppError> {
    if !intents.iter().any(|intent| requires_live_margin(intent)) {
        return Ok(guard_with_preflight(
            "margin_balance",
            "保证金余额",
            true,
            "Paper 模式无需保证金检查",
            margin_outcome(
                HedgePreflightStatus::Skipped,
                intents,
                &[],
                MarginBalanceEvidence::default(),
                None,
            ),
        ));
    }
    let venues = required_margin_venues(intents);
    let balances = match state.trading_service().list_scoped_balances(&venues).await {
        Ok(balances) => balances,
        Err(error) => {
            let detail = format!("保证金余额读取失败: {error}");
            return Ok(guard_with_preflight(
                "margin_balance",
                "保证金余额",
                false,
                &detail,
                margin_outcome(
                    HedgePreflightStatus::Failed,
                    intents,
                    &[],
                    MarginBalanceEvidence::default(),
                    Some(detail.clone()),
                ),
            ));
        }
    };
    let evidence = margin_balance_evidence(state, &venues, &balances, None);
    let balances = account_margin_rows(&balances, intents, &evidence.account_summaries);
    let missing = missing_margin_venues(&balances, intents);
    if !missing.is_empty() {
        let detail = format!("保证金余额缺少目标交易所数据: {}", missing.join(", "));
        return Ok(guard_with_preflight(
            "margin_balance",
            "保证金余额",
            false,
            &detail,
            margin_outcome(
                HedgePreflightStatus::Blocked,
                intents,
                &balances,
                evidence,
                Some(detail.clone()),
            ),
        ));
    }
    if let Some(problem) =
        margin_balance_evidence_problem(intents, &evidence, common::time::now_ms())
    {
        return Ok(blocked_margin_evidence_guard(
            intents,
            &balances,
            evidence,
            &problem.message,
        ));
    }
    Ok(
        match trading::execution::ensure_sufficient_margin_for_intents(&balances, intents) {
            Ok(()) => guard_with_preflight(
                "margin_balance",
                "保证金余额",
                true,
                "通过",
                margin_outcome(
                    HedgePreflightStatus::Passed,
                    intents,
                    &balances,
                    evidence,
                    None,
                ),
            ),
            Err(error) => {
                let detail = trading_error_text(&error);
                guard_with_preflight(
                    "margin_balance",
                    "保证金余额",
                    false,
                    &detail,
                    margin_outcome(
                        HedgePreflightStatus::Blocked,
                        intents,
                        &balances,
                        evidence,
                        Some(detail.clone()),
                    ),
                )
            }
        },
    )
}

fn blocked_margin_evidence_guard(
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: MarginBalanceEvidence,
    detail: &str,
) -> ExecutionGuard {
    let preflight = margin_outcome(
        HedgePreflightStatus::Blocked,
        intents,
        balances,
        evidence,
        Some(detail.to_owned()),
    );
    guard_with_preflight("margin_balance", "保证金余额", false, detail, preflight)
}

fn requires_live_margin(intent: &OrderIntent) -> bool {
    matches!(intent.mode, ExecutionMode::Live | ExecutionMode::Testnet) && !intent.reduce_only
}

pub(crate) async fn ensure_final_margin(
    state: &AppState,
    preview: &mut HedgePreviewResponse,
) -> Result<(), AppError> {
    let intents = [&preview.long_leg, &preview.short_leg];
    if !intents.iter().any(|intent| requires_live_margin(intent)) {
        let guard = final_margin_success_guard(
            HedgePreflightStatus::Skipped,
            "确认终检跳过：Paper 模式无需保证金检查",
            &intents,
            &[],
            MarginBalanceEvidence::default(),
        );
        return crate::services::hedge_preview::apply_final_margin_guard(preview, guard);
    }
    let venues = required_margin_venues(&intents);
    let balances = match state
        .trading_service()
        .refresh_scoped_balances(&venues)
        .await
    {
        Ok(balances) => balances,
        Err(error) => {
            let detail = format!("保证金余额读取失败: {error}");
            let app_error = balance_read_error(&error, &venues);
            let balance_evidence = margin_balance_evidence(state, &venues, &[], Some(&app_error));
            let margin_evidence = margin_evidence(
                HedgePreflightStatus::Failed,
                &intents,
                &[],
                balance_evidence,
                Some(detail),
            );
            return Err(with_margin_evidence(app_error, &margin_evidence));
        }
    };
    let balance_evidence = margin_balance_evidence(state, &venues, &balances, None);
    let balances = account_margin_rows(&balances, &intents, &balance_evidence.account_summaries);
    if let Err(error) = ensure_required_venue_balances(&balances, &intents) {
        let missing = missing_margin_venues(&balances, &intents);
        let detail = format!("保证金余额缺少目标交易所数据: {}", missing.join(", "));
        let evidence = margin_evidence(
            HedgePreflightStatus::Blocked,
            &intents,
            &balances,
            balance_evidence,
            Some(detail),
        );
        return Err(with_margin_evidence(error, &evidence));
    }
    if let Some(problem) =
        margin_balance_evidence_problem(&intents, &balance_evidence, common::time::now_ms())
    {
        let error = AppError::domain(
            StatusCode::BAD_GATEWAY,
            codes::BALANCE_READ_DEGRADED,
            problem.message.clone(),
        )
        .with_details(json!({"problem": problem}));
        let evidence = margin_evidence(
            HedgePreflightStatus::Blocked,
            &intents,
            &balances,
            balance_evidence,
            Some(error.to_string()),
        );
        return Err(with_margin_evidence(error, &evidence));
    }
    match trading::execution::ensure_sufficient_margin_for_intents(&balances, &intents) {
        Ok(()) => {
            let guard = final_margin_success_guard(
                HedgePreflightStatus::Passed,
                "确认终检通过",
                &intents,
                &balances,
                balance_evidence,
            );
            crate::services::hedge_preview::apply_final_margin_guard(preview, guard)
        }
        Err(error) => {
            let detail = trading_error_text(&error);
            let evidence = margin_evidence(
                HedgePreflightStatus::Blocked,
                &intents,
                &balances,
                balance_evidence,
                Some(detail),
            );
            Err(with_margin_evidence(
                crate::trading_errors::map_trading_error(error),
                &evidence,
            ))
        }
    }
}

fn final_margin_success_guard(
    status: HedgePreflightStatus,
    detail: &str,
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: MarginBalanceEvidence,
) -> ExecutionGuard {
    let outcome = margin_outcome(status, intents, balances, evidence, None);
    guard_with_preflight("margin_balance", "保证金余额", true, detail, outcome)
}

fn guard_with_preflight(
    key: &str,
    label: &str,
    passed: bool,
    detail: &str,
    preflight: MarginPreflightOutcome,
) -> ExecutionGuard {
    ExecutionGuard {
        key: key.to_owned(),
        label: label.to_owned(),
        passed,
        detail: detail.to_owned(),
        preflight_outcome: Some(preflight),
    }
}
