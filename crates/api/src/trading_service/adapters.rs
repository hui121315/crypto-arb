use super::*;

impl TradingService {
    pub(crate) fn exchange_capabilities(
        &self,
        exchange: &str,
    ) -> Result<ExchangeCapabilities, exchange::ExchangeError> {
        self.engine.adapter().exchange_capabilities(exchange)
    }

    pub(crate) fn exchange_capability_matrix_for_product(
        &self,
        exchange: &str,
        product: shared_types::FeeProduct,
    ) -> Result<shared_types::VenueCapabilityMatrix, exchange::ExchangeError> {
        self.engine
            .adapter()
            .exchange_capability_matrix_for_product(exchange, product)
    }

    pub(crate) async fn exchange_symbol_account_mode(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> Result<Option<VenueAccountModeInfo>, exchange::ExchangeError> {
        self.engine
            .adapter()
            .get_exchange_symbol_account_mode(exchange, symbol)
            .await
    }

    pub(crate) async fn preflight_order(
        &self,
        intent: &OrderIntent,
    ) -> Result<(), exchange::ExchangeError> {
        self.preflight_order_with_context(intent, &OrderSubmissionContext::default())
            .await
    }

    pub(crate) async fn preflight_order_with_context(
        &self,
        intent: &OrderIntent,
        context: &OrderSubmissionContext,
    ) -> Result<(), exchange::ExchangeError> {
        self.engine
            .adapter()
            .preflight_order_with_context(&intent.exchange, intent, context)
            .await
    }

    pub(crate) async fn submit_withdrawal(
        &self,
        request: &exchange::WithdrawalSubmitRequest,
    ) -> Result<exchange::WithdrawalSubmission, exchange::ExchangeError> {
        self.engine.adapter().submit_withdrawal(request).await
    }

    pub(crate) fn withdrawal_submission_supported(&self, venue: &str) -> bool {
        self.engine
            .adapter()
            .exchange_withdrawal_submission_supported(venue)
    }

    pub(crate) async fn withdrawal_source_balance(
        &self,
        request: &exchange::WithdrawalSourceBalanceRequest,
    ) -> Result<exchange::WithdrawalSourceBalance, exchange::ExchangeError> {
        self.engine
            .adapter()
            .withdrawal_source_balance(request)
            .await
    }

    pub(crate) async fn withdrawal_status(
        &self,
        request: &exchange::WithdrawalStatusRequest,
    ) -> Result<Option<exchange::WithdrawalStatusEvidence>, exchange::ExchangeError> {
        self.engine.adapter().withdrawal_status(request).await
    }

    pub(crate) fn deposit_status_supported(&self, venue: &str) -> bool {
        self.engine
            .adapter()
            .exchange_deposit_status_supported(venue)
    }

    pub(crate) async fn deposit_status(
        &self,
        request: &exchange::DepositStatusRequest,
    ) -> Result<Option<exchange::DepositStatusEvidence>, exchange::ExchangeError> {
        self.engine.adapter().deposit_status(request).await
    }

    /// Drain the per-route failures recorded by the most recent tolerant fanout
    /// read for `operation` (`"positions"` / `"balances"`), so callers can
    /// surface them as `RuntimeProblem`s instead of letting partial failures go
    /// silent.
    pub(crate) fn take_route_failures(&self, operation: &str) -> Vec<RouteFailure> {
        self.route_failures.take(operation)
    }

    pub(crate) fn take_route_failures_for_venues(
        &self,
        operation: &str,
        venues: &[String],
    ) -> Vec<RouteFailure> {
        self.route_failures.take_for_venues(operation, venues)
    }

    pub(crate) fn balance_cache_health(&self) -> Vec<AccountCacheSnapshot> {
        self.balance_cache
            .snapshots(self.account_cache_epoch(), common::time::now_ms())
    }

    pub(crate) fn position_cache_health(&self) -> Vec<AccountCacheSnapshot> {
        self.position_cache
            .snapshots(self.account_cache_epoch(), common::time::now_ms())
    }

    pub(crate) fn select_mock_adapter(&self) -> RiskConfig {
        self.engine.set_adapter(Arc::new(MockLiveAdapter::new()));
        self.set_adapter_name("mock");
        self.risk.update_config(|config| {
            config.live_trading_enabled = false;
            config.allowed_exchanges = BTreeSet::new();
        })
    }

    pub(crate) fn risk_config(&self) -> RiskConfig {
        self.risk.config()
    }

    pub(crate) fn set_kill_switch(&self, active: bool) -> RiskConfig {
        self.risk.set_kill_switch(active)
    }

    pub(crate) fn update_risk_config(&self, update: impl FnOnce(&mut RiskConfig)) -> RiskConfig {
        self.risk.update_config(update)
    }
}
