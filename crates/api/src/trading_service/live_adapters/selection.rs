use super::*;

pub(crate) struct PreparedAdapterSelection {
    adapter_id: &'static str,
    risk: RiskConfig,
    router: Option<Arc<LiveVenueRouter>>,
}

impl PreparedAdapterSelection {
    pub(crate) fn adapter_id(&self) -> &'static str {
        self.adapter_id
    }

    pub(crate) fn risk(&self) -> &RiskConfig {
        &self.risk
    }
}

impl TradingService {
    /// Validate and build locally; no runtime adapter/account changes until persistence succeeds.
    pub(crate) fn prepare_adapter_selection(
        &self,
        adapter_id: &str,
        credentials: AdapterCredentials,
    ) -> Result<PreparedAdapterSelection, SelectAdapterError> {
        let mut risk = self.risk_config();
        if self.open_order_count() > 0 && !risk.kill_switch_active {
            return Err(SelectAdapterError::OpenOrders);
        }
        let (adapter_id, router) = match adapter_id {
            "mock" => {
                risk.live_trading_enabled = false;
                risk.allowed_exchanges = BTreeSet::new();
                ("mock", None)
            }
            LIVE_ROUTER_ADAPTER_ID => {
                let router = Arc::new(LiveVenueRouter::with_failure_sink(
                    live_routes_from_credentials(credentials.clone())?,
                    Arc::clone(&self.route_failures),
                ).with_account_scopes(&credentials));
                risk.allowed_exchanges = router.allowed_exchanges();
                if risk.allowed_exchanges.is_empty() {
                    return Err(SelectAdapterError::MissingCredentials);
                }
                risk.live_trading_enabled = true;
                (LIVE_ROUTER_ADAPTER_ID, Some(router))
            }
            other => return Err(SelectAdapterError::Unsupported(other.to_owned())),
        };
        Ok(PreparedAdapterSelection { adapter_id, risk, router })
    }

    pub(crate) fn apply_adapter_selection(&self, prepared: PreparedAdapterSelection) -> RiskConfig {
        if let Some(router) = prepared.router {
            let execution_router: Arc<dyn exchange::LiveTradingAdapter> =
                Arc::<LiveVenueRouter>::clone(&router);
            self.engine.set_adapter_with_accounts(execution_router, router.account_scopes.clone());
            self.replace_private_ws_accounts(&router.account_scopes);
            self.account_reader.store(Some(router));
        } else {
            // Paper mode keeps the existing account reader, as before.
            self.select_mock_adapter();
        }
        self.set_adapter_name(prepared.adapter_id);
        self.risk.update_config(move |risk| *risk = prepared.risk)
    }

    pub(crate) fn try_select_adapter(
        &self,
        adapter_id: &str,
        credentials: AdapterCredentials,
    ) -> Result<RiskConfig, SelectAdapterError> {
        let prepared = self.prepare_adapter_selection(adapter_id, credentials)?;
        Ok(self.apply_adapter_selection(prepared))
    }
}
