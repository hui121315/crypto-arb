//! Binance USDT-M 永续合约适配器。
//!
//! 端点参考：[Binance Futures API](https://binance-docs.github.io/apidocs/futures/en/)

use super::binance_exchange_info::ExchangeInfoCache;
use super::binance_format::{build_spot_symbols_param, is_usdm_perp, is_usdt_perp};
use super::binance_funding_info::FundingIntervalCache;
use super::binance_market_data::{
    book_ticker_index, parse_depth_levels, parse_funding, parse_index_constituents,
    parse_spot_tick, parse_ticker, snap_binance_depth, spot_symbol_matches,
};
use super::binance_private_data::BinancePositionMode;
use super::binance_private_rest as private_rest;
use super::binance_public_rest as public_rest;
use super::binance_trade_data::{
    param_refs, rest_cancel_order_params, rest_place_order_params, safe_cancel_probe_params,
    validate_order_spec,
};
use super::binance_ws_market::MarketStream as WsMarketStream;
use super::binance_ws_trade;
use super::spot_order_contract;
use crate::adapter::{
    strip_common_suffixes, ExchangeAdapter, MetadataRefreshOutcome, PublicWsSnapshot,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{venue_balance_rows, ExchangeCapabilities, LiveTradingAdapter, VenueAccountRead};
use crate::services::RateLimiter;
use async_trait::async_trait;
use common::time::now_ms;
use shared_types::instrument_registry::VenueInstrument;
use shared_types::{
    BalanceInfo, CancelOrderRequest, ExecutionMode, FeeProduct, FundingPaymentData,
    FundingRateData, IndexCompositionSnapshot, MarginMode, MarkIndexInfo, OrderAck, OrderBookInfo,
    OrderInfo, OrderIntent, OrderSide, OrderSource, OrderSubmissionContext, OrderType,
    PositionInfo, SpotTick, TickerInfo, TimeInForce, VenueAccountModeInfo, VenueBalanceInfo,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

pub use crate::adapters::binance_config::{BinanceConfig, BinanceCredentials};

const NAME: &str = "binance";
const POSITION_MODE_UNKNOWN: i64 = -1;
const POSITION_MODE_CACHE_TTL_MS: i64 = 30_000;
const POSITION_MODE_SOURCE_ENDPOINT: i64 = 1;
const POSITION_MODE_SOURCE_POSITIONS: i64 = 2;
const SAFE_ORDER_TEST_SYMBOL: &str = "BTC";
const SAFE_ORDER_TEST_QUANTITY: f64 = 0.001;
const SAFE_ORDER_TEST_PRICE: f64 = 100_000.0;

#[derive(Debug)]
pub struct Binance {
    config: BinanceConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    /// 修复 P1 1.4：exchangeInfo cache（6h TTL）。
    exchange_info_cache: ExchangeInfoCache,
    /// 修复 P1 1.1：`funding_interval` cache（24h TTL）。
    funding_intervals: FundingIntervalCache,
    /// 修复 P2 1.11：服务器时间偏移（ms），`server_time - local_time`。
    time_offset_ms: AtomicI64,
    /// 修复 P2 1.11：上次校时本地时间（用于决定是否刷新）。
    time_synced_at_ms: AtomicI64,
    /// Binance position mode is account-wide for USD-M futures; cache briefly to avoid
    /// adding a high-weight account read to every hot write.
    position_mode_code: AtomicI64,
    position_mode_source_code: AtomicI64,
    position_mode_fetched_at_ms: AtomicI64,
    market_stream: OnceLock<Arc<WsMarketStream>>,
    /// 已上市 USDT-M 合约符号集（来自 REST 全量 ticker 应答，零额外请求）。
    /// `to_exchange_symbol` 用它在千倍族候选（SHIB→1000SHIBUSDT）中挑真实
    /// 存在的合约；集合为空（冷启动）时退回首候选，与历史行为一致。
    listed_usdm: Arc<dashmap::DashMap<String, ()>>,
}

#[async_trait]
impl LiveTradingAdapter for Binance {
    fn name(&self) -> &'static str {
        NAME
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_testnet: self.config.testnet,
            supports_live: !self.config.testnet && self.config.allow_live_writes,
            supports_spot: true,
            supports_perp: true,
            supports_limit_orders: true,
            supports_market_orders: true,
            supports_post_only: true,
            supports_reduce_only: true,
        }
    }

    async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "binance time sync failed; using cached offset");
        }
        Ok(Some(self.account_mode_info().await?))
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        // 修复 P2 1.11：下单前校时（5min TTL，多数情况是 noop），失败不阻塞。
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "binance time sync failed; using cached offset");
        }

        let spec = self.instrument_spec(&intent.symbol).await?;
        validate_order_spec(intent, &spec)?;
        let symbol = spec.native_symbol;
        let position_side = self.verified_position_side(intent).await?;
        if self.use_ws_request_api() {
            return binance_ws_trade::place_order(
                self.ws_trade_config()?,
                intent,
                &symbol,
                position_side,
            )
            .await;
        }

        let params = rest_place_order_params(intent, &symbol, position_side)?;
        let param_refs = param_refs(&params);
        let (signed_query, api_key) = self.signed_query(&param_refs)?;
        private_rest::place_order(
            &self.http,
            &self.base_url,
            &signed_query,
            &api_key,
            intent.id.clone(),
            intent.client_order_id.clone(),
        )
        .await
    }

    async fn place_order_with_context(
        &self,
        intent: &OrderIntent,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        if context.product != FeeProduct::Spot {
            return self.place_order(intent).await;
        }
        self.ensure_write_adapter()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "binance time sync failed; using cached offset");
        }
        let compiled = spot_order_contract::compile(NAME, intent, context)?;
        if !self.use_ws_request_api() {
            return Err(ExchangeError::NotImplemented(
                "binance spot REST override write fixture",
            ));
        }
        binance_ws_trade::place_spot_order(
            self.spot_ws_trade_config()?,
            intent,
            &compiled.native_symbol,
        )
        .await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;

        let symbol = self.instrument_spec(&request.symbol).await?.native_symbol;
        if self.use_ws_request_api() {
            return binance_ws_trade::cancel_order(self.ws_trade_config()?, request, &symbol).await;
        }

        let params = rest_cancel_order_params(&request.client_order_id, &symbol);
        let param_refs = param_refs(&params);
        let (signed_query, api_key) = self.signed_query(&param_refs)?;
        private_rest::cancel_order(
            &self.http,
            &self.base_url,
            &signed_query,
            &api_key,
            request.internal_order_id.clone(),
            request.client_order_id.clone(),
        )
        .await
    }

    async fn cancel_order_with_context(
        &self,
        request: &CancelOrderRequest,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        if context.product != FeeProduct::Spot {
            return self.cancel_order(request).await;
        }
        self.ensure_write_adapter()?;
        let symbol = spot_order_contract::cancel_symbol(NAME, request, context)?;
        if !self.use_ws_request_api() {
            return Err(ExchangeError::NotImplemented(
                "binance spot REST override cancel fixture",
            ));
        }
        binance_ws_trade::cancel_spot_order(self.spot_ws_trade_config()?, request, &symbol).await
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        self.prepare_private_request().await?;

        let exchange_symbol = self.instrument_spec(symbol).await?.native_symbol;
        if self.use_ws_request_api() {
            match binance_ws_trade::get_order(
                self.ws_trade_config()?,
                &exchange_symbol,
                client_order_id,
            )
            .await
            {
                Ok(order) => return Ok(order),
                Err(error) => tracing::warn!(
                    %error,
                    operation = "order.status",
                    "binance ws read failed; falling back to REST"
                ),
            }
        }
        let params = [
            ("symbol", exchange_symbol.as_str()),
            ("origClientOrderId", client_order_id),
            ("recvWindow", "5000"),
        ];
        let (signed_query, api_key) = self.signed_query(&params)?;
        private_rest::get_order(&self.http, &self.base_url, &signed_query, &api_key).await
    }

    async fn get_order_with_context(
        &self,
        symbol: &str,
        client_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        if context.product != FeeProduct::Spot {
            return self.get_order(symbol, client_order_id).await;
        }
        self.ensure_write_adapter()?;
        let native = spot_order_contract::query_symbol(NAME, symbol, context)?;
        binance_ws_trade::get_spot_order(self.spot_ws_trade_config()?, &native, client_order_id)
            .await
    }

    async fn get_order_by_exchange_order_id_with_context(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        if context.product != FeeProduct::Spot {
            return self
                .get_order_by_exchange_order_id(symbol, exchange_order_id)
                .await;
        }
        self.ensure_write_adapter()?;
        let native = spot_order_contract::query_symbol(NAME, symbol, context)?;
        binance_ws_trade::get_spot_order_by_id(
            self.spot_ws_trade_config()?,
            &native,
            exchange_order_id,
        )
        .await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        <Self as ExchangeAdapter>::get_open_orders(self, symbol).await
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        let balances = <Self as ExchangeAdapter>::get_balance(self, currency).await?;
        Ok(venue_balance_rows(NAME, balances))
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        self.prepare_private_request().await?;
        if self.use_ws_request_api() {
            match binance_ws_trade::account_read(self.ws_trade_config()?, currency, now_ms()).await
            {
                Ok(read) => return Ok(read),
                Err(error) => tracing::warn!(
                    %error,
                    operation = "v2/account.status",
                    "binance ws read failed; falling back to REST"
                ),
            }
        }
        let (signed_query, api_key) = self.signed_query(&[])?;
        private_rest::account_read(
            &self.http,
            &self.base_url,
            &signed_query,
            &api_key,
            currency,
            now_ms(),
        )
        .await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        <Self as ExchangeAdapter>::get_positions(self, symbol).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        <Self as ExchangeAdapter>::get_funding_payments(self, symbol, start_time_ms, end_time_ms)
            .await
    }

    fn withdrawal_submission_supported(&self) -> bool {
        !self.config.testnet
    }

    async fn withdrawal_source_balance(
        &self,
        request: &crate::WithdrawalSourceBalanceRequest,
    ) -> ExchangeResult<crate::WithdrawalSourceBalance> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "binance testnet withdrawal_source_balance",
            ));
        }
        self.prepare_private_request().await?;
        super::binance_withdrawals::source_balance(
            &self.http,
            self.spot_base_url(),
            request,
            |params| self.signed_query(params),
        )
        .await
    }

    async fn submit_withdrawal(
        &self,
        request: &crate::WithdrawalSubmitRequest,
    ) -> ExchangeResult<crate::WithdrawalSubmission> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "binance testnet withdrawal_submit",
            ));
        }
        self.ensure_write_adapter()?;
        self.prepare_private_request().await?;
        super::binance_withdrawals::submit(&self.http, self.spot_base_url(), request, |params| {
            self.signed_query(params)
        })
        .await
    }

    async fn withdrawal_status(
        &self,
        request: &crate::WithdrawalStatusRequest,
    ) -> ExchangeResult<Option<crate::WithdrawalStatusEvidence>> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "binance testnet withdrawal_status",
            ));
        }
        self.prepare_private_request().await?;
        super::binance_withdrawals::status(&self.http, self.spot_base_url(), request, |params| {
            self.signed_query(params)
        })
        .await
    }

    fn deposit_status_supported(&self) -> bool {
        !self.config.testnet
    }

    async fn deposit_status(
        &self,
        request: &crate::DepositStatusRequest,
    ) -> ExchangeResult<Option<crate::DepositStatusEvidence>> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "binance testnet deposit_status",
            ));
        }
        self.prepare_private_request().await?;
        super::binance_deposits::status(&self.http, self.spot_base_url(), request, |params| {
            self.signed_query(params)
        })
        .await
    }
}

impl Binance {
    async fn prepare_private_request(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "binance time sync failed; using cached offset");
        }
        Ok(())
    }

    pub async fn validate_safe_order_place_test_permission(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "binance time sync failed; using cached offset");
        }
        let intent = Self::safe_order_test_intent();
        let spec = self.instrument_spec(&intent.symbol).await?;
        validate_order_spec(&intent, &spec)?;
        let symbol = spec.native_symbol;
        let position_side = self.verified_position_side(&intent).await?;
        let params = rest_place_order_params(&intent, &symbol, position_side)?;
        let param_refs = param_refs(&params);
        private_rest::test_order(&self.http, &self.base_url, || {
            self.signed_query(&param_refs)
        })
        .await
    }

    pub async fn validate_safe_order_place_cancel_test_permission(&self) -> ExchangeResult<()> {
        self.validate_safe_order_place_test_permission().await?;
        let client_order_id = format!("xline-cancel-probe-{}", now_ms());
        let symbol = self
            .instrument_spec(SAFE_ORDER_TEST_SYMBOL)
            .await?
            .native_symbol;
        let params = safe_cancel_probe_params(&client_order_id, &symbol);
        let param_refs = param_refs(&params);
        private_rest::safe_cancel_probe(&self.http, &self.base_url, || {
            self.signed_query(&param_refs)
        })
        .await
    }

    fn safe_order_test_intent() -> OrderIntent {
        let now = now_ms();
        let client_order_id = format!("xline-test-{now}");
        OrderIntent {
            id: client_order_id.clone(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: NAME.to_owned(),
            symbol: SAFE_ORDER_TEST_SYMBOL.to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: SAFE_ORDER_TEST_QUANTITY,
            price: Some(SAFE_ORDER_TEST_PRICE),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id,
            client_order_id_policy: None,
            created_at_ms: now,
        }
    }

    async fn account_mode_info(&self) -> ExchangeResult<VenueAccountModeInfo> {
        let mode = self.position_mode().await?;
        let checked_at_ms = self.position_mode_fetched_at_ms.load(Ordering::Relaxed);
        let freshness_ms = now_ms()
            .checked_sub(checked_at_ms)
            .and_then(|value| u64::try_from(value).ok());
        Ok(VenueAccountModeInfo {
            venue: NAME.to_owned(),
            mode: mode.as_str().to_owned(),
            source: self.position_mode_source().to_owned(),
            checked_at_ms,
            freshness_ms,
            account_scope: Some("usds_m_futures".to_owned()),
        })
    }

    async fn verified_position_side(&self, intent: &OrderIntent) -> ExchangeResult<&'static str> {
        let mode = self.position_mode().await?;
        Ok(mode.position_side_for_intent(intent))
    }

    async fn position_mode(&self) -> ExchangeResult<BinancePositionMode> {
        if let Some(mode) = self.cached_position_mode() {
            return Ok(mode);
        }
        let params = [("recvWindow", "5000")];
        let mode = binance_account_mode::position_mode(&self.http, &self.base_url, || {
            self.signed_query(&params)
        })
        .await?;
        self.cache_position_mode(mode, POSITION_MODE_SOURCE_ENDPOINT);
        Ok(mode)
    }

    fn cache_position_mode(&self, mode: BinancePositionMode, source: i64) {
        self.position_mode_code
            .store(mode.code(), Ordering::Relaxed);
        self.position_mode_source_code
            .store(source, Ordering::Relaxed);
        self.position_mode_fetched_at_ms
            .store(now_ms(), Ordering::Release);
    }

    fn position_mode_source(&self) -> &'static str {
        match self.position_mode_source_code.load(Ordering::Relaxed) {
            POSITION_MODE_SOURCE_POSITIONS => "binance.GET /fapi/v3/positionRisk.positionSide",
            _ => "binance.GET /fapi/v1/positionSide/dual",
        }
    }

    fn cached_position_mode(&self) -> Option<BinancePositionMode> {
        let fetched_at = self.position_mode_fetched_at_ms.load(Ordering::Acquire);
        let fresh =
            fetched_at != 0 && now_ms().saturating_sub(fetched_at) <= POSITION_MODE_CACHE_TTL_MS;
        if !fresh {
            return None;
        }
        BinancePositionMode::from_code(self.position_mode_code.load(Ordering::Relaxed))
    }
}

// ============== Trait 实现 ==============

#[async_trait]
impl ExchangeAdapter for Binance {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
        self.refresh_funding_intervals().await?;
        Ok(MetadataRefreshOutcome::Refreshed)
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let checked_at_ms = now_ms();
        let (perpetuals, spot) = tokio::try_join!(
            self.fetch_instruments_inner(),
            super::spot_instruments::binance(
                &self.http,
                self.config
                    .base_url_override
                    .as_deref()
                    .unwrap_or(super::binance_config::SPOT_PROD_BASE),
                checked_at_ms,
            ),
        )?;
        Ok(perpetuals.into_iter().chain(spot).collect())
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        super::spot_instruments::binance(
            &self.http,
            self.config
                .base_url_override
                .as_deref()
                .unwrap_or(super::binance_config::SPOT_PROD_BASE),
            now_ms(),
        )
        .await
    }

    async fn fetch_transfer_networks(&self) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "binance testnet transfer_networks",
            ));
        }
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "binance time sync failed before transfer metadata read");
        }
        super::binance_transfer_networks::fetch(&self.http, self.spot_base_url(), || {
            self.signed_query(&[])
        })
        .await
    }

    async fn fetch_transfer_destination(
        &self,
        request: &crate::TransferDestinationRequest,
    ) -> ExchangeResult<crate::TransferDestinationEvidence> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "binance testnet transfer_destination",
            ));
        }
        self.prepare_private_request().await?;
        super::binance_transfer_destinations::fetch(
            &self.http,
            self.spot_base_url(),
            request,
            |params| self.signed_query(params),
        )
        .await
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        let exch = self.to_exchange_symbol(symbol);
        self.refresh_funding_intervals().await?;
        let interval = self.funding_interval_for(&exch);
        if let Some(funding) = super::binance_ws_mark::latest_funding(&self.config, &exch, interval)
        {
            return Ok(funding);
        }
        let item = public_rest::funding_rate(&self.http, &self.base_url, &exch).await?;
        parse_funding(&item, 0.0, interval).ok_or_else(|| {
            ExchangeError::Parse(format!(
                "binance funding {exch} missing rate or next funding time"
            ))
        })
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        self.refresh_funding_intervals().await?;
        if let Some(rows) = self.ws_funding_snapshot(symbols) {
            return Ok(rows);
        }

        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });

        let (premiums, tickers) =
            public_rest::funding_rates_and_tickers(&self.http, &self.base_url).await?;

        let volume_map: HashMap<String, f64> = tickers
            .into_iter()
            .filter_map(|t| t.quote_volume.parse::<f64>().ok().map(|v| (t.symbol, v)))
            .collect();

        let rates: Vec<FundingRateData> = premiums
            .into_iter()
            .filter(|p| include_discovery_perp(&p.symbol, requested.as_ref()))
            .filter_map(|p| {
                let v24 = volume_map.get(&p.symbol).copied().unwrap_or(0.0);
                let interval = self.funding_interval_for(&p.symbol);
                parse_funding(&p, v24, interval)
            })
            .collect();

        Ok(rates)
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        let exch = self.to_exchange_symbol(symbol);
        if let Some(row) = self.ws_ticker(&exch) {
            return Ok(row);
        }
        let (item, book) = public_rest::ticker_and_book(&self.http, &self.base_url, &exch).await?;
        let normalized = strip_common_suffixes(&item.symbol);
        parse_ticker(&item, &normalized, Some(&book)).ok_or_else(|| {
            ExchangeError::Parse(format!("binance ticker missing required price for {exch}"))
        })
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        if let Some(rows) = self.ws_ticker_snapshot(symbols) {
            return Ok(rows);
        }
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let (tickers, books) = public_rest::tickers_and_books(&self.http, &self.base_url).await?;
        self.note_listed_usdm(tickers.iter().map(|t| t.symbol.as_str()));
        let books = book_ticker_index(books);
        let result: Vec<TickerInfo> = tickers
            .into_iter()
            .filter(|t| include_discovery_perp(&t.symbol, requested.as_ref()))
            .filter_map(|t| {
                let sym = strip_common_suffixes(&t.symbol);
                parse_ticker(&t, &sym, books.get(&t.symbol))
            })
            .collect();
        Ok(result)
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        Ok(self
            .ws_ticker_snapshot(Some(symbols))
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        self.refresh_funding_intervals().await?;
        Ok(self
            .ws_funding_snapshot(Some(symbols))
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        Ok(self
            .ws_mark_index_snapshot(Some(symbols))
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        let rows = self
            .ws_spot_tick_snapshot(Some(symbols))
            .unwrap_or_default();
        let rows =
            super::binance_spot_ws_snapshot::supplement_spot_ticks(&self.config, symbols, rows)
                .await?;
        Ok(if rows.is_empty() {
            PublicWsSnapshot::Pending
        } else {
            PublicWsSnapshot::Ready(rows)
        })
    }

    fn public_ws_spot_problem(&self, symbol: &str) -> Option<String> {
        super::binance_ws_spot_ticker::spot_problem(&self.config, symbol)
            .map(|problem| format!("Binance Spot WS：{problem}"))
    }

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        if let Some(rows) = self.ws_mark_index_snapshot(symbols) {
            return self.enrich_open_interest(rows, symbols).await;
        }
        self.rest_mark_index_prices(symbols).await
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let exchange_symbol = self.to_exchange_symbol(symbol);
        let (body, evidence) =
            public_rest::index_constituents(&self.http, &self.base_url, &exchange_symbol).await?;
        Ok(crate::adapter::attach_payload_evidence(
            parse_index_constituents(body),
            evidence,
        ))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        if let Some(rows) = self.ws_spot_tick_snapshot(symbols) {
            return Ok(rows);
        }
        let symbols_param = symbols.and_then(build_spot_symbols_param);
        let tickers =
            public_rest::spot_tickers(&self.http, self.spot_base_url(), symbols_param.as_deref())
                .await?;
        Ok(tickers
            .into_iter()
            .filter(|item| spot_symbol_matches(&item.symbol, symbols))
            .filter_map(|item| parse_spot_tick(&item))
            .collect())
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let exch = self.to_exchange_symbol(symbol);
        if let Some(mut book) = self.ws_orderbook(&exch) {
            if depth > 0 {
                let cap = depth as usize;
                book.bids.truncate(cap);
                book.asks.truncate(cap);
            }
            return Ok(book);
        }
        let limit = snap_binance_depth(depth).to_string();
        let body = public_rest::orderbook(&self.http, &self.base_url, &exch, &limit).await?;
        Ok(OrderBookInfo {
            symbol: strip_common_suffixes(&exch),
            exchange: NAME.into(),
            bids: parse_depth_levels(body.bids),
            asks: parse_depth_levels(body.asks),
            timestamp: now_ms(),
        })
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let exch = self.to_exchange_symbol(symbol);
        Ok(match self.ws_orderbook(&exch) {
            Some(mut book) => {
                if depth > 0 {
                    let cap = depth as usize;
                    book.bids.truncate(cap);
                    book.asks.truncate(cap);
                }
                PublicWsSnapshot::Ready(vec![book])
            }
            None => PublicWsSnapshot::Pending,
        })
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        if let Some(book) =
            super::binance_ws_spot_depth::latest_spot_orderbook(&self.config, symbol, depth)
        {
            return Ok(book);
        }
        let exch = crate::spot::compact_pair_symbol(symbol)
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        // Official: <https://developers.binance.com/docs/binance-spot-api-docs/rest-api/market-data-endpoints#order-book>
        let limit = snap_binance_depth(depth).to_string();
        let body =
            public_rest::spot_orderbook(&self.http, self.spot_base_url(), &exch, &limit).await?;
        Ok(OrderBookInfo {
            symbol: crate::spot::native_pair_symbol(&exch, '/').unwrap_or(exch),
            exchange: NAME.into(),
            bids: parse_depth_levels(body.bids),
            asks: parse_depth_levels(body.asks),
            timestamp: now_ms(),
        })
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(
            match super::binance_ws_spot_depth::latest_spot_orderbook(&self.config, symbol, depth) {
                Some(book) => PublicWsSnapshot::Ready(vec![book]),
                None => PublicWsSnapshot::Pending,
            },
        )
    }

    async fn get_balance(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        self.prepare_private_request().await?;
        if self.use_ws_request_api() {
            match binance_ws_trade::balances(self.ws_trade_config()?, currency).await {
                Ok(balances) => return Ok(balances),
                Err(error) => tracing::warn!(
                    %error,
                    operation = "v2/account.balance",
                    "binance ws read failed; falling back to REST"
                ),
            }
        }
        let (signed_query, api_key) = self.signed_query(&[])?;
        private_rest::balances(
            &self.http,
            &self.base_url,
            &signed_query,
            &api_key,
            currency,
        )
        .await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        self.prepare_private_request().await?;
        let exchange_symbol = symbol.map(|value| self.to_exchange_symbol(value));
        let params = super::funding_payments::binance_funding_income_params(
            exchange_symbol.as_deref(),
            start_time_ms,
            end_time_ms,
            1_000,
        );
        let param_refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let (signed_query, api_key) = self.signed_query(&param_refs)?;
        private_rest::funding_payments(&self.http, &self.base_url, &signed_query, &api_key).await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        self.prepare_private_request().await?;
        let target = symbol.map(|s| self.to_exchange_symbol(s));
        let parsed = if self.use_ws_request_api() {
            match binance_ws_trade::positions(self.ws_trade_config()?, target.as_deref()).await {
                Ok(parsed) => parsed,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        operation = "v2/account.position",
                        "binance ws read failed; falling back to REST"
                    );
                    let (signed_query, api_key) = self.signed_query(&[])?;
                    private_rest::positions(
                        &self.http,
                        &self.base_url,
                        &signed_query,
                        &api_key,
                        target.as_deref(),
                    )
                    .await?
                }
            }
        } else {
            let (signed_query, api_key) = self.signed_query(&[])?;
            private_rest::positions(
                &self.http,
                &self.base_url,
                &signed_query,
                &api_key,
                target.as_deref(),
            )
            .await?
        };
        if let Some(mode) = parsed.mode {
            // Binance documents `positionSide` on Position Information V3. Portfolio reads
            // already refresh this account-wide fact, so live closes need no extra hot-path
            // position-mode request in the normal case.
            self.cache_position_mode(mode, POSITION_MODE_SOURCE_POSITIONS);
        }
        Ok(parsed.rows)
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        let exch_sym;
        let params: Vec<(&str, &str)> = if let Some(s) = symbol {
            exch_sym = self.to_exchange_symbol(s);
            vec![("symbol", exch_sym.as_str())]
        } else {
            vec![]
        };

        let (signed_query, api_key) = self.signed_query(&params)?;
        private_rest::open_orders(&self.http, &self.base_url, &signed_query, &api_key).await
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        strip_common_suffixes(symbol)
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        let candidates = super::binance_format::usdm_symbol_candidates(symbol);
        if !self.listed_usdm.is_empty() {
            if let Some(hit) = candidates
                .iter()
                .find(|candidate| self.listed_usdm.contains_key(*candidate))
            {
                return hit.clone();
            }
        }
        candidates.into_iter().next().unwrap_or_default()
    }
}

fn include_discovery_perp(symbol: &str, requested: Option<&HashSet<String>>) -> bool {
    is_usdm_perp(symbol)
        && requested.map_or_else(|| is_usdt_perp(symbol), |symbols| symbols.contains(symbol))
}

#[path = "binance_ws_cache.rs"]
mod ws_cache;

#[path = "binance_account_mode.rs"]
mod binance_account_mode;

#[path = "binance_trade_support.rs"]
mod trade_support;

#[path = "binance_market_reads.rs"]
mod market_reads;

#[path = "binance_support.rs"]
mod support;

#[cfg(test)]
#[path = "binance_tests.rs"]
mod tests;
