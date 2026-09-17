use super::*;

#[derive(Clone, Copy)]
pub(super) struct LiveCredentialsStatus {
    pub(super) available_count: usize,
}

pub(super) fn mock_adapter_option() -> TradingAdapterOption {
    TradingAdapterOption {
        id: "mock".to_owned(),
        label: "Paper".to_owned(),
        environment: ExecutionEnvironment::Paper,
        enabled: true,
        credentials_available: true,
        capabilities: TradingAdapterCapabilities {
            spot: false,
            perp: false,
            limit_orders: true,
            market_orders: false,
            post_only: false,
            reduce_only: true,
        },
        disabled_reason: None,
    }
}

pub(super) fn live_router_option(credentials: LiveCredentialsStatus) -> TradingAdapterOption {
    let credentials_available = credentials.available_count > 0;
    TradingAdapterOption {
        id: LIVE_ROUTER_ADAPTER_ID.to_owned(),
        label: "实盘".to_owned(),
        environment: ExecutionEnvironment::Live,
        enabled: credentials_available,
        credentials_available,
        capabilities: TradingAdapterCapabilities {
            spot: false,
            perp: true,
            limit_orders: true,
            market_orders: true,
            post_only: true,
            reduce_only: true,
        },
        disabled_reason: live_router_disabled_reason(&credentials),
    }
}

pub(super) fn live_router_disabled_reason(credentials: &LiveCredentialsStatus) -> Option<String> {
    if credentials.available_count == 0 {
        Some(
            "至少补齐一个交易所 API 字段组；每张 HedgeTicket 仍会校验双腿权限与运行态证据"
                .to_owned(),
        )
    } else {
        None
    }
}

pub(super) fn live_credentials_status(credentials: &AdapterCredentials) -> LiveCredentialsStatus {
    let available_count = credentials.available_count();
    LiveCredentialsStatus { available_count }
}

pub(super) async fn select_adapter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SelectAdapterPayload>,
) -> Result<Json<TradingStatusResponse>, AppError> {
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::TradingAdapterSelect,
            &headers,
            Some(payload.adapter_id.clone()),
            "adapter selection accepted",
        )
        .with_idempotency_key(explicit_idempotency_key(&headers)),
    )?;
    if claim.is_replayed() {
        return replay_trading_status_response(claim.run(), "adapter selection");
    }
    let _mutation = state.trading_runtime_config_mutation_lock().lock().await;
    let service = state.trading_service();
    let before = status_response(service, &service.risk_config());
    let credentials = adapter_credentials(&payload);
    let result = service
        .try_select_adapter(&payload.adapter_id, credentials)
        .map_err(map_select_adapter_error)
        .and_then(|risk| risk_event_response(&state, service, "adapter_selected", &risk));
    let result = result.map(|Json(mut response)| {
        let mutation = trading_mutation_diff(&before, &response);
        attach_action_receipt(&mut response, claim.run(), mutation);
        response
    });
    let mutation = match &result {
        Ok(response) => response.mutation.clone(),
        Err(_) => None,
    };
    action_runs::finish_result_with_payload_and_mutation(
        &state,
        &claim.run().id,
        result,
        "adapter selected",
        mutation,
    )
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_router_disabled_copy_does_not_claim_api_validation() {
        let reason = live_router_disabled_reason(&LiveCredentialsStatus { available_count: 0 });

        assert!(reason.is_some());
        let reason = reason.as_deref().unwrap_or_default();
        assert!(reason.contains("字段组"));
        assert!(reason.contains("运行态证据"));
        assert!(!reason.contains("保存并验证"));
        assert!(!reason.contains("可下单"));
    }

    #[test]
    fn live_router_enabled_copy_is_not_a_readiness_claim() {
        let option = live_router_option(LiveCredentialsStatus { available_count: 1 });

        assert!(option.enabled);
        assert!(option.credentials_available);
        assert!(option.disabled_reason.is_none());
    }
}
