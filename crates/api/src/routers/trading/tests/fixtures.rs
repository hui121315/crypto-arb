#![allow(clippy::panic)]
use super::super::*;
use axum::http::HeaderValue;
use common::config::AppConfig;
use serde_json::json;

pub(super) async fn submitted_record(
    state: &AppState,
    id: &'static str,
    client_order_id: &'static str,
) -> OrderRecord {
    submitted_record_with_mode(state, id, client_order_id, "dry_run").await
}

pub(super) async fn submitted_live_record(
    state: &AppState,
    id: &'static str,
    client_order_id: &'static str,
) -> OrderRecord {
    submitted_record_with_mode(state, id, client_order_id, "live").await
}

pub(super) async fn submitted_record_with_mode(
    state: &AppState,
    id: &'static str,
    client_order_id: &'static str,
    mode: &'static str,
) -> OrderRecord {
    match submit_order(
        State(state.clone()),
        HeaderMap::new(),
        ApiJson(submit_payload(id, Some(client_order_id), mode)),
    )
    .await
    {
        Ok(Json(record)) => record,
        Err(error) => panic!("submit failed: {error}"),
    }
}

pub(super) async fn cancelled_record(state: &AppState, id: &str) -> OrderRecord {
    match cancel_order(State(state.clone()), HeaderMap::new(), Path(id.to_owned())).await {
        Ok(Json(record)) => record,
        Err(error) => panic!("cancel failed: {error}"),
    }
}

pub(super) fn find_action_run(state: &AppState, kind: ActionRunKind) -> ActionRun {
    match action_runs::recent(state)
        .into_iter()
        .find(|run| run.kind == kind)
    {
        Some(run) => run,
        None => panic!("missing action run: {kind:?}"),
    }
}

pub(super) fn recv_order_event(
    orders: &mut tokio::sync::broadcast::Receiver<realtime::WsMessage>,
) -> String {
    match orders.try_recv() {
        Ok(message) => match message.payload_json() {
            Some(value) => value["event"].as_str().unwrap_or_default().to_owned(),
            None => panic!("missing JSON order event: {message:?}"),
        },
        other => panic!("missing JSON order event: {other:?}"),
    }
}

pub(super) async fn test_state() -> AppState {
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    config.storage.execution_run_ledger_path = None;
    match AppState::new(config).await {
        Ok(state) => state,
        Err(error) => panic!("state init failed: {error}"),
    }
}

pub(super) fn submit_payload(
    id: &'static str,
    client_order_id: Option<&'static str>,
    mode: &'static str,
) -> serde_json::Value {
    let mut payload = json!({
        "id": id,
        "mode": mode,
        "source": "manual",
        "exchange": "mock",
        "symbol": "BTCUSDT",
        "side": "buy",
        "orderType": "limit",
        "quantity": 1.0,
        "price": 10.0,
        "timeInForce": "gtc",
        "leverage": 1.0
    });
    if let Some(client_order_id) = client_order_id {
        payload["clientOrderId"] = json!(client_order_id);
    }
    payload
}

pub(super) fn kill_switch_request(
    active: bool,
    expected_active: bool,
    expected_open_order_count: usize,
    reason: &'static str,
) -> shared_types::KillSwitchRequest {
    shared_types::KillSwitchRequest {
        active,
        expected_active: Some(expected_active),
        expected_open_order_count: Some(expected_open_order_count),
        reason: reason.to_owned(),
    }
}

pub(super) fn idempotency_headers(name: &'static str, key: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(name, HeaderValue::from_static(key));
    headers
}

pub(super) fn execution_run(
    run_id: &'static str,
    ticket_id: &'static str,
    opportunity_id: &'static str,
    updated_at_ms: i64,
) -> ExecutionRun {
    ExecutionRun {
        run_id: run_id.to_owned(),
        ticket_id: ticket_id.to_owned(),
        opportunity_id: opportunity_id.to_owned(),
        state: shared_types::ExecutionRunState::Previewed,
        long_leg: execution_run_leg(HedgeLegRole::Long),
        short_leg: execution_run_leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "seed".to_owned(),
        created_at_ms: updated_at_ms,
        updated_at_ms,
    }
}

pub(super) fn execution_run_leg(role: HedgeLegRole) -> shared_types::ExecutionRunLeg {
    shared_types::ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Created,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
