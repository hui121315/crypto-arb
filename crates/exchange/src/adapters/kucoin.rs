//! KuCoin Futures USDT-margined 永续适配器。
//!
//! V1 范围：公共行情 + 私有读接口。
//!
//! ## 协议特点
//! - Symbol 特殊：BTC 用 `XBT` 表示（KuCoin 沿用 Bitcoin 老符号），永续后缀 `M`，例 `XBTUSDTM`
//! - 响应 `code = "200000"` 为成功（不是 `"0"`）
//! - `passphrase` 必须用 `api_secret` 二次 HMAC-SHA256 + Base64 加密后才能放头部
//! - `/api/v1/contracts/active` 提供费率 + 24h 成交量；`/api/v1/allTickers` 提供可成交盘口。

use crate::adapter::{
    unverified_index_composition, ExchangeAdapter, MetadataRefreshOutcome, PublicWsSnapshot,
};
use crate::adapters::contract_orderbook::normalize_contract_book;
pub use crate::adapters::kucoin_config::{KucoinConfig, KucoinCredentials, KucoinMarginMode};
use crate::adapters::kucoin_contracts::KucoinContractMultipliers;
use crate::adapters::kucoin_market_data::{
    kucoin_to_normalized, normalized_to_kucoin, parse_funding, parse_spot_depth_levels,
    parse_spot_tick, parse_ticker, snap_kucoin_depth_endpoint, spot_symbol_matches,
};
use crate::adapters::kucoin_private_data::{KucoinPositionMode, NativePositionInfo};
use crate::adapters::kucoin_private_rest as private_rest;
use crate::adapters::kucoin_public_rest as public_rest;
use crate::adapters::kucoin_spot_trade_data as spot_trade_data;
use crate::adapters::kucoin_spot_ws_trade as spot_ws_trade;
use crate::adapters::kucoin_trade_data::{
    ack_from_order_query, checked_client_oid, get_order_by_client_oid_path,
    get_order_by_order_id_path, place_order_body_json, safe_cancel_probe_path,
};
use crate::adapters::kucoin_ws_market::MarketStream as WsMarketStream;
use crate::adapters::spot_order_contract;
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

const PROD_BASE: &str = "https://api-futures.kucoin.com";
const SPOT_PROD_BASE: &str = "https://api.kucoin.com";
const NAME: &str = "kucoin";
const POSITION_MODE_PATH: &str = "/api/v2/position/getPositionMode";
const POSITION_DETAIL_PATH: &str = "/api/v2/position";
const POSITION_DETAIL_DOC_URL: &str =
    "https://www.kucoin.com/docs-new/rest/futures-trading/positions/get-position-details";
const POSITION_MODE_UNKNOWN: i64 = -1;
const POSITION_MODE_CACHE_TTL_MS: i64 = 30_000;
const SAFE_ORDER_TEST_SYMBOL: &str = "BTC";
const SAFE_ORDER_TEST_QUANTITY: f64 = 0.001;
const SAFE_ORDER_TEST_PRICE: f64 = 100_000.0;
pub(super) const PLACE_ORDER_PATH: &str = "/api/v1/orders";
pub(super) const TEST_ORDER_PATH: &str = "/api/v1/orders/test";
pub(super) const OPEN_ORDERS_PATH: &str = "/api/v1/orders";

/// 修复 P2 7.x：KuCoin 签名容差 ~30s。5min TTL 在常见 NTP 慢漂下足够保险。
/// 文档：<https://www.kucoin.com/docs/rest/futures-trading/general/get-server-time>
const TIME_SYNC_INTERVAL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug)]
pub struct Kucoin {
    config: KucoinConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    /// 修复 P2 7.x：服务器时间偏移（毫秒），`server_ms - local_ms`。
    time_offset_ms: AtomicI64,
    /// 修复 P2 7.x：上次校时本地时间（毫秒）。
    time_synced_at_ms: AtomicI64,
    /// KuCoin position mode is account-wide for futures; cache it briefly to keep the write path lean.
    position_mode_code: AtomicI64,
    position_mode_fetched_at_ms: AtomicI64,
    contract_multipliers: KucoinContractMultipliers,
    market_stream: OnceLock<Option<Arc<WsMarketStream>>>,
}

#[async_trait]
impl LiveTradingAdapter for Kucoin {
    fn name(&self) -> &'static str {
        NAME
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

    fn order_margin_modes(&self) -> Vec<MarginMode> {
        vec![MarginMode::Cross, MarginMode::Isolated]
    }

    async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.require_credentials()?;
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        Ok(Some(self.account_mode_info().await?))
    }

    async fn preflight_order(&self, _exchange: &str, intent: &OrderIntent) -> ExchangeResult<()> {
        self.ensure_write_adapter()?;
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        let (symbol, _) = self.prepare_live_order(intent).await?;
        self.ensure_position_compatibility(intent, &symbol).await
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        let (symbol, body) = self.prepare_live_order(intent).await?;
        self.ensure_position_compatibility(intent, &symbol).await?;
        let path = PLACE_ORDER_PATH;
        let headers = self.build_signed_headers("POST", path, &body)?;
        let result = private_rest::place_order(
            &self.signed_request(path, &headers),
            body,
            intent.id.clone(),
            intent.client_order_id.clone(),
        )
        .await;
        match result {
            Ok(ack) => Ok(ack),
            Err(error) if private_rest::is_ambiguous_place_result(&error) => {
                match LiveTradingAdapter::get_order(self, &intent.symbol, &intent.client_order_id)
                    .await
                {
                    Ok(Some(order)) => Ok(ack_from_order_query(
                        intent.id.clone(),
                        intent.client_order_id.clone(),
                        order,
                    )),
                    Ok(None) => Err(error),
                    Err(query_error) => {
                        tracing::warn!(
                            %query_error,
                            client_order_id = %intent.client_order_id,
                            "kucoin ambiguous place result could not be confirmed by order query"
                        );
                        Err(error)
                    }
                }
            }
            Err(error) => Err(error),
        }
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
            tracing::warn!(%error, "kucoin time sync failed; using cached offset");
        }
        let compiled = spot_order_contract::compile(NAME, intent, context)?;
        spot_ws_trade::place_order(self.spot_ws_trade_config()?, intent, &compiled).await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        let symbol = self.contract_native_symbol(&request.symbol).await?;
        let target = private_rest::cancel_request_target(request, &symbol)?;
        let headers = self.build_signed_headers("DELETE", &target.request.signing_path, "")?;
        private_rest::cancel_order(
            &self.signed_request(&target.request.wire_path, &headers),
            request,
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
        let native_symbol = spot_order_contract::cancel_symbol(NAME, request, context)?;
        spot_ws_trade::cancel_order(self.spot_ws_trade_config()?, request, &native_symbol).await
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "kucoin: server time sync failed; falling back to local clock");
        }
        let venue_client_order_id = checked_client_oid(client_order_id)?;
        let path = get_order_by_client_oid_path(client_order_id)?;
        self.query_order(symbol, &path, Some(&venue_client_order_id))
            .await
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
        let native_symbol = spot_order_contract::query_symbol(NAME, symbol, context)?;
        let path = spot_trade_data::query_by_client_path(client_order_id, &native_symbol)?;
        self.query_spot_order(&path).await
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
        let native_symbol = spot_order_contract::query_symbol(NAME, symbol, context)?;
        let path = spot_trade_data::query_by_order_path(exchange_order_id, &native_symbol)?;
        self.query_spot_order(&path).await
    }

    async fn get_order_by_exchange_order_id(
        &self,
        symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "kucoin: server time sync failed; falling back to local clock");
        }
        let path = get_order_by_order_id_path(exchange_order_id)?;
        let Some(order) = self.query_order(symbol, &path, None).await? else {
            return Ok(None);
        };
        if order.order_id != exchange_order_id.trim() {
            return Err(ExchangeError::Parse(format!(
                "kucoin get order orderId mismatch: requested={exchange_order_id:?} response={:?}",
                order.order_id
            )));
        }
        Ok(Some(order))
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        ExchangeAdapter::get_open_orders(self, symbol).await
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        let balances = ExchangeAdapter::get_balance(self, currency).await?;
        Ok(venue_balance_rows(NAME, balances))
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "kucoin: server time sync failed; falling back to local clock");
        }
        let currency = currency.unwrap_or("USDT");
        let path = format!("/api/v1/account-overview?currency={currency}");
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::account_read(&self.signed_request(&path, &headers), currency, now_ms()).await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        ExchangeAdapter::get_positions(self, symbol).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        ExchangeAdapter::get_funding_payments(self, symbol, start_time_ms, end_time_ms).await
    }
}

impl Kucoin {
    fn spot_ws_trade_config(&self) -> ExchangeResult<spot_ws_trade::WsSpotTradeConfig<'_>> {
        if self.config.base_url_override.is_some() {
            return Err(ExchangeError::NotImplemented(
                "kucoin Pro Spot WebSocket override",
            ));
        }
        let credentials = self.require_credentials()?;
        Ok(spot_ws_trade::WsSpotTradeConfig {
            api_key: &credentials.api_key,
            api_secret: &credentials.api_secret,
            passphrase: &credentials.passphrase,
            timeout_secs: self.config.timeout_secs,
            time_offset_ms: self.time_offset_ms.load(Ordering::Relaxed),
        })
    }

    async fn query_spot_order(&self, path: &str) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "kucoin time sync failed; using cached offset");
        }
        let headers = self.build_signed_headers("GET", path, "")?;
        match spot_trade_data::query_order(&self.signed_spot_request(path, &headers)).await {
            Ok(order) => Ok(Some(order)),
            Err(error) if kucoin_spot_order_not_found(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn query_order(
        &self,
        symbol: &str,
        path: &str,
        expected_client_oid: Option<&str>,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let headers = self.build_signed_headers("GET", path, "")?;
        let order = match private_rest::get_order_row(&self.signed_request(path, &headers)).await {
            Ok(order) => order,
            Err(error) if kucoin_order_not_found(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Some(expected_client_oid) = expected_client_oid {
            private_rest::ensure_order_client_oid(&order, expected_client_oid)?;
        }
        let parsed = if let Some(order_id) = private_rest::fill_lookup_order_id(&order) {
            let target = private_rest::fills_request_target(order_id)?;
            let headers = self.build_signed_headers("GET", &target.signing_path, "")?;
            let fills =
                private_rest::fills(&self.signed_request(&target.wire_path, &headers), order_id)
                    .await?;
            private_rest::order_with_fills(&order, &fills)?
        } else {
            private_rest::order_with_fills(&order, &[])?
        };
        let unit = self.contract_order_unit(symbol).await?;
        normalize_kucoin_order_contracts(parsed, unit).map(Some)
    }

    async fn normalize_order_rows(&self, rows: Vec<OrderInfo>) -> ExchangeResult<Vec<OrderInfo>> {
        let mut normalized = Vec::with_capacity(rows.len());
        for row in rows {
            let unit = self.contract_order_unit(&row.symbol).await?;
            normalized.push(normalize_kucoin_order_contracts(row, unit)?);
        }
        Ok(normalized)
    }

    async fn normalize_position_rows(
        &self,
        rows: Vec<NativePositionInfo>,
    ) -> ExchangeResult<Vec<PositionInfo>> {
        let mut normalized = Vec::with_capacity(rows.len());
        for row in rows {
            let unit = self.contract_order_unit(&row.native_symbol).await?;
            normalized.push(normalize_kucoin_position_contracts(row.position, unit)?);
        }
        Ok(normalized)
    }

    async fn prepare_live_order(&self, intent: &OrderIntent) -> ExchangeResult<(String, String)> {
        let position_side = self.verified_position_side(intent).await?;
        let symbol = self.contract_native_symbol(&intent.symbol).await?;
        let unit = self.contract_order_unit(&symbol).await?;
        let body = place_order_body_json(intent, symbol.clone(), unit, position_side)?;
        Ok((symbol, body))
    }

    async fn ensure_position_compatibility(
        &self,
        intent: &OrderIntent,
        native_symbol: &str,
    ) -> ExchangeResult<()> {
        let path = format!("{POSITION_DETAIL_PATH}?symbol={native_symbol}");
        let headers = self.build_signed_headers("GET", &path, "")?;
        let positions =
            private_rest::positions(&self.signed_request(&path, &headers), Some(native_symbol))
                .await?;
        for position in positions {
            ensure_position_matches_intent(&position.position, intent)?;
        }
        Ok(())
    }

    pub async fn validate_safe_order_place_test_permission(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "kucoin: server time sync failed; falling back to local clock");
        }
        let intent = Self::safe_order_test_intent();
        let position_side = self.verified_position_side(&intent).await?;
        let symbol = self.contract_native_symbol(&intent.symbol).await?;
        let unit = self.contract_order_unit(&symbol).await?;
        let body = place_order_body_json(&intent, symbol, unit, position_side)?;
        let path = TEST_ORDER_PATH;
        let headers = self.build_signed_headers("POST", path, &body)?;
        private_rest::test_order(&self.signed_request(path, &headers), body).await
    }

    pub async fn validate_safe_order_place_cancel_test_permission(&self) -> ExchangeResult<()> {
        self.validate_safe_order_place_test_permission().await?;
        let client_order_id = format!("xline-cancel-probe-{}", now_ms());
        let symbol = self.contract_native_symbol(SAFE_ORDER_TEST_SYMBOL).await?;
        let path = safe_cancel_probe_path(&client_order_id, &symbol);
        let headers = self.build_signed_headers("DELETE", &path, "")?;
        private_rest::safe_cancel_probe(&self.signed_request(&path, &headers)).await
    }

    pub async fn actual_fee_rates(&self, symbol: &str) -> ExchangeResult<(f64, f64)> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "kucoin: server time sync failed; falling back to local clock");
        }
        let symbol = self.contract_native_symbol(symbol).await?;
        let target = private_rest::fee_rate_request_target(&symbol)?;
        let headers = self.build_signed_headers("GET", &target.signing_path, "")?;
        let evidence = private_rest::fee_rate(
            &self.signed_request(&target.wire_path, &headers),
            &symbol,
            now_ms(),
        )
        .await?;
        Ok((evidence.maker_fee_rate, evidence.taker_fee_rate))
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
            source: "kucoin.GET /api/v2/position/getPositionMode".to_owned(),
            checked_at_ms,
            freshness_ms,
            account_scope: Some("classic_futures".to_owned()),
        })
    }

    async fn verified_position_side(&self, intent: &OrderIntent) -> ExchangeResult<&'static str> {
        let mode = self.position_mode().await?;
        mode.position_side_for_intent(intent)
    }

    async fn position_mode(&self) -> ExchangeResult<KucoinPositionMode> {
        if let Some(mode) = self.cached_position_mode() {
            return Ok(mode);
        }
        let headers = self.build_signed_headers("GET", POSITION_MODE_PATH, "")?;
        let mode =
            private_rest::position_mode(&self.signed_request(POSITION_MODE_PATH, &headers)).await?;
        self.position_mode_code
            .store(mode.code(), Ordering::Relaxed);
        self.position_mode_fetched_at_ms
            .store(now_ms(), Ordering::Relaxed);
        Ok(mode)
    }

    async fn funding_payment_symbol(&self, symbol: Option<&str>) -> ExchangeResult<String> {
        let symbol = symbol.ok_or_else(|| {
            ExchangeError::UnsupportedSymbol(
                "kucoin funding history requires an explicit contract symbol".into(),
            )
        })?;
        self.contract_native_symbol(symbol).await
    }

    fn cached_position_mode(&self) -> Option<KucoinPositionMode> {
        let fetched_at = self.position_mode_fetched_at_ms.load(Ordering::Relaxed);
        let fresh =
            fetched_at != 0 && now_ms().saturating_sub(fetched_at) <= POSITION_MODE_CACHE_TTL_MS;
        if !fresh {
            return None;
        }
        KucoinPositionMode::from_code(self.position_mode_code.load(Ordering::Relaxed))
    }
}

fn ensure_position_matches_intent(
    position: &PositionInfo,
    intent: &OrderIntent,
) -> ExchangeResult<()> {
    let expected_mode = match intent.margin_mode {
        MarginMode::Cross => "cross",
        MarginMode::Isolated => "isolated",
    };
    // KuCoin requires a leverage field on every futures order, but an emergency
    // reduce-only order must not be blocked by a stale caller-side leverage
    // value. The venue position remains the authority for the existing risk;
    // leverage equality is only an entry/increase invariant.
    if !intent.reduce_only && (position.leverage - intent.leverage).abs() > 1e-9 {
        return Err(position_compatibility_error(&format!(
            "open position {} leverage={} conflicts with order leverage={}",
            intent.symbol, position.leverage, intent.leverage
        )));
    }
    match position.margin_mode.as_deref().map(str::trim) {
        Some(mode) if mode.eq_ignore_ascii_case(expected_mode) => Ok(()),
        Some(mode) if !mode.is_empty() => Err(position_compatibility_error(&format!(
            "open position {} marginMode={mode} conflicts with order marginMode={expected_mode}",
            intent.symbol
        ))),
        _ => Err(position_compatibility_error(&format!(
            "open position {} is missing marginMode evidence for order marginMode={expected_mode}",
            intent.symbol
        ))),
    }
}

fn position_compatibility_error(message: &str) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.to_owned(),
        code: "position_compatibility".to_owned(),
        message: format!("{message}; source: {POSITION_DETAIL_DOC_URL}"),
    }
}

#[async_trait]
impl ExchangeAdapter for Kucoin {
    fn name(&self) -> &'static str {
        NAME
    }

    /// PR-DP-04 D-2: 冷启动 metadata prewarm。
    /// 调一次 `/api/v1/contracts/active` 把所有有效合约的 multiplier 填进
    /// `KucoinContractMultipliers`（24h TTL），避免首次 `contract_order_unit`
    /// 在 hot path 上逐 symbol 拉 REST。失败由 `Aggregator::refresh_all_metadata`
    /// 转 warn，hot path 仍回退既有 lazy per-symbol 拉取。
    async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
        self.refresh_contract_multipliers().await?;
        Ok(MetadataRefreshOutcome::Refreshed)
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let checked_at_ms = now_ms();
        let spot_base = self
            .config
            .base_url_override
            .as_deref()
            .unwrap_or(SPOT_PROD_BASE);
        let (rows, spot) = tokio::try_join!(
            public_rest::instruments_rest(&self.http, &self.base_url),
            super::spot_instruments::kucoin(&self.http, spot_base, checked_at_ms),
        )?;
        Ok(
            crate::adapters::kucoin_instruments::instruments_from_rows(&rows, checked_at_ms)
                .into_iter()
                .chain(spot)
                .collect(),
        )
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let spot_base = self
            .config
            .base_url_override
            .as_deref()
            .unwrap_or(SPOT_PROD_BASE);
        super::spot_instruments::kucoin(&self.http, spot_base, now_ms()).await
    }

    async fn fetch_transfer_networks(&self) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        super::kucoin_transfer_networks::fetch(&self.http, self.spot_base_url()).await
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        let requested = [symbol.to_owned()];
        if let Some(mut rows) =
            super::kucoin_ws_mark_index::snapshot_funding(&self.config, Some(&requested))
        {
            if let Some(row) = rows.pop() {
                return Ok(row);
            }
        }

        // 修复 P1 7.3：直接调单合约 endpoint，避免拉全市场 500+ 合约后过滤丢弃 99%。
        // 文档：`GET /api/v1/contracts/{symbol}` 与 `contracts/active` 单 entry schema 一致，
        // 同时返回 `fundingFeeRate` + `predictedFundingFeeRate`（与 `funding-rate/{symbol}/current`
        // 端点不同，后者**缺失 predicted_rate**）。
        let exch = self.to_exchange_symbol(symbol);
        let item = public_rest::contract(&self.http, &self.base_url, &exch).await?;
        let parsed = parse_funding(&item).ok_or_else(|| {
            ExchangeError::Parse(format!("kucoin funding missing required fields for {exch}"))
        })?;
        Ok(parsed)
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        if let Some(rows) = super::kucoin_ws_mark_index::snapshot_funding(&self.config, symbols) {
            return Ok(rows);
        }

        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| normalized_to_kucoin(symbol))
                .collect::<HashSet<_>>()
        });
        let items = public_rest::contracts(&self.http, &self.base_url).await?;

        let rows: Vec<FundingRateData> = items
            .into_iter()
            .filter(|c| c.symbol.ends_with("USDTM"))
            .filter(|c| match requested.as_ref() {
                Some(symbols) => symbols.contains(&c.symbol),
                None => true,
            })
            .filter_map(|c| parse_funding(&c))
            .collect();
        Ok(rows)
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        // PR-DP-12 follow-up: prefer the WS snapshot + tickerV2 cache when
        // both halves are fresh; fall back to the existing REST pair otherwise.
        if let Some(row) = super::kucoin_ws_ticker::latest_ticker(&self.config, &self.http, symbol)
        {
            return Ok(row);
        }
        let exch = self.to_exchange_symbol(symbol);
        let items = public_rest::contracts(&self.http, &self.base_url).await?;
        let quote = public_rest::futures_ticker(&self.http, &self.base_url, &exch)
            .await
            .ok();
        let item = items
            .into_iter()
            .find(|c| c.symbol == exch)
            .ok_or(ExchangeError::UnsupportedSymbol(exch))?;
        parse_ticker(&item, quote.as_ref()).ok_or_else(|| {
            ExchangeError::Parse(format!("kucoin ticker missing required price for {symbol}"))
        })
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        // PR-DP-12 follow-up: WS fast-path for explicit watchlist symbols;
        // full-scan (None) keeps the REST aggregate because there is no
        // symbol universe cache yet.
        if let Some(rows) =
            super::kucoin_ws_ticker::snapshot_tickers(&self.config, &self.http, symbols)
        {
            return Ok(rows);
        }
        let items = public_rest::contracts(&self.http, &self.base_url).await?;
        let quotes = public_rest::futures_tickers(&self.http, &self.base_url)
            .await
            .unwrap_or_default();
        Ok(items
            .into_iter()
            .filter(|c| c.symbol.ends_with("USDTM"))
            .filter_map(|c| parse_ticker(&c, quotes.get(&c.symbol)))
            .collect())
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        Ok(
            super::kucoin_ws_ticker::snapshot_tickers(&self.config, &self.http, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        Ok(
            super::kucoin_ws_mark_index::snapshot_funding(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        Ok(
            super::kucoin_ws_mark_index::snapshot_mark_index(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        Ok(super::kucoin_ws_spot_ticker::snapshot_spot_ticks(
            &self.config,
            &self.http,
            Some(symbols),
        )
        .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    fn public_ws_spot_problem(&self, _symbol: &str) -> Option<String> {
        super::kucoin_ws_spot_ticker::spot_connection_problem(&self.config, &self.http)
            .map(|problem| format!("KuCoin Spot WS 连接失败：{problem}"))
    }

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        if let Some(rows) = super::kucoin_ws_mark_index::snapshot_mark_index(&self.config, symbols)
        {
            return Ok(rows);
        }
        self.rest_mark_index_prices(symbols).await
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let normalized = self.normalize_symbol(symbol);
        Ok(unverified_index_composition(
            NAME,
            &normalized,
            self.to_exchange_symbol(symbol),
            "kucoin index price available; public index components endpoint not verified",
            "KuCoin adapter only exposes index price/mark data in this product path; no verified public component list is wired.",
        ))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        if let Some(rows) =
            super::kucoin_ws_spot_ticker::snapshot_spot_ticks(&self.config, &self.http, symbols)
        {
            return Ok(rows);
        }
        let data = public_rest::spot_tickers(&self.http, self.spot_base_url()).await?;
        let exchange_ts_ms = (data.time > 0).then_some(data.time);
        Ok(data
            .ticker
            .into_iter()
            .filter(|item| spot_symbol_matches(&item.symbol, symbols))
            .filter_map(|item| parse_spot_tick(&item, exchange_ts_ms))
            .collect())
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let ws_book = self.ws_orderbook(symbol, depth);
        let contract_size = self.contract_order_unit(symbol).await?;
        if let Some(book) = ws_book {
            return normalize_contract_book(book, contract_size);
        }

        let exch = self.to_exchange_symbol(symbol);
        // 修复 P2 7.x：KuCoin Futures 公开 level2 endpoint 仅两档 `depth20` / `depth100`。
        // 文档：<https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-part-orderbook>
        // 完整 level2 stream（无限档）需 WebSocket 增量同步，超本适配器 V1 范畴；
        // 这里 snap：depth ≤ 20 → 20；20 < depth ≤ 100 → 100。
        let snapped_endpoint = snap_kucoin_depth_endpoint(depth);
        let body =
            public_rest::orderbook(&self.http, &self.base_url, snapped_endpoint, &exch).await?;
        normalize_contract_book(
            OrderBookInfo {
                symbol: kucoin_to_normalized(&exch),
                exchange: NAME.into(),
                bids: body.bids,
                asks: body.asks,
                timestamp: if body.ts == 0 {
                    now_ms()
                } else {
                    body.ts / 1_000_000
                }, // KuCoin 返回 ns
            },
            contract_size,
        )
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let Some(book) = self.ws_orderbook(symbol, depth) else {
            return Ok(PublicWsSnapshot::Pending);
        };
        let contract_size = self.contract_order_unit(symbol).await?;
        Ok(PublicWsSnapshot::Ready(vec![normalize_contract_book(
            book,
            contract_size,
        )?]))
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        if let Some(book) = super::kucoin_ws_spot_depth::latest_spot_orderbook(
            &self.config,
            &self.http,
            symbol,
            depth,
        ) {
            return Ok(book);
        }
        let pair = crate::spot::native_pair_symbol(symbol, '-')
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        // Official: <https://www.kucoin.com/docs-new/rest/spot-trading/market-data/get-part-order-book>
        let endpoint = if depth <= 20 {
            "level2_20"
        } else {
            "level2_100"
        };
        let body =
            public_rest::spot_orderbook(&self.http, self.spot_base_url(), endpoint, &pair).await?;
        Ok(OrderBookInfo {
            symbol: crate::spot::native_pair_symbol(&pair, '/').unwrap_or(pair),
            exchange: NAME.into(),
            bids: parse_spot_depth_levels(body.bids),
            asks: parse_spot_depth_levels(body.asks),
            timestamp: if body.time == 0 { now_ms() } else { body.time },
        })
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(
            match super::kucoin_ws_spot_depth::latest_spot_orderbook(
                &self.config,
                &self.http,
                symbol,
                depth,
            ) {
                Some(book) => PublicWsSnapshot::Ready(vec![book]),
                None => PublicWsSnapshot::Pending,
            },
        )
    }

    async fn get_balance(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        // 修复 P2 7.x：私有调用前校时（5min TTL），失败不阻塞。
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        // 注：KuCoin Futures 默认 settle currency 是 USDT（主型）；caller 可传 USDC/USDM 拉他币种资产。
        let want = currency.unwrap_or("USDT");
        let path = format!("/api/v1/account-overview?currency={want}");
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::balances(&self.signed_request(&path, &headers), want).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        let native_symbol = self.funding_payment_symbol(symbol).await?;
        let mut offset = None;
        let mut pagination = super::funding_payments::FundingPaymentPagination::new(NAME);
        let mut payments = Vec::new();
        let mut venue_event_ids = HashSet::new();
        loop {
            let path = super::funding_payments::kucoin_funding_history_path(
                Some(&native_symbol),
                start_time_ms,
                end_time_ms,
                offset.as_deref(),
            );
            let headers = self.build_signed_headers("GET", &path, "")?;
            let page =
                private_rest::funding_payments(&self.signed_request(&path, &headers)).await?;
            offset = pagination.accept(page, &mut payments, &mut venue_event_ids)?;
            if offset.is_none() {
                return Ok(payments);
            }
        }
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        let path = "/api/v1/positions";
        let headers = self.build_signed_headers("GET", path, "")?;
        let target = if let Some(s) = symbol {
            Some(self.contract_native_symbol(s).await?)
        } else {
            None
        };
        let rows = private_rest::positions(&self.signed_request(path, &headers), target.as_deref())
            .await?;
        self.normalize_position_rows(rows).await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "kucoin: server time sync failed; falling back to local clock");
        }
        let mut path = format!("{OPEN_ORDERS_PATH}?status=active");
        if let Some(s) = symbol {
            path.push_str("&symbol=");
            path.push_str(&self.contract_native_symbol(s).await?);
        }
        let headers = self.build_signed_headers("GET", &path, "")?;
        let rows = private_rest::open_orders(&self.signed_request(&path, &headers)).await?;
        self.normalize_order_rows(rows).await
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        kucoin_to_normalized(symbol)
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        self.contract_multipliers
            .cached_native_symbol(symbol)
            .unwrap_or_else(|| normalized_to_kucoin(symbol))
    }
}

fn kucoin_order_not_found(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::Api {
            exchange,
            code,
            message,
        } if exchange == NAME
            && code == "100001"
            && message.contains("error.getOrder.orderNotExist")
    )
}

fn kucoin_spot_order_not_found(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::Api { code, message, .. }
            if code == "400100"
                && (message.to_ascii_lowercase().contains("not exist")
                    || message.to_ascii_lowercase().contains("not found"))
    )
}

fn normalize_kucoin_order_contracts(
    mut order: OrderInfo,
    contract_unit: f64,
) -> ExchangeResult<OrderInfo> {
    order.quantity =
        kucoin_contracts_to_base(order.quantity, contract_unit, "order.quantity", false)?;
    order.filled_quantity = kucoin_contracts_to_base(
        order.filled_quantity,
        contract_unit,
        "order.filled_quantity",
        true,
    )?;
    Ok(order)
}

fn normalize_kucoin_position_contracts(
    mut position: PositionInfo,
    contract_unit: f64,
) -> ExchangeResult<PositionInfo> {
    position.quantity =
        kucoin_contracts_to_base(position.quantity, contract_unit, "position.quantity", false)?;
    position.available_position = position
        .available_position
        .map(|value| kucoin_contracts_to_base(value, contract_unit, "position.available", true))
        .transpose()?;
    position.frozen_position = position
        .frozen_position
        .map(|value| kucoin_contracts_to_base(value, contract_unit, "position.frozen", true))
        .transpose()?;
    Ok(position)
}

fn kucoin_contracts_to_base(
    contracts: f64,
    contract_unit: f64,
    field: &str,
    allow_zero: bool,
) -> ExchangeResult<f64> {
    if !contract_unit.is_finite() || contract_unit <= 0.0 {
        return Err(ExchangeError::Parse(format!(
            "kucoin {field} has invalid contract multiplier {contract_unit}"
        )));
    }
    if !contracts.is_finite() || contracts < 0.0 || (!allow_zero && contracts == 0.0) {
        return Err(ExchangeError::Parse(format!(
            "kucoin {field} has invalid contract count {contracts}"
        )));
    }
    let quantity = contracts * contract_unit;
    if quantity.is_finite() {
        Ok(quantity)
    } else {
        Err(ExchangeError::Parse(format!(
            "kucoin {field} overflows base quantity: contracts={contracts} multiplier={contract_unit}"
        )))
    }
}

#[cfg(test)]
#[path = "kucoin_tests.rs"]
mod tests;

#[path = "kucoin_market_reads.rs"]
mod market_reads;

#[path = "kucoin_support.rs"]
mod support;
