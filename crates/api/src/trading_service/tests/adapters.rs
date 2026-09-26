use super::*;

mod balance_probe;

pub(super) use balance_probe::{balance_row, BalanceProbeAdapter};

#[derive(Debug)]
pub(super) struct ReconcileTestAdapter {
    open_orders: RwLock<Vec<OrderInfo>>,
    order: RwLock<Option<OrderInfo>>,
    order_error: RwLock<Option<String>>,
    open_orders_error: Option<String>,
    place_timeout_secs: Option<u64>,
    exchange_order_queries: RwLock<Vec<String>>,
    exchange_order_query_ids: RwLock<Vec<String>>,
    exchange_order_id_queries: RwLock<Vec<String>>,
    cancel_requests: RwLock<Vec<String>>,
}

impl ReconcileTestAdapter {
    pub(super) fn new(open_orders: Vec<OrderInfo>, order: Option<OrderInfo>) -> Self {
        Self {
            open_orders: RwLock::new(open_orders),
            order: RwLock::new(order),
            order_error: RwLock::new(None),
            open_orders_error: None,
            place_timeout_secs: None,
            exchange_order_queries: RwLock::new(Vec::new()),
            exchange_order_query_ids: RwLock::new(Vec::new()),
            exchange_order_id_queries: RwLock::new(Vec::new()),
            cancel_requests: RwLock::new(Vec::new()),
        }
    }

    pub(super) fn with_order_error(open_orders: Vec<OrderInfo>, error: &str) -> Self {
        Self {
            open_orders: RwLock::new(open_orders),
            order: RwLock::new(None),
            order_error: RwLock::new(Some(error.to_owned())),
            open_orders_error: None,
            place_timeout_secs: None,
            exchange_order_queries: RwLock::new(Vec::new()),
            exchange_order_query_ids: RwLock::new(Vec::new()),
            exchange_order_id_queries: RwLock::new(Vec::new()),
            cancel_requests: RwLock::new(Vec::new()),
        }
    }

    pub(super) fn exchange_order_queries(&self) -> Vec<String> {
        self.exchange_order_queries.read().clone()
    }

    pub(super) fn with_place_timeout(order: Option<OrderInfo>, seconds: u64) -> Self {
        Self {
            open_orders: RwLock::new(Vec::new()),
            order: RwLock::new(order),
            order_error: RwLock::new(None),
            open_orders_error: None,
            place_timeout_secs: Some(seconds),
            exchange_order_queries: RwLock::new(Vec::new()),
            exchange_order_query_ids: RwLock::new(Vec::new()),
            exchange_order_id_queries: RwLock::new(Vec::new()),
            cancel_requests: RwLock::new(Vec::new()),
        }
    }

    pub(super) fn with_open_orders_error(order: Option<OrderInfo>, error: &str) -> Self {
        Self {
            open_orders: RwLock::new(Vec::new()),
            order: RwLock::new(order),
            order_error: RwLock::new(None),
            open_orders_error: Some(error.to_owned()),
            place_timeout_secs: None,
            exchange_order_queries: RwLock::new(Vec::new()),
            exchange_order_query_ids: RwLock::new(Vec::new()),
            exchange_order_id_queries: RwLock::new(Vec::new()),
            cancel_requests: RwLock::new(Vec::new()),
        }
    }

    pub(super) fn exchange_order_query_ids(&self) -> Vec<String> {
        self.exchange_order_query_ids.read().clone()
    }

    pub(super) fn exchange_order_id_queries(&self) -> Vec<String> {
        self.exchange_order_id_queries.read().clone()
    }

    pub(super) fn cancel_requests(&self) -> Vec<String> {
        self.cancel_requests.read().clone()
    }
}

#[async_trait::async_trait]
impl LiveTradingAdapter for ReconcileTestAdapter {
    fn name(&self) -> &'static str {
        "reconcile_test"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities::testnet_limit_only()
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        if let Some(seconds) = self.place_timeout_secs {
            return Err(ExchangeError::Timeout { seconds });
        }
        Ok(OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: common::time::now_ms(),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.cancel_requests.write().push(request.internal_order_id.clone());
        Ok(OrderAck {
            internal_order_id: request.internal_order_id.clone(),
            exchange_order_id: request.exchange_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Cancelled,
            accepted_at_ms: common::time::now_ms(),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    async fn get_order(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        if let Some(error) = self.order_error.read().clone() {
            return Err(ExchangeError::Network(error));
        }
        Ok(self.order.read().clone())
    }

    async fn get_exchange_order(
        &self,
        exchange: &str,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.exchange_order_queries
            .write()
            .push(exchange.to_owned());
        self.exchange_order_query_ids
            .write()
            .push(client_order_id.to_owned());
        self.get_order(symbol, client_order_id).await
    }

    async fn get_exchange_order_with_context(
        &self,
        exchange: &str,
        symbol: &str,
        client_order_id: &str,
        _context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_exchange_order(exchange, symbol, client_order_id)
            .await
    }

    async fn get_exchange_order_by_exchange_order_id(
        &self,
        exchange: &str,
        symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.exchange_order_queries
            .write()
            .push(exchange.to_owned());
        self.exchange_order_id_queries
            .write()
            .push(exchange_order_id.to_owned());
        self.get_order(symbol, exchange_order_id).await
    }

    async fn get_exchange_order_by_exchange_order_id_with_context(
        &self,
        exchange: &str,
        symbol: &str,
        exchange_order_id: &str,
        _context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_exchange_order_by_exchange_order_id(exchange, symbol, exchange_order_id)
            .await
    }

    async fn get_open_orders(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        if let Some(error) = &self.open_orders_error {
            return Err(ExchangeError::Network(error.clone()));
        }
        Ok(self.open_orders.read().clone())
    }

    async fn get_balances(&self, _currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(vec![balance_row("mock", 1_000.0)])
    }

    async fn get_positions(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        Ok(Vec::new())
    }
}
