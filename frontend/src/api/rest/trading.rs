use super::*;

impl ApiClient {
    pub async fn trading_status(&self) -> Result<TradingStatusResponse, ApiError> {
        self.get_json("/api/trading/status").await
    }

    pub async fn trading_adapters(&self) -> Result<TradingAdaptersResponse, ApiError> {
        self.get_json("/api/trading/adapters").await
    }

    pub async fn select_trading_adapter_with_context(
        &self,
        request: &shared_types::SelectTradingAdapterRequest,
        context: &MutationRequestContext,
    ) -> Result<TradingStatusResponse, ApiError> {
        self.post_json_with_context("/api/trading/adapters/select", request, context)
            .await
    }

    pub async fn trading_credentials_env_template(
        &self,
    ) -> Result<shared_types::EnvTemplateResponse, ApiError> {
        self.get_json("/api/trading/credentials/env-template").await
    }

    pub async fn update_trading_risk_config(
        &self,
        patch: &shared_types::RiskConfigPatch,
    ) -> Result<TradingStatusResponse, ApiError> {
        self.patch_json("/api/trading/risk-config", patch).await
    }

    pub async fn update_trading_risk_config_with_idempotency_key(
        &self,
        patch: &shared_types::RiskConfigPatch,
        idempotency_key: &str,
    ) -> Result<TradingStatusResponse, ApiError> {
        self.update_trading_risk_config_with_context(
            patch,
            &MutationRequestContext::with_idempotency_key(idempotency_key),
        )
        .await
    }

    pub(crate) async fn update_trading_risk_config_with_context(
        &self,
        patch: &shared_types::RiskConfigPatch,
        context: &MutationRequestContext,
    ) -> Result<TradingStatusResponse, ApiError> {
        self.patch_json_with_context("/api/trading/risk-config", patch, context)
            .await
    }

    pub async fn trading_ws_venues(
        &self,
    ) -> Result<shared_types::ExchangeWsVenuesResponse, ApiError> {
        self.get_json("/api/trading/ws/venues").await
    }

    pub async fn trading_ws_operations(
        &self,
    ) -> Result<shared_types::ExchangeWsOperationsResponse, ApiError> {
        self.get_json("/api/trading/ws/operations").await
    }

    pub async fn trading_rest_endpoints(
        &self,
    ) -> Result<shared_types::RestEndpointsResponse, ApiError> {
        self.get_json("/api/trading/rest/endpoints").await
    }

    pub async fn trading_transport_registry(
        &self,
    ) -> Result<shared_types::ExchangeTransportRegistryResponse, ApiError> {
        self.get_json("/api/trading/transport/registry").await
    }

    pub async fn trading_fee_schedules(
        &self,
    ) -> Result<shared_types::FeeScheduleRegistryResponse, ApiError> {
        self.get_json("/api/trading/fee-schedules").await
    }

    pub async fn trading_orders(
        &self,
    ) -> Result<shared_types::ListEnvelope<shared_types::OrderRecord>, ApiError> {
        self.get_json("/api/trading/orders?limit=50").await
    }

    pub(crate) async fn trading_order(&self, id: &str) -> Result<shared_types::OrderRecord, ApiError> {
        self.get_json(&format!("/api/trading/orders/{}", encode_path_segment(id))).await
    }

    pub async fn execution_runs(
        &self,
    ) -> Result<shared_types::ListEnvelope<shared_types::ExecutionRun>, ApiError> {
        self.get_json("/api/trading/execution-runs").await
    }

    pub async fn execution_runs_for_context(
        &self,
        opportunity_id: Option<&str>,
        ticket_id: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<shared_types::ListEnvelope<shared_types::ExecutionRun>, ApiError> {
        self.get_json(&execution_runs_path(opportunity_id, ticket_id, run_id))
            .await
    }

    pub async fn action_runs_envelope(&self) -> Result<shared_types::ActionRunEnvelope, ApiError> {
        self.get_json("/api/trading/action-runs").await
    }

    pub async fn action_runs(&self) -> Result<Vec<shared_types::ActionRun>, ApiError> {
        self.action_runs_envelope()
            .await?
            .into_data()
            .map_err(|problem| ApiError::from_problem(*problem))
    }

    pub async fn action_run(&self, id: &str) -> Result<shared_types::ActionRun, ApiError> {
        self.get_json(&action_run_path(id)).await
    }

    pub async fn trading_positions(&self) -> Result<Vec<shared_types::PositionInfo>, ApiError> {
        let envelope: shared_types::VenuePositionEnvelope =
            self.get_json("/api/trading/positions").await?;
        Ok(envelope.rows)
    }

    pub async fn trading_balances(&self) -> Result<shared_types::VenueBalanceEnvelope, ApiError> {
        self.get_json("/api/trading/balances").await
    }

    pub async fn trading_account_state(
        &self,
    ) -> Result<shared_types::AccountStateSnapshot, ApiError> {
        self.get_json("/api/trading/account-state").await
    }

    pub async fn submit_order(
        &self,
        req: &SubmitOrderRequest,
    ) -> Result<shared_types::OrderRecord, ApiError> {
        self.post_json("/api/trading/orders", req).await
    }

    pub async fn cancel_order(&self, id: &str) -> Result<shared_types::OrderRecord, ApiError> {
        self.cancel_order_with_context(id, &MutationRequestContext::new())
            .await
    }

    pub(crate) async fn cancel_order_with_context(
        &self,
        id: &str,
        context: &MutationRequestContext,
    ) -> Result<shared_types::OrderRecord, ApiError> {
        self.post_json_with_context(&cancel_order_path(id), &serde_json::json!({}), context)
            .await
    }

    pub async fn set_kill_switch(
        &self,
        request: &shared_types::KillSwitchRequest,
    ) -> Result<shared_types::KillSwitchResponse, ApiError> {
        self.post_json("/api/trading/kill-switch", request).await
    }

    pub async fn set_kill_switch_with_idempotency_key(
        &self,
        request: &shared_types::KillSwitchRequest,
        idempotency_key: &str,
    ) -> Result<shared_types::KillSwitchResponse, ApiError> {
        self.set_kill_switch_with_context(
            request,
            &MutationRequestContext::with_idempotency_key(idempotency_key),
        )
        .await
    }

    pub(crate) async fn set_kill_switch_with_context(
        &self,
        request: &shared_types::KillSwitchRequest,
        context: &MutationRequestContext,
    ) -> Result<shared_types::KillSwitchResponse, ApiError> {
        self.post_json_with_context("/api/trading/kill-switch", request, context)
            .await
    }
}

fn action_run_path(id: &str) -> String {
    let id = encode_path_segment(id);
    format!("/api/trading/action-runs/{id}")
}

fn cancel_order_path(id: &str) -> String {
    let id = encode_path_segment(id);
    format!("/api/trading/orders/{id}/cancel")
}

fn execution_runs_path(
    opportunity_id: Option<&str>,
    ticket_id: Option<&str>,
    run_id: Option<&str>,
) -> String {
    let mut params = Vec::new();
    push_query_param(&mut params, "opportunityId", opportunity_id);
    push_query_param(&mut params, "ticketId", ticket_id);
    push_query_param(&mut params, "runId", run_id);
    if params.is_empty() {
        "/api/trading/execution-runs".to_owned()
    } else {
        format!("/api/trading/execution-runs?{}", params.join("&"))
    }
}

fn push_query_param(params: &mut Vec<String>, key: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    params.push(format!("{key}={}", encode_query_component(value)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_runs_path_omits_empty_filters() {
        assert_eq!(
            execution_runs_path(Some("opp 1"), Some(" "), None),
            "/api/trading/execution-runs?opportunityId=opp%201"
        );
        assert_eq!(
            execution_runs_path(None, None, None),
            "/api/trading/execution-runs"
        );
    }

    #[test]
    fn trading_identifier_paths_cannot_escape_their_route_segment() {
        let id = "run/order?mode=cancel#leg %";
        let encoded = "run%2Forder%3Fmode%3Dcancel%23leg%20%25";

        assert_eq!(
            action_run_path(id),
            format!("/api/trading/action-runs/{encoded}")
        );
        assert_eq!(
            cancel_order_path(id),
            format!("/api/trading/orders/{encoded}/cancel")
        );
    }
}
