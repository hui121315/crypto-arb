use super::*;

impl TradingService {
    pub(super) fn select_live_router_adapter(
        &self,
        adapter_id: &'static str,
        credentials: AdapterCredentials,
    ) -> Result<RiskConfig, SelectAdapterError> {
        let router = Arc::new(LiveVenueRouter::with_failure_sink(
            live_routes_from_credentials(credentials)?,
            Arc::clone(&self.route_failures),
        ));
        let allowed_exchanges = router.allowed_exchanges();
        if allowed_exchanges.is_empty() {
            return Err(SelectAdapterError::MissingCredentials);
        }

        let execution_router: Arc<dyn exchange::LiveTradingAdapter> =
            Arc::<LiveVenueRouter>::clone(&router);
        self.engine.set_adapter(execution_router);
        self.account_reader.store(Some(router));
        self.set_adapter_name(adapter_id);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = allowed_exchanges;
        }))
    }

    /// Apply adapter selection with safety gates. Callers pass already-scoped
    /// credentials so env reads stay at the HTTP edge.
    pub(crate) fn try_select_adapter(
        &self,
        adapter_id: &str,
        credentials: AdapterCredentials,
    ) -> Result<RiskConfig, SelectAdapterError> {
        if self.open_order_count() > 0 && !self.risk_config().kill_switch_active {
            return Err(SelectAdapterError::OpenOrders);
        }
        match adapter_id {
            "mock" => Ok(self.select_mock_adapter()),
            LIVE_ROUTER_ADAPTER_ID => {
                self.select_live_router_adapter(LIVE_ROUTER_ADAPTER_ID, credentials)
            }
            other => Err(SelectAdapterError::Unsupported(other.to_owned())),
        }
    }
}
