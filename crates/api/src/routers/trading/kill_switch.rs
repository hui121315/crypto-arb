use super::*;

pub(super) async fn set_kill_switch(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<KillSwitchRequest>,
) -> Result<Json<KillSwitchResponse>, AppError> {
    let target = kill_switch_target(payload.active);
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::TradingKillSwitch,
            &headers,
            Some(target),
            kill_switch_accepted_message(payload.active),
        )
        .with_idempotency_key(explicit_idempotency_key(&headers)),
    )?;
    if claim.is_replayed() {
        return replay_kill_switch_response(claim.run());
    }
    let _mutation = state.trading_runtime_config_mutation_lock().lock().await;
    let service = state.trading_service();
    let previous_risk = service.risk_config();
    let before = status_response(service, &previous_risk);
    let previous_active = previous_risk.kill_switch_active;
    let open_order_count = service.open_order_count();
    let result = kill_switch_update_response(
        &state,
        service,
        &payload,
        KillSwitchUpdateContext {
            previous_active,
            open_order_count,
            action_run_id: &claim.run().id,
            request_id: claim.run().request_id.clone(),
            idempotency_key: claim.run().idempotency_key.clone(),
        },
    )
    .map(|mut response| {
        let mutation = trading_mutation_diff(&before, &response.status);
        attach_action_receipt(&mut response.status, claim.run(), mutation);
        response
    });
    let message = result.as_ref().map_or_else(
        |_| "kill switch update failed".to_owned(),
        kill_switch_success_message,
    );
    let mutation = match &result {
        Ok(response) => response.status.mutation.clone(),
        Err(_) => None,
    };
    let response = action_runs::finish_result_with_payload_and_mutation(
        &state,
        &claim.run().id,
        result,
        message,
        mutation,
    )?;
    Ok(Json(response))
}

pub(super) fn replay_kill_switch_response(
    run: &ActionRun,
) -> Result<Json<KillSwitchResponse>, AppError> {
    match run.status {
        ActionRunStatus::Succeeded => {
            let response = replay_succeeded_kill_switch_response(run)?;
            Ok(Json(response))
        }
        ActionRunStatus::Failed => Err(kill_switch_replay_failed(run)),
        ActionRunStatus::Accepted => Err(kill_switch_action_in_flight(run)),
    }
}

pub(super) fn replay_succeeded_kill_switch_response(
    run: &ActionRun,
) -> Result<KillSwitchResponse, AppError> {
    let mut response = action_runs::replay_payload::<KillSwitchResponse>(run)?;
    response.action_run_id.get_or_insert_with(|| run.id.clone());
    if response.request_id.is_none() {
        response.request_id = run.request_id.clone();
    }
    if response.idempotency_key.is_none() {
        response.idempotency_key = run.idempotency_key.clone();
    }
    Ok(response)
}

pub(super) fn kill_switch_update_response(
    state: &AppState,
    service: &crate::trading_service::TradingService,
    payload: &KillSwitchRequest,
    context: KillSwitchUpdateContext<'_>,
) -> Result<KillSwitchResponse, AppError> {
    let reason =
        validate_kill_switch_request(payload, context.previous_active, context.open_order_count)?;
    let mut next = service.risk_config();
    next.kill_switch_active = payload.active;
    // A storage failure must never undo an emergency stop or release an existing stop.
    if payload.active {
        service.set_kill_switch(true);
    }
    if let Err(error) = super::events::persist_risk_config(
        state, service.adapter_name(), &next, payload.active,
    ) {
        if payload.active {
            let _ = publish_risk_event(state, "kill_switch_updated", &next);
        }
        return Err(error);
    }
    let risk = service.set_kill_switch(payload.active);
    publish_risk_event(state, "kill_switch_updated", &risk)?;
    let summary = KillSwitchSummary {
        previous_active: context.previous_active,
        active: payload.active,
        open_order_count: context.open_order_count,
        expected_open_order_count: payload.expected_open_order_count,
        reason,
        checked_at_ms: common::time::now_ms(),
    };
    Ok(KillSwitchResponse {
        status: status_response(service, &risk),
        summary,
        action_run_id: Some(context.action_run_id.to_owned()),
        request_id: context.request_id,
        idempotency_key: context.idempotency_key,
    })
}

pub(super) struct KillSwitchUpdateContext<'a> {
    pub(super) previous_active: bool,
    pub(super) open_order_count: usize,
    pub(super) action_run_id: &'a str,
    pub(super) request_id: Option<String>,
    pub(super) idempotency_key: Option<String>,
}

pub(super) fn kill_switch_target(active: bool) -> String {
    if active {
        "kill-switch:on"
    } else {
        "kill-switch:off"
    }
    .to_owned()
}

pub(super) fn kill_switch_accepted_message(active: bool) -> &'static str {
    if active {
        "kill switch enable accepted"
    } else {
        "kill switch disable accepted"
    }
}

pub(super) fn kill_switch_success_message(response: &KillSwitchResponse) -> String {
    let state = if response.summary.active {
        "enabled"
    } else {
        "disabled"
    };
    format!(
        "kill switch {state}; reason {}; openOrders {}",
        response.summary.reason, response.summary.open_order_count
    )
}

pub(super) fn risk_config_update_response(
    state: &AppState,
    service: &crate::trading_service::TradingService,
    payload: RiskConfigPatch,
) -> Result<Json<TradingStatusResponse>, AppError> {
    let next = crate::services::risk_config::prepare(service.risk_config(), payload)?;
    super::events::persist_risk_config(state, service.adapter_name(), &next, false)?;
    let risk = service.update_risk_config(move |risk| *risk = next);
    risk_event_response(state, service, "risk_config_updated", &risk)
}

pub(super) fn risk_event_response(
    state: &AppState,
    service: &crate::trading_service::TradingService,
    event: &'static str,
    risk: &trading::RiskConfig,
) -> Result<Json<TradingStatusResponse>, AppError> {
    publish_risk_event(state, event, risk)?;
    Ok(Json(status_response(service, risk)))
}
