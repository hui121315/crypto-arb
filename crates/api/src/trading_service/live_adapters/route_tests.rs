use super::*;
use shared_types::{OrderSide, OrderStatus, OrderType};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
enum PositionRead {
    Empty,
    One,
    Error,
}

#[derive(Clone, Copy)]
enum BalanceRead {
    Empty,
    One,
    Partial,
    Error,
}

#[derive(Clone, Copy)]
enum OrderRead {
    Empty,
    One,
    Error,
}

#[derive(Clone, Copy)]
enum FundingPaymentRead {
    Empty,
    One,
    Error,
    Unsupported,
}

struct NamedAdapter {
    name: &'static str,
    capabilities: ExchangeCapabilities,
    position_read: PositionRead,
    balance_read: BalanceRead,
    order_read: OrderRead,
    funding_payment_read: FundingPaymentRead,
    funding_payment_symbols: Arc<Mutex<Vec<Option<String>>>>,
    order_reads: Arc<AtomicUsize>,
    account_mode: Option<&'static str>,
}

#[async_trait::async_trait]
impl LiveTradingAdapter for NamedAdapter {
    fn name(&self) -> &'static str {
        self.name
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        self.capabilities
    }

    async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        Ok(self
            .account_mode
            .map(|mode| account_mode_info(self.name, mode)))
    }

    async fn preflight_order(&self, _exchange: &str, intent: &OrderIntent) -> ExchangeResult<()> {
        if intent.exchange == self.name {
            Ok(())
        } else {
            Err(ExchangeError::UnsupportedSymbol(format!(
                "{} cannot preflight {}",
                self.name, intent.exchange
            )))
        }
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        Ok(OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some(self.name.into()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: shared_types::LiveOrderState::Accepted,
            accepted_at_ms: intent.created_at_ms,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        Ok(OrderAck {
            internal_order_id: request.internal_order_id.clone(),
            exchange_order_id: Some(self.name.into()),
            client_order_id: request.client_order_id.clone(),
            identity_update: Default::default(),
            state: shared_types::LiveOrderState::Cancelled,
            accepted_at_ms: 1,
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
        self.order_reads.fetch_add(1, Ordering::Relaxed);
        match self.order_read {
            OrderRead::Empty => Ok(None),
            OrderRead::One => Ok(Some(order_info(self.name))),
            OrderRead::Error => Err(ExchangeError::Parse(format!(
                "{} order fixture failed",
                self.name
            ))),
        }
    }

    async fn get_open_orders(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        match self.order_read {
            OrderRead::Empty => Ok(Vec::new()),
            OrderRead::One => Ok(vec![order_info(self.name)]),
            OrderRead::Error => Err(ExchangeError::Parse(format!(
                "{} open orders fixture failed",
                self.name
            ))),
        }
    }

    async fn get_balances(&self, _currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        match self.balance_read {
            BalanceRead::Empty => Ok(Vec::new()),
            BalanceRead::One | BalanceRead::Partial => Ok(vec![balance(self.name)]),
            BalanceRead::Error => Err(ExchangeError::Parse(format!(
                "{} balances fixture failed",
                self.name
            ))),
        }
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        let balances = self.get_balances(currency).await?;
        let issues = if matches!(self.balance_read, BalanceRead::Partial) {
            vec![exchange::VenueAccountReadIssue::new(
                "hyperliquid:spot",
                "spot_truth",
                ExchangeError::Network("spot fixture failed".into()),
            )]
        } else {
            Vec::new()
        };
        Ok(VenueAccountRead {
            balances,
            summaries: Vec::new(),
            asset_valuations: Vec::new(),
            issues,
        })
    }

    async fn get_positions(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        match self.position_read {
            PositionRead::Empty => Ok(Vec::new()),
            PositionRead::One => Ok(vec![position(self.name)]),
            PositionRead::Error => Err(ExchangeError::Parse(format!(
                "{} positions fixture failed",
                self.name
            ))),
        }
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        _start_time_ms: Option<i64>,
        _end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        self.funding_payment_symbols
            .lock()
            .push(symbol.map(str::to_owned));
        match self.funding_payment_read {
            FundingPaymentRead::Empty => Ok(Vec::new()),
            FundingPaymentRead::One => Ok(vec![funding_payment(self.name)]),
            FundingPaymentRead::Error => Err(ExchangeError::Parse(format!(
                "{} funding payments fixture failed",
                self.name
            ))),
            FundingPaymentRead::Unsupported => {
                Err(ExchangeError::NotImplemented("get_funding_payments"))
            }
        }
    }
}

mod builders;
mod cases;

use builders::*;
