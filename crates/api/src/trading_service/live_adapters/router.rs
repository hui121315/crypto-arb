use super::*;
use tokio::sync::Semaphore;

mod account_reads;
mod funding_payments;
mod route_reads;

#[derive(Clone)]
pub(crate) struct LiveVenueRouter {
    pub(super) routes: Arc<LiveRouteMap>,
    pub(super) failures: Arc<RouteFailureSink>,
    pub(super) private_read_budget: Arc<Semaphore>,
}

impl LiveVenueRouter {
    #[cfg(test)]
    pub(super) fn new(routes: LiveRouteMap) -> Self {
        Self::with_failure_sink(routes, Arc::new(RouteFailureSink::default()))
    }

    pub(super) fn with_failure_sink(routes: LiveRouteMap, failures: Arc<RouteFailureSink>) -> Self {
        Self {
            routes: Arc::new(routes),
            failures,
            private_read_budget: Arc::new(Semaphore::new(PRIVATE_READ_CONCURRENCY)),
        }
    }

    pub(crate) fn allowed_exchanges(&self) -> BTreeSet<String> {
        self.routes.keys().cloned().collect()
    }

    pub(super) fn route_for(&self, venue: &str) -> ExchangeResult<LiveAdapter> {
        let key = normalized_venue(venue);
        self.routes
            .get(&key)
            .or_else(|| route_family_route(&key).and_then(|family| self.routes.get(family)))
            .cloned()
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(format!("live route exchange={venue}")))
    }
}

#[async_trait::async_trait]
impl LiveTradingAdapter for LiveVenueRouter {
    fn name(&self) -> &'static str {
        LIVE_ROUTER_ADAPTER_ID
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        self.routes
            .values()
            .fold(empty_capabilities(), merge_capabilities)
    }

    fn exchange_capabilities(&self, exchange: &str) -> ExchangeResult<ExchangeCapabilities> {
        Ok(self.route_for(exchange)?.capabilities())
    }

    fn exchange_capability_matrix_for_product(
        &self,
        exchange: &str,
        product: shared_types::FeeProduct,
    ) -> ExchangeResult<shared_types::VenueCapabilityMatrix> {
        self.route_for(exchange)?
            .exchange_capability_matrix_for_product(exchange, product)
    }

    fn exchange_order_margin_modes(&self, exchange: &str) -> ExchangeResult<Vec<MarginMode>> {
        Ok(self.route_for(exchange)?.order_margin_modes())
    }

    async fn get_exchange_account_mode(
        &self,
        exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.route_for(exchange)?
            .get_exchange_account_mode(exchange)
            .await
    }

    async fn get_exchange_symbol_account_mode(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.route_for(exchange)?
            .get_exchange_symbol_account_mode(exchange, symbol)
            .await
    }

    async fn preflight_order(&self, exchange: &str, intent: &OrderIntent) -> ExchangeResult<()> {
        self.route_for(exchange)?
            .preflight_order(exchange, intent)
            .await
    }

    async fn preflight_order_with_context(
        &self,
        exchange: &str,
        intent: &OrderIntent,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<()> {
        self.route_for(exchange)?
            .preflight_order_with_context(exchange, intent, context)
            .await
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.route_for(&intent.exchange)?.place_order(intent).await
    }

    async fn place_order_with_context(
        &self,
        intent: &OrderIntent,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        self.route_for(&intent.exchange)?
            .place_order_with_context(intent, context)
            .await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.route_for(&request.exchange)?
            .cancel_order(request)
            .await
    }

    async fn cancel_order_with_context(
        &self,
        request: &CancelOrderRequest,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        self.route_for(&request.exchange)?
            .cancel_order_with_context(request, context)
            .await
    }

    fn exchange_withdrawal_submission_supported(&self, exchange: &str) -> bool {
        self.route_for(exchange)
            .is_ok_and(|route| route.withdrawal_submission_supported())
    }

    async fn withdrawal_source_balance(
        &self,
        request: &exchange::WithdrawalSourceBalanceRequest,
    ) -> ExchangeResult<exchange::WithdrawalSourceBalance> {
        self.route_for(&request.venue)?
            .withdrawal_source_balance(request)
            .await
    }

    async fn submit_withdrawal(
        &self,
        request: &exchange::WithdrawalSubmitRequest,
    ) -> ExchangeResult<exchange::WithdrawalSubmission> {
        self.route_for(&request.venue)?
            .submit_withdrawal(request)
            .await
    }

    async fn withdrawal_status(
        &self,
        request: &exchange::WithdrawalStatusRequest,
    ) -> ExchangeResult<Option<exchange::WithdrawalStatusEvidence>> {
        self.route_for(&request.venue)?
            .withdrawal_status(request)
            .await
    }

    fn exchange_deposit_status_supported(&self, exchange: &str) -> bool {
        self.route_for(exchange)
            .is_ok_and(|route| route.deposit_status_supported())
    }

    async fn deposit_status(
        &self,
        request: &exchange::DepositStatusRequest,
    ) -> ExchangeResult<Option<exchange::DepositStatusEvidence>> {
        self.route_for(&request.venue)?
            .deposit_status(request)
            .await
    }

    async fn get_order(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        Err(ExchangeError::NotImplemented(
            "live router order read requires exchange-scoped get_exchange_order",
        ))
    }

    async fn get_exchange_order(
        &self,
        exchange: &str,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.route_for(exchange)?
            .get_order(symbol, client_order_id)
            .await
    }

    async fn get_exchange_order_with_context(
        &self,
        exchange: &str,
        symbol: &str,
        client_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.route_for(exchange)?
            .get_order_with_context(symbol, client_order_id, context)
            .await
    }

    async fn get_exchange_order_by_exchange_order_id(
        &self,
        exchange: &str,
        symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.route_for(exchange)?
            .get_order_by_exchange_order_id(symbol, exchange_order_id)
            .await
    }

    async fn get_exchange_order_by_exchange_order_id_with_context(
        &self,
        exchange: &str,
        symbol: &str,
        exchange_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.route_for(exchange)?
            .get_order_by_exchange_order_id_with_context(symbol, exchange_order_id, context)
            .await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        let venues = self.allowed_exchanges().into_iter().collect::<Vec<_>>();
        self.open_orders_for_venues(&venues, symbol).await
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(self.get_account_read(currency).await?.balances)
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        self.collect_account_read(currency, None, BALANCE_ROUTE_TIMEOUT)
            .await
    }

    async fn get_exchange_balances(
        &self,
        exchange: &str,
        currency: Option<&str>,
    ) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        self.route_for(exchange)?.get_balances(currency).await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        let venues = self.allowed_exchanges().into_iter().collect::<Vec<_>>();
        self.positions_for_venues(&venues, symbol).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        self.get_routed_funding_payments(symbol, start_time_ms, end_time_ms)
            .await
    }
}

fn open_order_route_timeout(route: &str) -> Duration {
    if is_hyperliquid_route(route) {
        HYPERLIQUID_OPEN_ORDER_ROUTE_TIMEOUT
    } else {
        OPEN_ORDER_ROUTE_TIMEOUT
    }
}

fn account_route_timeout(route: &str, default: Duration) -> Duration {
    if is_hyperliquid_route(route) {
        default.max(HYPERLIQUID_ACCOUNT_ROUTE_TIMEOUT)
    } else {
        default
    }
}

fn position_route_timeout(route: &str) -> Duration {
    account_route_timeout(route, POSITION_ROUTE_TIMEOUT)
}

fn is_hyperliquid_route(route: &str) -> bool {
    route == "hyperliquid" || route.starts_with("hyperliquid:")
}

async fn with_route_timeout<T>(
    limit: std::time::Duration,
    fut: impl std::future::Future<Output = ExchangeResult<T>>,
) -> ExchangeResult<T> {
    match tokio::time::timeout(limit, fut).await {
        Ok(inner) => inner,
        Err(_) => Err(ExchangeError::Timeout {
            seconds: limit.as_secs(),
        }),
    }
}
