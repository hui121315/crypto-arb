use super::*;

pub(super) async fn get_order(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<OrderRecord>, AppError> {
    let record = state
        .trading_service()
        .get_order(&id)
        .ok_or_else(|| AppError::NotFound(format!("order: {id}")))?;
    Ok(Json(record))
}

pub(super) async fn reconcile_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<trading::ReconcileDiff>>, AppError> {
    let run = action_runs::begin(
        &state,
        ActionRunStart::new(
            ActionRunKind::TradingOrderReconcile,
            &headers,
            Some("open-orders".to_owned()),
            "order reconcile accepted",
        ),
    )?;
    let result = state
        .trading_service()
        .reconcile_open_orders()
        .await
        .map_err(AppError::from);
    action_runs::finish_result_with_payload(&state, &run.id, result, "orders reconciled").map(Json)
}

pub(super) fn adapter_credentials(_payload: &SelectAdapterPayload) -> AdapterCredentials {
    trading_credentials::current_adapter_credentials()
}

pub(super) async fn submit_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<serde_json::Value>,
) -> Result<Json<OrderRecord>, AppError> {
    let request = submit_order_request(payload)?;
    let intent = submit_order_intent(&request)?;
    let client_order_id = intent.client_order_id.clone();
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::TradingOrderSubmit,
            &headers,
            Some(client_order_id.clone()),
            "order submit accepted",
        )
        .with_idempotency_key(Some(client_order_id.clone())),
    )?;
    if claim.is_replayed() {
        return replay_submit_order(&state, claim.run(), &client_order_id);
    }
    let internal_order_id = intent.id.clone();
    let result = match state.trading_service().submit(intent).await {
        Ok(record) if record.intent.id == internal_order_id => {
            order_event_record(&state, "order_submitted", record)
        }
        Ok(record) => {
            return finish_order_submit_payload(
                &state,
                claim.run(),
                record,
                "order submit replayed",
            )
        }
        Err(error) => Err(map_trading_error(error)),
    };
    let record = action_runs::finish_result_with_payload(
        &state,
        &claim.run().id,
        result,
        "order submitted",
    )?;
    Ok(Json(record))
}

pub(super) fn finish_order_submit_payload(
    state: &AppState,
    run: &ActionRun,
    record: OrderRecord,
    message: &'static str,
) -> Result<Json<OrderRecord>, AppError> {
    let record = action_runs::finish_result_with_payload(state, &run.id, Ok(record), message)?;
    Ok(Json(record))
}

pub(super) async fn cancel_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<OrderRecord>, AppError> {
    let idempotency_key =
        explicit_idempotency_key(&headers).unwrap_or_else(|| cancel_idempotency_key(&id));
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::TradingOrderCancel,
            &headers,
            Some(id.clone()),
            "order cancel accepted",
        )
        .with_idempotency_key(Some(idempotency_key.clone())),
    )?;
    if claim.is_replayed() {
        return replay_cancel_order(&state, claim.run(), &id, &idempotency_key).await;
    }
    let result = state
        .trading_service()
        .cancel(&id)
        .await
        .map_err(map_trading_error)
        .and_then(|record| cancel_order_event_record(&state, record));
    finish_cancel_action_payload(&state, claim.run(), result)
}

pub(super) fn order_event_record(
    state: &AppState,
    event: &'static str,
    record: OrderRecord,
) -> Result<OrderRecord, AppError> {
    publish_order_event(state, event, &record)?;
    Ok(record)
}

pub(super) fn cancel_order_event_record(
    state: &AppState,
    record: OrderRecord,
) -> Result<OrderRecord, AppError> {
    order_event_record(state, cancel_order_event(&record), record)
}

pub(super) fn cancel_order_event(record: &OrderRecord) -> &'static str {
    match record.state {
        LiveOrderState::Cancelled => "order_cancelled",
        LiveOrderState::CancelRequested => "order_cancel_requested",
        _ => "order_cancel_result",
    }
}

pub(super) fn finish_cancel_action_payload(
    state: &AppState,
    run: &ActionRun,
    result: Result<OrderRecord, AppError>,
) -> Result<Json<OrderRecord>, AppError> {
    let message = result
        .as_ref()
        .map_or("order cancel failed", cancel_action_message);
    let record = action_runs::finish_result_with_payload(state, &run.id, result, message)?;
    Ok(Json(record))
}

pub(super) fn cancel_action_message(record: &OrderRecord) -> &'static str {
    match record.state {
        LiveOrderState::Cancelled => "order cancelled",
        LiveOrderState::CancelRequested => "order cancel accepted; awaiting finality",
        _ => "order cancel result recorded",
    }
}
