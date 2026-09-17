use super::*;

pub(super) async fn get_action_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ActionRun>, AppError> {
    action_runs::get(&state, &id)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("action-run: {id}")))
}

pub(super) async fn list_positions(State(state): State<AppState>) -> Json<VenuePositionEnvelope> {
    Json(account_positions::envelope(&state).await)
}

pub(super) async fn list_balances(State(state): State<AppState>) -> Json<VenueBalanceEnvelope> {
    Json(account_balances::envelope(&state).await)
}

pub(super) async fn account_state_snapshot(
    State(state): State<AppState>,
) -> Json<AccountStateSnapshot> {
    Json(account_state::cached_snapshot(&state))
}

pub(super) async fn upsert_fee_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(snapshot): Json<TradeFeeSnapshot>,
) -> Result<Json<TradeFeeSnapshot>, AppError> {
    let run = action_runs::begin(
        &state,
        ActionRunStart::new(
            ActionRunKind::TradingFeeSnapshotUpsert,
            &headers,
            Some(format!(
                "{}:{}:{:?}",
                snapshot.venue, snapshot.symbol, snapshot.product
            )),
            "fee snapshot upsert accepted",
        ),
    )?;
    if let Err(error) =
        crate::services::fees::validate_external_fee_snapshot(&snapshot, common::time::now_ms())
    {
        return action_runs::fail_response(&state, &run.id, error);
    }
    state.trade_fee_cache().upsert(snapshot.clone());
    action_runs::finish_result_with_payload(&state, &run.id, Ok(snapshot), "fee snapshot upserted")
        .map(Json)
}

pub(super) async fn credentials_env_template() -> Json<EnvTemplateResponse> {
    Json(crate::services::venue_credentials::env_template())
}

pub(super) async fn update_risk_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RiskConfigPatch>,
) -> Result<Json<TradingStatusResponse>, AppError> {
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::TradingRiskConfigUpdate,
            &headers,
            Some("risk-config".to_owned()),
            "risk config update accepted",
        )
        .with_idempotency_key(explicit_idempotency_key(&headers)),
    )?;
    if claim.is_replayed() {
        return replay_trading_status_response(claim.run(), "risk config");
    }
    let _mutation = state.trading_runtime_config_mutation_lock().lock().await;
    let service = state.trading_service();
    let before = status_response(service, &service.risk_config());
    let result = risk_config_update_response(&state, service, payload).map(|Json(mut response)| {
        let mutation = trading_mutation_diff(&before, &response);
        attach_action_receipt(&mut response, claim.run(), mutation);
        response
    });
    let mutation = match &result {
        Ok(response) => response.mutation.clone(),
        Err(_) => None,
    };
    if result
        .as_ref()
        .is_ok_and(|response| before.risk.auto_profit_close != response.risk.auto_profit_close)
    {
        state.request_portfolio_refresh();
    }
    action_runs::finish_result_with_payload_and_mutation(
        &state,
        &claim.run().id,
        result,
        "risk config updated",
        mutation,
    )
    .map(Json)
}

pub(super) async fn list_adapters(State(state): State<AppState>) -> Json<TradingAdaptersResponse> {
    let credentials = trading_credentials::current_adapter_credentials();
    let credential_status = live_credentials_status(&credentials);
    let service = state.trading_service();
    let risk = service.risk_config();
    Json(TradingAdaptersResponse {
        current: service.adapter_name().to_owned(),
        current_environment: execution_environment(&risk),
        options: vec![mock_adapter_option(), live_router_option(credential_status)],
        venues: live_venue_capabilities(&credentials),
    })
}
