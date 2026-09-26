//! Synthetic cancel receipts through the real run projector, storage and WS publisher.
use crate::{services::{execution_runs, ws_publish}, state::AppState};
use axum::{http::{HeaderMap, StatusCode}, routing::post, Json, Router};
use serde_json::json;
use shared_types::{ExecutionRun, OrderRecord};

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scenario { Zero, Partial, Balanced, Unknown }

#[derive(serde::Deserialize)]
struct Input { scenario: Scenario, complete: bool }

#[derive(serde::Deserialize)]
struct RecoveryInput {
    quantity: Option<f64>,
    #[serde(default)]
    second: bool,
    #[serde(default)]
    save_stale: bool,
}

pub(super) fn controls(state: AppState) -> Router {
    let recovery_state = state.clone();
    Router::new().route("/__paper/execution-cancel", post(move |headers: HeaderMap, Json(input): Json<Input>| {
        let state = state.clone();
        async move {
            if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer isolated-paper-browser") {
                return Err(StatusCode::UNAUTHORIZED);
            }
            project(&state, input).map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }
    })).route("/__paper/execution-recovery", post(move |headers: HeaderMap, Json(input): Json<RecoveryInput>| {
        let state = recovery_state.clone();
        async move {
            if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer isolated-paper-browser") {
                return Err(StatusCode::UNAUTHORIZED);
            }
            recovery(&state, input).map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }
    }))
}

fn recovery(state: &AppState, input: RecoveryInput) -> anyhow::Result<ExecutionRun> {
    let mut run = project_named(state, Input { scenario: Scenario::Partial, complete: true }, "paper-recovery")?;
    let mut stale = run.clone();
    stale.long_leg.state = shared_types::LiveOrderState::Submitted;
    stale.long_leg.filled_quantity = None;
    stale.long_leg.filled_notional_usd = None;
    stale.evidence = Default::default();
    let id = if input.second { "paper-recovery-second" } else { "paper-recovery-first" };
    if !run.long_leg.order_ids.iter().any(|value| value == id) {
        run.long_leg.order_ids.push(id.into());
        execution_runs::record(state, run);
    }
    let record = serde_json::from_value(json!({
        "intent": { "id": id, "clientOrderId": id, "source": "arbitrage_preview", "mode": "dry_run",
            "exchange": "paper", "symbol": "BTC-USDT", "side": "sell", "reduceOnly": true,
            "orderType": "market", "quantity": 0.4, "price": 100.0, "createdAtMs": common::time::now_ms() },
        "state": if input.second { "filled" } else { "cancelled" }, "risk": null, "lastUpdateSource": "order_query", "exchangeOrderId": id,
        "filledQuantity": input.quantity, "filledPrice": 100.0, "filledFee": null, "updatedAtMs": common::time::now_ms()
    }))?;
    ws_publish::publish_order_event(state, "order_updated", &record)?;
    if input.save_stale {
        let saved = execution_runs::record(state, stale);
        ws_publish::publish_execution_run_event(state, "execution_run_updated", &saved)?;
    }
    state.execution_runs().get("paper-recovery").map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("projected recovery absent"))
}

fn project(state: &AppState, input: Input) -> anyhow::Result<ExecutionRun> {
    let scenario = match input.scenario {
        Scenario::Zero => "zero", Scenario::Partial => "partial",
        Scenario::Balanced => "balanced", Scenario::Unknown => "unknown",
    };
    let id = format!("paper-cancel-{scenario}");
    project_named(state, input, &id)
}

fn project_named(state: &AppState, input: Input, id: &str) -> anyhow::Result<ExecutionRun> {
    let now = common::time::now_ms();
    let leg = |role: &str| json!({
        "role": role, "exchange": "paper", "symbol": "BTC-USDT",
        "orderIds": [format!("{id}-{role}")], "state": "submitted",
        "targetQuantity": 1.0, "targetNotionalUsd": 100.0,
        "filledQuantity": null, "filledNotionalUsd": null, "filledFee": null
    });
    if !state.execution_runs().contains_key(id) {
        let run = serde_json::from_value(json!({
            "runId": id, "ticketId": format!("ticket-{id}"), "opportunityId": format!("opp-{id}"),
            "state": "second_leg_submitted", "longLeg": leg("long"), "shortLeg": leg("short"),
            "netExposureUsd": 0.0, "recoveryAction": null, "statusReason": "等待撤单结果",
            "createdAtMs": now, "updatedAtMs": now
        }))?;
        execution_runs::record(state, run);
    }
    let long_quantity = match input.scenario { Scenario::Partial | Scenario::Balanced => 0.4, _ => 0.0 };
    let short_quantity = match (input.scenario, input.complete) {
        (Scenario::Unknown, _) | (_, false) => None,
        (Scenario::Balanced, true) => Some(0.4), _ => Some(0.0),
    };
    for (role, side, quantity) in [("long", "buy", Some(long_quantity)), ("short", "sell", short_quantity)] {
        let order_id = format!("{id}-{role}");
        let record: OrderRecord = serde_json::from_value(json!({
            "intent": { "id": order_id, "clientOrderId": format!("client-{order_id}"),
                "source": "arbitrage_preview", "mode": "dry_run", "exchange": "paper", "symbol": "BTC-USDT",
                "side": side, "orderType": "limit", "quantity": 1.0, "price": 100.0, "createdAtMs": now },
            "state": "cancelled", "risk": null, "lastUpdateSource": "order_query",
            "exchangeOrderId": format!("venue-{order_id}"), "message": "isolated cancellation receipt",
            "filledQuantity": quantity, "filledPrice": 100.0, "filledFee": null, "updatedAtMs": now
        }))?;
        ws_publish::publish_order_event(state, "order_updated", &record)?;
    }
    state.execution_runs().get(id).map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("projected run absent"))
}
