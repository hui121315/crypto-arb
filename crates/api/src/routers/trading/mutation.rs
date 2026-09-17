use super::*;
use shared_types::{ActionMutationChange, ActionMutationDiff};

pub(super) fn attach_action_receipt(
    response: &mut TradingStatusResponse,
    run: &ActionRun,
    mutation: Option<ActionMutationDiff>,
) {
    response.action_run_id = Some(run.id.clone());
    response.request_id = run.request_id.clone();
    response.idempotency_key = run.idempotency_key.clone();
    response.mutation = mutation;
}

pub(super) fn trading_mutation_diff(
    before: &TradingStatusResponse,
    after: &TradingStatusResponse,
) -> Option<ActionMutationDiff> {
    let mut changes = Vec::new();
    if before.adapter != after.adapter {
        changes.push(ActionMutationChange::Adapter {
            before: before.adapter.clone(),
            after: after.adapter.clone(),
        });
    }
    if before.environment != after.environment {
        changes.push(ActionMutationChange::ExecutionEnvironment {
            before: environment_value(before.environment).to_owned(),
            after: environment_value(after.environment).to_owned(),
        });
    }
    append_risk_mutation_changes(&mut changes, &before.risk, &after.risk);
    (!changes.is_empty()).then(|| ActionMutationDiff {
        effective_at_ms: common::time::now_ms(),
        changes,
    })
}

fn append_risk_mutation_changes(
    changes: &mut Vec<ActionMutationChange>,
    before: &shared_types::TradingRiskStatus,
    after: &shared_types::TradingRiskStatus,
) {
    if before.live_trading_enabled != after.live_trading_enabled {
        changes.push(ActionMutationChange::LiveTradingEnabled {
            before: before.live_trading_enabled,
            after: after.live_trading_enabled,
        });
    }
    if before.kill_switch_active != after.kill_switch_active {
        changes.push(ActionMutationChange::KillSwitchActive {
            before: before.kill_switch_active,
            after: after.kill_switch_active,
        });
    }
    if before.max_order_notional != after.max_order_notional {
        changes.push(ActionMutationChange::MaxOrderNotional {
            before: before.max_order_notional,
            after: after.max_order_notional,
        });
    }
    if before.max_open_orders != after.max_open_orders {
        changes.push(ActionMutationChange::MaxOpenOrders {
            before: before.max_open_orders,
            after: after.max_open_orders,
        });
    }
    if before.max_hedge_imbalance_pct != after.max_hedge_imbalance_pct {
        changes.push(ActionMutationChange::MaxHedgeImbalancePct {
            before: before.max_hedge_imbalance_pct,
            after: after.max_hedge_imbalance_pct,
        });
    }
    if before.liquidation_warn_pct != after.liquidation_warn_pct {
        changes.push(ActionMutationChange::LiquidationWarnPct {
            before: before.liquidation_warn_pct,
            after: after.liquidation_warn_pct,
        });
    }
    if before.liquidation_danger_pct != after.liquidation_danger_pct {
        changes.push(ActionMutationChange::LiquidationDangerPct {
            before: before.liquidation_danger_pct,
            after: after.liquidation_danger_pct,
        });
    }
    if before.allowed_exchanges != after.allowed_exchanges {
        changes.push(ActionMutationChange::AllowedExchanges {
            before: before.allowed_exchanges.clone(),
            after: after.allowed_exchanges.clone(),
        });
    }
    if before.allowed_symbols != after.allowed_symbols {
        changes.push(ActionMutationChange::AllowedSymbols {
            before: before.allowed_symbols.clone(),
            after: after.allowed_symbols.clone(),
        });
    }
    if before.protected_positions != after.protected_positions {
        changes.push(ActionMutationChange::ProtectedPositions {
            before: before.protected_positions.clone(),
            after: after.protected_positions.clone(),
        });
    }
    if before.auto_profit_close != after.auto_profit_close {
        changes.push(ActionMutationChange::AutoProfitClose {
            before: before.auto_profit_close.clone(),
            after: after.auto_profit_close.clone(),
        });
    }
}

fn environment_value(environment: ExecutionEnvironment) -> &'static str {
    match environment {
        ExecutionEnvironment::Paper => "paper",
        ExecutionEnvironment::Live => "live",
    }
}

pub(super) fn replay_trading_status_response(
    run: &ActionRun,
    action: &'static str,
) -> Result<Json<TradingStatusResponse>, AppError> {
    match run.status {
        ActionRunStatus::Succeeded => {
            action_runs::replay_payload::<TradingStatusResponse>(run).map(Json)
        }
        ActionRunStatus::Failed => Err(trading_status_replay_failed(run, action)),
        ActionRunStatus::Accepted => Err(AppError::domain(
            StatusCode::CONFLICT,
            codes::ACTION_RUN_IN_FLIGHT,
            format!("{action} idempotency key is already in flight"),
        )
        .with_details(serde_json::json!({
            "actionRunId": run.id,
            "requestId": run.request_id,
            "idempotencyKey": run.idempotency_key,
            "status": run.status,
        }))),
    }
}

fn trading_status_replay_failed(run: &ActionRun, action: &'static str) -> AppError {
    if let Some(problem) = run
        .problem
        .as_ref()
        .filter(|problem| problem.code == codes::ACTION_RUN_REPLAY_UNAVAILABLE)
    {
        let status = problem
            .status
            .and_then(|status| StatusCode::from_u16(status).ok())
            .unwrap_or(StatusCode::CONFLICT);
        return AppError::domain(
            status,
            codes::ACTION_RUN_REPLAY_UNAVAILABLE,
            problem.message.clone(),
        )
        .with_details(serde_json::json!({
            "actionRunId": run.id,
            "requestId": run.request_id,
            "idempotencyKey": run.idempotency_key,
            "replayed": true,
            "originalProblem": problem,
        }));
    }
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_FAILED,
        format!("{action} idempotency key already failed"),
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
        "problem": run.problem,
    }))
}
