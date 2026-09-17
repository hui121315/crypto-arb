use super::*;

#[derive(Debug)]
pub(in crate::trading_service::tests) struct BalanceProbeAdapter {
    balance_queries: RwLock<Vec<String>>,
    delay_ms: u64,
    full_rate_limit_secs: Option<u64>,
}

impl BalanceProbeAdapter {
    pub(in crate::trading_service::tests) fn new() -> Self {
        Self {
            balance_queries: RwLock::new(Vec::new()),
            delay_ms: 0,
            full_rate_limit_secs: None,
        }
    }

    pub(in crate::trading_service::tests) fn with_delay(delay_ms: u64) -> Self {
        Self {
            balance_queries: RwLock::new(Vec::new()),
            delay_ms,
            full_rate_limit_secs: None,
        }
    }

    pub(in crate::trading_service::tests) fn with_full_rate_limit(
        delay_ms: u64,
        retry_after_secs: u64,
    ) -> Self {
        Self {
            balance_queries: RwLock::new(Vec::new()),
            delay_ms,
            full_rate_limit_secs: Some(retry_after_secs),
        }
    }

    pub(in crate::trading_service::tests) fn balance_queries(&self) -> Vec<String> {
        self.balance_queries.read().clone()
    }
}

#[async_trait::async_trait]
impl LiveTradingAdapter for BalanceProbeAdapter {
    fn name(&self) -> &'static str {
        "balance_probe"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities::testnet_limit_only()
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        Ok(OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("probe".into()),
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
        self.balance_queries.write().push(exchange.to_owned());
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if exchange == "fail" {
            return Err(ExchangeError::Network("balance probe failed".into()));
        }
        if exchange == "rate_limited" {
            return Err(ExchangeError::RateLimited {
                retry_after_secs: 2,
            });
        }
        Ok(vec![balance_row(exchange, 1_000.0)])
    }

    async fn get_balances(&self, _currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        self.balance_queries.write().push("*".to_owned());
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if let Some(retry_after_secs) = self.full_rate_limit_secs {
            return Err(ExchangeError::RateLimited { retry_after_secs });
        }
        Ok(vec![
            balance_row("binance", 1_000.0),
            balance_row("gate", 1_000.0),
        ])
    }

    async fn get_positions(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        Ok(Vec::new())
    }
}

pub(in crate::trading_service::tests) fn balance_row(
    venue: &str,
    available: f64,
) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: normalized_venue_name(venue),
        currency: "USDT".into(),
        total: available,
        available,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }
}
