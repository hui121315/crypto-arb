use super::adapters::balance_row;
use super::*;

#[derive(Debug)]
pub(super) struct SubmitRateLimitAdapter {
    retry_after_secs: u64,
}

impl SubmitRateLimitAdapter {
    pub(super) fn new(retry_after_secs: u64) -> Self {
        Self { retry_after_secs }
    }
}

#[async_trait::async_trait]
impl LiveTradingAdapter for SubmitRateLimitAdapter {
    fn name(&self) -> &'static str {
        "submit_rate_limit"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities::testnet_limit_only()
    }

    async fn place_order(&self, _intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        Err(ExchangeError::RateLimited {
            retry_after_secs: self.retry_after_secs,
        })
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
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
        Ok(None)
    }

    async fn get_open_orders(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        Ok(Vec::new())
    }

    async fn get_exchange_balances(
        &self,
        exchange: &str,
        _currency: Option<&str>,
    ) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(vec![balance_row(exchange, 1_000_000.0)])
    }

    async fn get_balances(&self, _currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(vec![balance_row("mock", 1_000_000.0)])
    }

    async fn get_positions(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        Ok(Vec::new())
    }
}
