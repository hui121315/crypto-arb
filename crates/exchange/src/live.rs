//! V2 实盘交易 adapter trait。

use crate::error::{ExchangeError, ExchangeResult};
use crate::transfer_network::{
    DepositStatusEvidence, DepositStatusRequest, WithdrawalSourceBalance,
    WithdrawalSourceBalanceRequest, WithdrawalStatusEvidence, WithdrawalStatusRequest,
    WithdrawalSubmission, WithdrawalSubmitRequest,
};
use async_trait::async_trait;
use shared_types::{
    BalanceInfo, CancelOrderRequest, FeeProduct, FundingPaymentData, MarginMode, OrderAck,
    OrderInfo, OrderIntent, OrderSubmissionContext, PositionInfo, VenueAccountModeInfo,
    VenueAccountSummary, VenueAssetValuation, VenueBalanceInfo, VenueCapabilityMatrix,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExchangeCapabilities {
    pub supports_testnet: bool,
    pub supports_live: bool,
    pub supports_spot: bool,
    pub supports_perp: bool,
    pub supports_limit_orders: bool,
    pub supports_market_orders: bool,
    pub supports_post_only: bool,
    pub supports_reduce_only: bool,
}

/// Authenticated WebSocket readiness reported by one live venue adapter.
///
/// Counts are used instead of booleans because one venue can expose separate
/// Spot and Derivatives sessions. A stream is ready only after every configured
/// source has delivered an authoritative snapshot or update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrivateWsRuntimeStatus {
    pub sessions: usize,
    pub subscriptions: usize,
    pub account_streams: usize,
    pub account_samples: usize,
    pub order_streams: usize,
    pub order_samples: usize,
}

impl PrivateWsRuntimeStatus {
    pub fn account_ready(self) -> bool {
        self.account_streams > 0 && self.account_samples == self.account_streams
    }

    pub fn order_ready(self) -> bool {
        self.order_streams > 0 && self.order_samples == self.order_streams
    }
}

#[derive(Debug, Default)]
pub struct VenueAccountRead {
    pub balances: Vec<VenueBalanceInfo>,
    pub summaries: Vec<VenueAccountSummary>,
    pub asset_valuations: Vec<VenueAssetValuation>,
    pub issues: Vec<VenueAccountReadIssue>,
}

/// One failed source inside an otherwise usable account read.
///
/// Adapters use this for composite accounts whose independent truth sources
/// must not collapse into all-or-nothing delivery. The API router converts
/// each issue into the same typed route-failure path used by failed venues.
#[derive(Debug)]
pub struct VenueAccountReadIssue {
    pub venue: String,
    pub operation: &'static str,
    pub error: ExchangeError,
}

impl VenueAccountReadIssue {
    pub fn new(venue: impl Into<String>, operation: &'static str, error: ExchangeError) -> Self {
        Self {
            venue: venue.into(),
            operation,
            error,
        }
    }
}

impl ExchangeCapabilities {
    pub fn testnet_limit_only() -> Self {
        Self {
            supports_testnet: true,
            supports_live: false,
            supports_spot: false,
            supports_perp: false,
            supports_limit_orders: true,
            supports_market_orders: false,
            supports_post_only: false,
            supports_reduce_only: true,
        }
    }
}

#[async_trait]
pub trait LiveTradingAdapter: Send + Sync {
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> ExchangeCapabilities;

    fn exchange_capabilities(&self, _exchange: &str) -> ExchangeResult<ExchangeCapabilities> {
        Ok(self.capabilities())
    }

    /// Margin modes the adapter's order write path can honor per order.
    /// Empty means the venue order payload does not carry a margin mode.
    fn order_margin_modes(&self) -> Vec<MarginMode> {
        Vec::new()
    }

    fn exchange_order_margin_modes(&self, _exchange: &str) -> ExchangeResult<Vec<MarginMode>> {
        Ok(self.order_margin_modes())
    }

    fn exchange_capability_matrix(&self, exchange: &str) -> ExchangeResult<VenueCapabilityMatrix> {
        self.exchange_capability_matrix_for_product(exchange, FeeProduct::Perp)
    }

    fn exchange_capability_matrix_for_product(
        &self,
        exchange: &str,
        product: FeeProduct,
    ) -> ExchangeResult<VenueCapabilityMatrix> {
        Ok(crate::venue_capability_matrix_for_product(
            exchange,
            self.exchange_capabilities(exchange)?,
            self.exchange_order_margin_modes(exchange)?,
            product,
        ))
    }

    async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        Ok(None)
    }

    async fn get_exchange_symbol_account_mode(
        &self,
        exchange: &str,
        _symbol: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.get_exchange_account_mode(exchange).await
    }

    async fn preflight_order(&self, _exchange: &str, _intent: &OrderIntent) -> ExchangeResult<()> {
        Ok(())
    }

    async fn preflight_order_with_context(
        &self,
        exchange: &str,
        intent: &OrderIntent,
        _context: &OrderSubmissionContext,
    ) -> ExchangeResult<()> {
        self.preflight_order(exchange, intent).await
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck>;

    async fn place_order_with_context(
        &self,
        intent: &OrderIntent,
        _context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        self.place_order(intent).await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck>;

    async fn cancel_order_with_context(
        &self,
        request: &CancelOrderRequest,
        _context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        self.cancel_order(request).await
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>>;

    async fn get_order_with_context(
        &self,
        symbol: &str,
        client_order_id: &str,
        _context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_order(symbol, client_order_id).await
    }

    async fn get_order_by_exchange_order_id(
        &self,
        symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_order(symbol, exchange_order_id).await
    }

    async fn get_order_by_exchange_order_id_with_context(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        _context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_order_by_exchange_order_id(symbol, exchange_order_id)
            .await
    }

    async fn get_exchange_order(
        &self,
        _exchange: &str,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_order(symbol, client_order_id).await
    }

    async fn get_exchange_order_with_context(
        &self,
        _exchange: &str,
        symbol: &str,
        client_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_order_with_context(symbol, client_order_id, context)
            .await
    }

    async fn get_exchange_order_by_exchange_order_id(
        &self,
        _exchange: &str,
        symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_order_by_exchange_order_id(symbol, exchange_order_id)
            .await
    }

    async fn get_exchange_order_by_exchange_order_id_with_context(
        &self,
        _exchange: &str,
        symbol: &str,
        exchange_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.get_order_by_exchange_order_id_with_context(symbol, exchange_order_id, context)
            .await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>>;

    async fn get_balances(&self, _currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Err(ExchangeError::NotImplemented("get_balances"))
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        Ok(VenueAccountRead {
            balances: self.get_balances(currency).await?,
            summaries: Vec::new(),
            asset_valuations: Vec::new(),
            issues: Vec::new(),
        })
    }

    async fn get_exchange_balances(
        &self,
        _exchange: &str,
        currency: Option<&str>,
    ) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        self.get_balances(currency).await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>>;

    async fn get_funding_payments(
        &self,
        _symbol: Option<&str>,
        _start_time_ms: Option<i64>,
        _end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        Err(ExchangeError::NotImplemented("get_funding_payments"))
    }

    fn withdrawal_submission_supported(&self) -> bool {
        false
    }

    fn exchange_withdrawal_submission_supported(&self, _exchange: &str) -> bool {
        self.withdrawal_submission_supported()
    }

    async fn withdrawal_source_balance(
        &self,
        _request: &WithdrawalSourceBalanceRequest,
    ) -> ExchangeResult<WithdrawalSourceBalance> {
        Err(ExchangeError::UnsupportedCapability(
            "withdrawal_source_balance",
        ))
    }

    /// Submit exactly one non-idempotent venue withdrawal. Implementations
    /// must never retry this write; callers reconcile using persisted venue identities.
    async fn submit_withdrawal(
        &self,
        _request: &WithdrawalSubmitRequest,
    ) -> ExchangeResult<WithdrawalSubmission> {
        Err(ExchangeError::UnsupportedCapability("withdrawal_submit"))
    }

    /// Read withdrawal finality using client identity or the persisted provider ID.
    async fn withdrawal_status(
        &self,
        _request: &WithdrawalStatusRequest,
    ) -> ExchangeResult<Option<WithdrawalStatusEvidence>> {
        Err(ExchangeError::UnsupportedCapability("withdrawal_status"))
    }

    fn deposit_status_supported(&self) -> bool {
        false
    }

    fn exchange_deposit_status_supported(&self, _exchange: &str) -> bool {
        self.deposit_status_supported()
    }

    /// Read an exchange deposit by its exact chain transaction identity.
    async fn deposit_status(
        &self,
        _request: &DepositStatusRequest,
    ) -> ExchangeResult<Option<DepositStatusEvidence>> {
        Err(ExchangeError::UnsupportedCapability("deposit_status"))
    }
}

pub fn venue_balance_rows(
    venue: &str,
    balances: HashMap<String, BalanceInfo>,
) -> Vec<VenueBalanceInfo> {
    let mut rows: Vec<VenueBalanceInfo> = balances
        .into_values()
        .map(|balance| VenueBalanceInfo::from_balance(venue, balance))
        .collect();
    rows.sort_by(|a, b| a.currency.cmp(&b.currency));
    rows
}
