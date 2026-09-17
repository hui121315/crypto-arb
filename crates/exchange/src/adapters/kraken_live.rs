//! Kraken Spot and Derivatives live-trading/account integration.

use super::kraken::Kraken;
use super::kraken_futures_rest;
use super::kraken_futures_ws_private::KrakenFuturesPrivateStream;
use super::kraken_spot_private_data::{parse_balance_ex, parse_rest_orders};
use super::kraken_spot_rest::{fetch_balance_ex, fetch_open_orders, fetch_orders};
use super::kraken_spot_ws_private::KrakenSpotPrivateStream;
use crate::error::{ExchangeError, ExchangeResult};
use crate::live::{
    ExchangeCapabilities, LiveTradingAdapter, PrivateWsRuntimeStatus, VenueAccountRead,
};
use async_trait::async_trait;
use shared_types::{
    CancelOrderRequest, FeeProduct, OrderAck, OrderInfo, OrderIntent, OrderSubmissionContext,
    PositionInfo, VenueBalanceInfo,
};

impl Kraken {
    pub async fn prepare_stock_submission(&self) -> ExchangeResult<()> {
        self.ensure_live_writes()?;
        self.spot_private()?.warm().await
    }
    /// The stock coordinator must journal and reserve the two-leg plan before
    /// calling this. No public HTTP endpoint enables standalone stock submission.
    pub async fn submit_stock_order(
        &self,
        draft: shared_types::stocks::StockPeerOrderDraft,
        client: String,
    ) -> ExchangeResult<shared_types::stocks::StockPeerOrderReceipt> {
        self.ensure_live_writes()?;
        let original = shared_types::stocks::StockPeerOrderReceipt::pending(draft, client)
            .map_err(|e| ExchangeError::Parse(e.into()))?;
        self.spot_private()?.submit_stock_order(original).await
    }

    pub async fn cancel_stock_order(
        &self,
        client: &str,
    ) -> ExchangeResult<shared_types::stocks::StockPeerCancelAck> {
        self.ensure_live_writes()?;
        self.spot_private()?.cancel_stock_order(client).await
    }
    /// Register a durably owned original order for exact receipt observation.
    /// This does not place, cancel, retry or poll an order.
    pub fn track_stock_order(
        &self,
        receipt: shared_types::stocks::StockPeerOrderReceipt,
    ) -> ExchangeResult<()> {
        self.spot_private()?.track_stock_order(receipt)
    }

    pub fn stock_order_receipt(
        &self,
        client: &str,
    ) -> Option<shared_types::stocks::StockPeerOrderReceipt> {
        self.spot_private_stream
            .get()
            .and_then(|s| s.stock_order_receipt(client))
    }

    pub fn subscribe_stock_receipts(
        &self,
    ) -> ExchangeResult<tokio::sync::broadcast::Receiver<shared_types::stocks::StockPeerOrderReceipt>> {
        Ok(self.spot_private()?.subscribe_stock_receipts())
    }

    /// Call only after persisting this exact settled revision. Pending/conflicting
    /// receipts are never evicted to make room for new orders.
    pub fn release_stock_receipt(
        &self,
        expected: &shared_types::stocks::StockPeerOrderReceipt,
    ) -> ExchangeResult<()> {
        self.spot_private()?.release_stock_receipt(expected)
    }
    pub(super) fn spot_private(&self) -> ExchangeResult<std::sync::Arc<KrakenSpotPrivateStream>> {
        if let Some(stream) = self.spot_private_stream.get() {
            return Ok(stream.clone());
        }
        let stream = KrakenSpotPrivateStream::shared(&self.config, &self.http)?;
        Ok(self.spot_private_stream.get_or_init(|| stream).clone())
    }

    fn futures_private(&self) -> ExchangeResult<std::sync::Arc<KrakenFuturesPrivateStream>> {
        if let Some(stream) = self.futures_private_stream.get() {
            return Ok(stream.clone());
        }
        let stream = KrakenFuturesPrivateStream::shared(&self.config)?;
        Ok(self.futures_private_stream.get_or_init(|| stream).clone())
    }

    pub fn subscribe_spot_executions(
        &self,
    ) -> ExchangeResult<tokio::sync::broadcast::Receiver<super::kraken::KrakenSpotExecution>> {
        Ok(self.spot_private()?.subscribe_executions())
    }

    pub fn spot_execution_snapshot(
        &self,
    ) -> ExchangeResult<Vec<super::kraken::KrakenSpotExecution>> {
        Ok(self.spot_private()?.execution_snapshot())
    }

    fn spot_credentials(&self) -> ExchangeResult<&super::kraken_config::KrakenSpotCredentials> {
        self.config
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.spot.as_ref())
            .ok_or_else(|| ExchangeError::Auth("Kraken Spot credentials missing".to_owned()))
    }

    fn futures_credentials(
        &self,
    ) -> ExchangeResult<&super::kraken_config::KrakenFuturesCredentials> {
        self.config
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.futures.as_ref())
            .ok_or_else(|| ExchangeError::Auth("Kraken Futures credentials missing".to_owned()))
    }

    fn ensure_live_writes(&self) -> ExchangeResult<()> {
        if self.config.allow_live_writes {
            Ok(())
        } else {
            Err(ExchangeError::Auth(
                "Kraken live writes are disabled by configuration".to_owned(),
            ))
        }
    }

    pub async fn warm_private_ws(&self) -> ExchangeResult<PrivateWsRuntimeStatus> {
        if self.spot_credentials().is_ok() {
            self.spot_private()?.warm().await?;
        }
        if self.futures_credentials().is_ok() {
            self.futures_private()?.warm().await?;
        }
        self.private_ws_runtime_status()
    }

    pub fn private_ws_runtime_status(&self) -> ExchangeResult<PrivateWsRuntimeStatus> {
        let mut status = PrivateWsRuntimeStatus::default();
        if self.spot_credentials().is_ok() {
            merge_private_ws_status(&mut status, self.spot_private()?.runtime_status());
        }
        if self.futures_credentials().is_ok() {
            merge_private_ws_status(&mut status, self.futures_private()?.runtime_status());
        }
        if status.sessions == 0 {
            return Err(ExchangeError::Auth(
                "Kraken Spot or Futures credentials missing".to_owned(),
            ));
        }
        Ok(status)
    }

    async fn spot_order_by_client_id(
        &self,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let private = self.spot_private()?;
        private.warm().await?;
        let venue_client_id = crate::client_order_id_policy::required_venue_client_order_id(
            "kraken",
            client_order_id,
        )?;
        if let Some(order) = private.order_by_client_id(&venue_client_id) {
            return Ok(Some(order));
        }
        let body =
            fetch_open_orders(&self.http, &self.spot_base_url, self.spot_credentials()?).await?;
        Ok(parse_rest_orders(&body)?
            .into_iter()
            .find(|row| row.client_order_id.as_deref() == Some(venue_client_id.as_str())))
    }

    async fn futures_order_by_client_id(
        &self,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let private = self.futures_private()?;
        private.warm().await?;
        let venue_client_id = crate::client_order_id_policy::required_venue_client_order_id(
            "kraken",
            client_order_id,
        )?;
        if let Some(order) = private.order_by_client_id(&venue_client_id) {
            return Ok(Some(order));
        }
        kraken_futures_rest::fetch_order(
            &self.http,
            &self.futures_base_url,
            self.futures_credentials()?,
            None,
            Some(&venue_client_id),
        )
        .await
    }
}

fn merge_private_ws_status(total: &mut PrivateWsRuntimeStatus, next: PrivateWsRuntimeStatus) {
    total.sessions += next.sessions;
    total.subscriptions += next.subscriptions;
    total.account_streams += next.account_streams;
    total.account_samples += next.account_samples;
    total.order_streams += next.order_streams;
    total.order_samples += next.order_samples;
}

#[async_trait]
impl LiveTradingAdapter for Kraken {
    fn name(&self) -> &'static str {
        "kraken"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_testnet: false,
            supports_live: self.config.allow_live_writes,
            supports_spot: true,
            supports_perp: true,
            supports_limit_orders: true,
            supports_market_orders: true,
            supports_post_only: true,
            supports_reduce_only: true,
        }
    }

    async fn preflight_order_with_context(
        &self,
        _exchange: &str,
        _intent: &OrderIntent,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<()> {
        match context.product {
            FeeProduct::Spot => {
                self.spot_credentials()?;
            }
            FeeProduct::Perp => {
                self.futures_credentials()?;
            }
            _ => return Err(ExchangeError::UnsupportedCapability("kraken product")),
        }
        Ok(())
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.place_order_with_context(
            intent,
            &OrderSubmissionContext {
                product: FeeProduct::Perp,
                ..Default::default()
            },
        )
        .await
    }

    async fn place_order_with_context(
        &self,
        intent: &OrderIntent,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        self.ensure_live_writes()?;
        self.preflight_order_with_context(&intent.exchange, intent, context)
            .await?;
        match context.product {
            FeeProduct::Spot => self.spot_private()?.place_order(intent).await,
            FeeProduct::Perp => {
                let venue_client_id =
                    crate::client_order_id_policy::required_venue_client_order_id(
                        "kraken",
                        &intent.client_order_id,
                    )?;
                kraken_futures_rest::place_order(
                    &self.http,
                    &self.futures_base_url,
                    self.futures_credentials()?,
                    intent,
                    &venue_client_id,
                )
                .await
            }
            _ => unreachable!("preflight rejects unsupported Kraken products"),
        }
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.cancel_order_with_context(
            request,
            &OrderSubmissionContext {
                product: FeeProduct::Perp,
                ..Default::default()
            },
        )
        .await
    }

    async fn cancel_order_with_context(
        &self,
        request: &CancelOrderRequest,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        self.ensure_live_writes()?;
        match context.product {
            FeeProduct::Spot => self.spot_private()?.cancel_order(request).await,
            FeeProduct::Perp => {
                let venue_client_id =
                    crate::client_order_id_policy::required_venue_client_order_id(
                        "kraken",
                        &request.client_order_id,
                    )?;
                kraken_futures_rest::cancel_order(
                    &self.http,
                    &self.futures_base_url,
                    self.futures_credentials()?,
                    request,
                    &venue_client_id,
                )
                .await
            }
            _ => Err(ExchangeError::UnsupportedCapability("kraken product")),
        }
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let _ = symbol;
        self.futures_order_by_client_id(client_order_id).await
    }

    async fn get_order_with_context(
        &self,
        _symbol: &str,
        client_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        match context.product {
            FeeProduct::Spot => self.spot_order_by_client_id(client_order_id).await,
            FeeProduct::Perp => self.futures_order_by_client_id(client_order_id).await,
            _ => Ok(None),
        }
    }

    async fn get_order_by_exchange_order_id_with_context(
        &self,
        _symbol: &str,
        exchange_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        match context.product {
            FeeProduct::Spot => {
                let private = self.spot_private()?;
                private.warm().await?;
                if let Some(order) = private.order_by_exchange_id(exchange_order_id) {
                    return Ok(Some(order));
                }
                let body = fetch_orders(
                    &self.http,
                    &self.spot_base_url,
                    self.spot_credentials()?,
                    &[exchange_order_id.to_owned()],
                )
                .await?;
                Ok(parse_rest_orders(&body)?.into_iter().next())
            }
            FeeProduct::Perp => {
                let private = self.futures_private()?;
                private.warm().await?;
                if let Some(order) = private.order_by_exchange_id(exchange_order_id) {
                    return Ok(Some(order));
                }
                kraken_futures_rest::fetch_order(
                    &self.http,
                    &self.futures_base_url,
                    self.futures_credentials()?,
                    Some(exchange_order_id),
                    None,
                )
                .await
            }
            _ => Ok(None),
        }
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        let mut rows = Vec::new();
        if self.spot_credentials().is_ok() {
            let private = self.spot_private()?;
            private.warm().await?;
            let mut spot = if private.has_order_sample() {
                private.open_orders(symbol)
            } else {
                parse_rest_orders(
                    &fetch_open_orders(&self.http, &self.spot_base_url, self.spot_credentials()?)
                        .await?,
                )?
            };
            rows.append(&mut spot);
        }
        if self.futures_credentials().is_ok() {
            let private = self.futures_private()?;
            private.warm().await?;
            let mut futures = if private.has_order_sample() {
                private.open_orders(symbol)
            } else {
                kraken_futures_rest::fetch_open_orders(
                    &self.http,
                    &self.futures_base_url,
                    self.futures_credentials()?,
                )
                .await?
            };
            rows.append(&mut futures);
        }
        if let Some(symbol) = symbol {
            rows.retain(|row| row.symbol.eq_ignore_ascii_case(symbol));
        }
        Ok(rows)
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        let mut rows = Vec::new();
        if self.spot_credentials().is_ok() {
            self.spot_private()?.warm().await?;
            let body =
                fetch_balance_ex(&self.http, &self.spot_base_url, self.spot_credentials()?).await?;
            rows.extend(parse_balance_ex(&body)?);
        }
        if self.futures_credentials().is_ok() {
            let private = self.futures_private()?;
            private.warm().await?;
            if private.has_balance_sample() {
                rows.extend(private.balances(currency));
            } else {
                rows.extend(
                    kraken_futures_rest::fetch_balances(
                        &self.http,
                        &self.futures_base_url,
                        self.futures_credentials()?,
                    )
                    .await?,
                );
            }
        }
        if let Some(currency) = currency {
            rows.retain(|row| row.currency.eq_ignore_ascii_case(currency));
        }
        Ok(rows)
    }

    fn withdrawal_submission_supported(&self) -> bool {
        self.spot_credentials().is_ok()
    }

    async fn withdrawal_source_balance(&self, request: &crate::WithdrawalSourceBalanceRequest) -> ExchangeResult<crate::WithdrawalSourceBalance> {
        super::kraken_withdrawals::source_balance(&self.http, &self.spot_base_url, self.spot_credentials()?, request).await
    }

    async fn submit_withdrawal(&self, request: &crate::WithdrawalSubmitRequest) -> ExchangeResult<crate::WithdrawalSubmission> {
        self.ensure_live_writes()?;
        super::kraken_withdrawals::submit(&self.http, &self.spot_base_url, self.spot_credentials()?, request).await
    }

    async fn withdrawal_status(&self, request: &crate::WithdrawalStatusRequest) -> ExchangeResult<Option<crate::WithdrawalStatusEvidence>> {
        super::kraken_withdrawals::status(&self.http, &self.spot_base_url, self.spot_credentials()?, request).await
    }

    fn deposit_status_supported(&self) -> bool {
        self.spot_credentials().is_ok()
    }

    async fn deposit_status(
        &self,
        request: &crate::DepositStatusRequest,
    ) -> ExchangeResult<Option<crate::DepositStatusEvidence>> {
        super::kraken_deposits::status(
            &self.http,
            &self.spot_base_url,
            self.spot_credentials()?,
            request,
        )
        .await
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        Ok(VenueAccountRead {
            balances: self.get_balances(currency).await?,
            summaries: Vec::new(),
            asset_valuations: Vec::new(),
            issues: Vec::new(),
        })
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        let private = self.futures_private()?;
        private.warm().await?;
        let mut rows = if private.has_position_sample() {
            private.positions(symbol)
        } else {
            kraken_futures_rest::fetch_positions(
                &self.http,
                &self.futures_base_url,
                self.futures_credentials()?,
            )
            .await?
        };
        if let Some(symbol) = symbol {
            rows.retain(|row| row.symbol.eq_ignore_ascii_case(symbol));
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{
        KrakenConfig, KrakenCredentials, KrakenFuturesCredentials, KrakenSpotCredentials,
    };

    #[tokio::test]
    async fn private_stream_handles_survive_between_adapter_calls_and_share_one_session() {
        let config = KrakenConfig {
            credentials: Some(KrakenCredentials {
                spot: Some(KrakenSpotCredentials {
                    api_key: "session-retention".into(),
                    api_secret: "c2VjcmV0".into(),
                }),
                futures: Some(KrakenFuturesCredentials {
                    api_key: "session-retention".into(),
                    api_secret: "c2VjcmV0".into(),
                }),
            }),
            spot_rest_url_override: Some("http://127.0.0.1:9".into()),
            spot_private_ws_url_override: Some("ws://127.0.0.1:9".into()),
            futures_ws_url_override: Some("ws://127.0.0.1:9".into()),
            ..Default::default()
        };
        let adapter = Kraken::new(config.clone()).unwrap();
        let other = Kraken::new(config).unwrap();
        let spot = std::sync::Arc::downgrade(&adapter.spot_private().unwrap());
        let futures = std::sync::Arc::downgrade(&adapter.futures_private().unwrap());
        assert!(std::sync::Arc::ptr_eq(
            &spot.upgrade().unwrap(),
            &other.spot_private().unwrap()
        ));
        assert!(std::sync::Arc::ptr_eq(
            &futures.upgrade().unwrap(),
            &other.futures_private().unwrap()
        ));
        drop(adapter);
        assert!(spot.upgrade().is_some());
        drop(other);
        assert!(spot.upgrade().is_none());
        assert!(futures.upgrade().is_none());
    }

    #[tokio::test]
    async fn capabilities_expose_both_products_without_claiming_live_by_default() {
        let kraken = Kraken::new(KrakenConfig {
            credentials: Some(KrakenCredentials {
                spot: Some(KrakenSpotCredentials {
                    api_key: "s".to_owned(),
                    api_secret: "c2VjcmV0".to_owned(),
                }),
                futures: Some(KrakenFuturesCredentials {
                    api_key: "f".to_owned(),
                    api_secret: "c2VjcmV0".to_owned(),
                }),
            }),
            ..Default::default()
        })
        .unwrap();
        let capabilities = LiveTradingAdapter::capabilities(&kraken);
        assert!(capabilities.supports_spot);
        assert!(capabilities.supports_perp);
        assert!(!capabilities.supports_live);
    }
}
