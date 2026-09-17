mod errors;
mod events;
mod types;

use crate::routers::extractors::ApiJson;
use crate::services::{
    account_balances, account_positions, account_state,
    action_runs::{self, ActionRunStart},
    execution_runs, trading_credentials,
};
use crate::state::AppState;
use crate::trading_errors::map_trading_error;
use crate::trading_service::{live_venue_capabilities, AdapterCredentials, LIVE_ROUTER_ADAPTER_ID};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use common::AppError;
use errors::map_select_adapter_error;
use events::{publish_order_event, publish_risk_event};
use serde::Deserialize;
use shared_types::{
    problem::codes, AccountStateSnapshot, ActionRun, ActionRunKind, ActionRunStatus, ApiProblem,
    EnvTemplateResponse, ExchangeTransportRegistryResponse, ExchangeWsOperationsResponse,
    ExchangeWsVenuesResponse, ExecutionEnvironment, ExecutionLedgerEvent, ExecutionRun,
    FeeScheduleRegistryResponse, HedgeLegRole, KillSwitchRequest, KillSwitchResponse,
    KillSwitchSummary, ListEnvelope, ListPage, ListStatus, LiveOrderState, OrderRecord,
    RestEndpointsResponse, RiskConfigPatch, TradeFeeSnapshot, TradingAdapterCapabilities,
    TradingAdapterOption, TradingAdaptersResponse, TradingStatusResponse, TradingWsChannels,
    VenueBalanceEnvelope, VenuePositionEnvelope,
};
use types::{
    risk_snapshot, submit_order_intent, submit_order_request, validate_kill_switch_request,
    SelectAdapterPayload,
};

const ORDER_LIST_DEFAULT_LIMIT: usize = 50;
const ORDER_LIST_MAX_LIMIT: usize = 100;
const ORDER_LIST_SOURCE: &str = "order_journal";
const EXECUTION_LEDGER_LIST_DEFAULT_LIMIT: usize = 100;
const EXECUTION_LEDGER_LIST_MAX_LIMIT: usize = 500;
const EXECUTION_LEDGER_LIST_SOURCE: &str = "execution_ledger";
const EXECUTION_RUN_LIST_DEFAULT_LIMIT: usize = 32;
const EXECUTION_RUN_LIST_MAX_LIMIT: usize = 32;
const EXECUTION_RUN_LIST_SOURCE: &str = "execution_run_store";
const HEADER_IDEMPOTENCY_KEY: &str = "idempotency-key";
const HEADER_X_IDEMPOTENCY_KEY: &str = "x-idempotency-key";

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/trading/status", get(status))
        .route("/api/trading/risk-config", patch(update_risk_config))
        .route(
            "/api/trading/credentials/env-template",
            get(credentials_env_template),
        )
        .route("/api/trading/adapters", get(list_adapters))
        .route("/api/trading/ws/venues", get(list_ws_venues))
        .route("/api/trading/ws/operations", get(list_ws_operations))
        .route("/api/trading/rest/endpoints", get(list_rest_endpoints))
        .route(
            "/api/trading/transport/registry",
            get(list_transport_registry),
        )
        .route("/api/trading/fee-schedules", get(list_fee_schedules))
        .route("/api/trading/adapters/select", post(select_adapter))
        .route("/api/trading/kill-switch", post(set_kill_switch))
        .route("/api/trading/balances", get(list_balances))
        .route("/api/trading/account-state", get(account_state_snapshot))
        .route("/api/trading/fee-snapshots", post(upsert_fee_snapshot))
        .route("/api/trading/positions", get(list_positions))
        .route("/api/trading/orders/reconcile", post(reconcile_orders))
        .route("/api/trading/orders", get(list_orders).post(submit_order))
        .route("/api/trading/orders/:id", get(get_order))
        .route("/api/trading/orders/:id/cancel", post(cancel_order))
        .route("/api/trading/execution-ledger", get(list_execution_ledger))
        .route("/api/trading/action-runs", get(list_action_runs))
        .route("/api/trading/action-runs/:id", get(get_action_run))
        .route("/api/trading/execution-runs", get(list_execution_runs))
}

async fn status(State(state): State<AppState>) -> Json<TradingStatusResponse> {
    let service = state.trading_service();
    Json(status_response(service, &service.risk_config()))
}

async fn list_ws_venues() -> Json<ExchangeWsVenuesResponse> {
    Json(exchange::trading_ws_venues())
}

async fn list_ws_operations() -> Json<ExchangeWsOperationsResponse> {
    Json(exchange::trading_ws_operation_registry())
}

async fn list_rest_endpoints() -> Json<RestEndpointsResponse> {
    Json(exchange::rest_endpoint_registry())
}

async fn list_transport_registry() -> Json<ExchangeTransportRegistryResponse> {
    Json(ExchangeTransportRegistryResponse::new(
        exchange::rest_endpoint_registry(),
        exchange::trading_ws_operation_registry(),
    ))
}

async fn list_fee_schedules() -> Result<Json<FeeScheduleRegistryResponse>, AppError> {
    arbitrage::algorithms::fee_evidence::standard_fee_schedule_registry()
        .map(Json)
        .map_err(|error| fee_schedule_registry_error(&error))
}

fn fee_schedule_registry_error(
    error: &arbitrage::algorithms::fee_evidence::FeeScheduleRegistryError,
) -> AppError {
    AppError::domain(
        StatusCode::SERVICE_UNAVAILABLE,
        codes::FEE_SCHEDULE_REGISTRY_UNAVAILABLE,
        "fee schedule registry is unavailable",
    )
    .with_details(serde_json::json!({
        "source": "embedded_fee_schedule_registry",
        "reason": error.to_string(),
    }))
}

fn status_response(
    service: &crate::trading_service::TradingService,
    risk: &trading::RiskConfig,
) -> TradingStatusResponse {
    TradingStatusResponse {
        adapter: service.adapter_name().to_owned(),
        environment: execution_environment(risk),
        open_order_count: service.open_order_count(),
        risk: risk_snapshot(risk),
        ws_channels: TradingWsChannels {
            orders: realtime::channels::ORDERS.to_owned(),
            execution: realtime::channels::EXECUTION.to_owned(),
            risk_alerts: realtime::channels::RISK_ALERTS.to_owned(),
        },
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        mutation: None,
    }
}

fn execution_environment(risk: &trading::RiskConfig) -> ExecutionEnvironment {
    if risk.live_trading_enabled {
        ExecutionEnvironment::Live
    } else {
        ExecutionEnvironment::Paper
    }
}

mod account;
mod adapters;
mod kill_switch;
mod list_parse;
mod listing;
mod mutation;
mod order_replay;
mod orders;
#[cfg(test)]
mod tests;

use account::*;
use adapters::*;
use kill_switch::*;
use list_parse::*;
use listing::*;
use mutation::*;
use order_replay::*;
use orders::*;
