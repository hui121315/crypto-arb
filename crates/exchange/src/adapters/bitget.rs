//! Bitget USDT-margined 永续适配器。
//!
//! 市场数据与私有 REST / WS 已统一走 V3 / UTA endpoint。

use super::bitget_instruments::{native_identity, BitgetInstrumentCache};
use super::bitget_market_data::{bitget_depth_limit, spot_symbol_matches};
use super::bitget_order_compiler::{compile_order, compile_spot_order};
use super::bitget_uta_config::BitgetUtaCategory;
use super::bitget_uta_market_data::{
    finalize_orderbook_levels, instrument_funding_interval, parse_funding,
    parse_funding_from_ticker, parse_index_components, parse_mark_index, parse_spot_tick,
    parse_ticker,
};
use super::bitget_uta_private_rest as private_rest;
use super::bitget_uta_public_rest as uta_public_rest;
use super::bitget_uta_trade_data::{
    ack_from_order_query, cancel_order_body_json_for, checked_client_oid, place_order_body_json,
};
use super::bitget_uta_ws_trade as bitget_ws_trade;
use super::spot_order_contract;
use crate::adapter::{strip_common_suffixes, ExchangeAdapter, PublicWsSnapshot};
pub use crate::adapters::bitget_config::{BitgetConfig, BitgetCredentials, BitgetMarginMode};
use crate::adapters::bitget_uta_ws_market::MarketStream as WsMarketStream;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{ExchangeCapabilities, LiveTradingAdapter, VenueAccountRead};
use crate::services::RateLimiter;
use async_trait::async_trait;
use common::time::now_ms;
use shared_types::instrument_registry::VenueInstrument;
use shared_types::{
    BalanceInfo, CancelOrderRequest, FeeProduct, FundingPaymentData, FundingRateData,
    IndexCompositionSnapshot, MarginMode, MarkIndexInfo, OrderAck, OrderBookInfo, OrderInfo,
    OrderIntent, OrderSubmissionContext, PositionInfo, SpotTick, TickerInfo, VenueAccountModeInfo,
    VenueBalanceInfo,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, AtomicU8};
use std::sync::{Arc, OnceLock};

const NAME: &str = "bitget";
pub(super) const PLACE_ORDER_PATH: &str = "/api/v3/trade/place-order";
pub(super) const CANCEL_ORDER_PATH: &str = "/api/v3/trade/cancel-order";
pub(super) const GET_ORDER_PATH: &str = "/api/v3/trade/order-info";
pub(super) const OPEN_ORDERS_PATH: &str = "/api/v3/trade/unfilled-orders";
pub(super) const ACCOUNT_SETTINGS_PATH: &str = "/api/v3/account/settings";

const TIME_SYNC_INTERVAL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug)]
pub struct Bitget {
    config: BitgetConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    time_offset_ms: AtomicI64,
    time_synced_at_ms: AtomicI64,
    instrument_cache: BitgetInstrumentCache,
    position_mode: AtomicU8,
    position_mode_checked_at_ms: AtomicI64,
    market_stream: OnceLock<Arc<WsMarketStream>>,
}

// ============== 内部 deserialize 结构 ==============

#[async_trait]
impl LiveTradingAdapter for Bitget {
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
        match self.config.margin_mode {
            BitgetMarginMode::Crossed => vec![MarginMode::Cross],
            BitgetMarginMode::Isolated => vec![MarginMode::Isolated],
        }
    }

    async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        let mode = self.account_position_mode().await?;
        Ok(Some(VenueAccountModeInfo {
            venue: NAME.to_owned(),
            mode: mode.as_str().to_owned(),
            source: "bitget.GET /api/v3/account/settings".to_owned(),
            checked_at_ms: now_ms(),
            freshness_ms: Some(0),
            account_scope: Some("uta".to_owned()),
        }))
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        // 修复 P2 4.7：下单前校时（5min TTL，多数情况是 noop），失败不阻塞。
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed; using cached offset");
        }
        let spec = self.resolve_instrument_spec(&intent.symbol).await?;
        let position_mode = self.account_position_mode().await?;
        let compiled = compile_order(intent, &spec, position_mode)?;
        let result = if self.config.base_url_override.is_none() {
            bitget_ws_trade::place_order(
                self.ws_trade_config()?,
                intent,
                &compiled,
                self.config.margin_mode,
            )
            .await
        } else {
            // 修复 P1 4.2：margin_mode 从配置读取，支持 isolated 用户。
            let body = place_order_body_json(intent, &compiled, self.config.margin_mode)?;
            let path = PLACE_ORDER_PATH;
            let headers = self.build_signed_headers("POST", path, &body)?;
            private_rest::place_order(
                &self.signed_request(path, &headers),
                body,
                intent.id.clone(),
                intent.client_order_id.clone(),
            )
            .await
        };
        self.recover_ambiguous_place_result(intent, result).await
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
            tracing::warn!(%error, "bitget time sync failed; using cached offset");
        }
        let compiled = compile_spot_order(intent, context)?;
        let result = if self.config.base_url_override.is_none() {
            bitget_ws_trade::place_order(
                self.ws_trade_config()?,
                intent,
                &compiled,
                self.config.margin_mode,
            )
            .await
        } else {
            let body = place_order_body_json(intent, &compiled, self.config.margin_mode)?;
            let headers = self.build_signed_headers("POST", PLACE_ORDER_PATH, &body)?;
            private_rest::place_order(
                &self.signed_request(PLACE_ORDER_PATH, &headers),
                body,
                intent.id.clone(),
                intent.client_order_id.clone(),
            )
            .await
        };
        self.recover_ambiguous_place_result_for(
            intent,
            result,
            BitgetUtaCategory::Spot,
            &compiled.native_symbol,
        )
        .await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        let identity = native_identity(&request.symbol)?;
        let category = identity.category;
        let symbol = identity.native_symbol;
        if self.config.base_url_override.is_none() {
            return bitget_ws_trade::cancel_order(
                self.ws_trade_config()?,
                request,
                category,
                symbol,
            )
            .await;
        }

        let body = cancel_order_body_json_for(request, category, symbol)?;
        let path = CANCEL_ORDER_PATH;
        let headers = self.build_signed_headers("POST", path, &body)?;
        private_rest::cancel_order(&self.signed_request(path, &headers), body, request).await
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
        if self.config.base_url_override.is_none() {
            return bitget_ws_trade::cancel_order(
                self.ws_trade_config()?,
                request,
                BitgetUtaCategory::Spot,
                symbol,
            )
            .await;
        }
        let body = cancel_order_body_json_for(request, BitgetUtaCategory::Spot, symbol)?;
        let headers = self.build_signed_headers("POST", CANCEL_ORDER_PATH, &body)?;
        private_rest::cancel_order(
            &self.signed_request(CANCEL_ORDER_PATH, &headers),
            body,
            request,
        )
        .await
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        let client_oid = checked_client_oid(client_order_id)?;
        self.query_order(symbol, "clientOid", &client_oid).await
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
        let native = spot_order_contract::query_symbol(NAME, symbol, context)?;
        let client_oid = checked_client_oid(client_order_id)?;
        self.query_order_for(BitgetUtaCategory::Spot, &native, "clientOid", &client_oid)
            .await
    }

    async fn get_order_by_exchange_order_id(
        &self,
        symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        let order_id = required_order_query_id("orderId", exchange_order_id)?;
        self.query_order(symbol, "orderId", order_id).await
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
        let native = spot_order_contract::query_symbol(NAME, symbol, context)?;
        let order_id = required_order_query_id("orderId", exchange_order_id)?;
        self.query_order_for(BitgetUtaCategory::Spot, &native, "orderId", order_id)
            .await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        ExchangeAdapter::get_open_orders(self, symbol).await
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(self.get_account_read(currency).await?.balances)
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        let path = "/api/v3/account/assets";
        let headers = self.build_signed_headers("GET", path, "")?;
        private_rest::account_read(&self.signed_request(path, &headers), currency).await
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

    fn withdrawal_submission_supported(&self) -> bool {
        true
    }

    async fn withdrawal_source_balance(
        &self,
        request: &crate::WithdrawalSourceBalanceRequest,
    ) -> ExchangeResult<crate::WithdrawalSourceBalance> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed before withdrawal balance read");
        }
        super::bitget_withdrawals::source_balance(
            &self.http,
            &self.base_url,
            request,
            |path, body| self.build_signed_headers("GET", path, body),
        )
        .await
    }

    async fn submit_withdrawal(
        &self,
        request: &crate::WithdrawalSubmitRequest,
    ) -> ExchangeResult<crate::WithdrawalSubmission> {
        self.ensure_write_adapter()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed before withdrawal submit");
        }
        super::bitget_withdrawals::submit(&self.http, &self.base_url, request, |path, body| {
            self.build_signed_headers("POST", path, body)
        })
        .await
    }

    async fn withdrawal_status(
        &self,
        request: &crate::WithdrawalStatusRequest,
    ) -> ExchangeResult<Option<crate::WithdrawalStatusEvidence>> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed before withdrawal history read");
        }
        super::bitget_withdrawals::status(&self.http, &self.base_url, request, |path, body| {
            self.build_signed_headers("GET", path, body)
        })
        .await
    }

    fn deposit_status_supported(&self) -> bool {
        true
    }

    async fn deposit_status(
        &self,
        request: &crate::DepositStatusRequest,
    ) -> ExchangeResult<Option<crate::DepositStatusEvidence>> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed before deposit history read");
        }
        super::bitget_deposits::status(&self.http, &self.base_url, request, |path| {
            self.build_signed_headers("GET", path, "")
        })
        .await
    }
}

impl Bitget {
    async fn query_order(
        &self,
        symbol: &str,
        query_field: &'static str,
        query_value: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let identity = native_identity(symbol)?;
        self.query_order_for(
            identity.category,
            &identity.native_symbol,
            query_field,
            query_value,
        )
        .await
    }

    async fn query_order_for(
        &self,
        category: BitgetUtaCategory,
        symbol: &str,
        query_field: &'static str,
        query_value: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let path = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer
                .append_pair("category", category.as_query())
                .append_pair("symbol", symbol)
                .append_pair(query_field, query_value);
            format!("{GET_ORDER_PATH}?{}", serializer.finish())
        };
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::get_order(&self.signed_request(&path, &headers)).await
    }

    async fn recover_ambiguous_place_result(
        &self,
        intent: &OrderIntent,
        result: ExchangeResult<OrderAck>,
    ) -> ExchangeResult<OrderAck> {
        let identity = native_identity(&intent.symbol)?;
        self.recover_ambiguous_place_result_for(
            intent,
            result,
            identity.category,
            &identity.native_symbol,
        )
        .await
    }

    async fn recover_ambiguous_place_result_for(
        &self,
        intent: &OrderIntent,
        result: ExchangeResult<OrderAck>,
        category: BitgetUtaCategory,
        symbol: &str,
    ) -> ExchangeResult<OrderAck> {
        match result {
            Ok(ack) => Ok(ack),
            Err(error) if private_rest::is_unknown_place_result(&error) => {
                let client_oid = checked_client_oid(&intent.client_order_id)?;
                match self
                    .query_order_for(category, symbol, "clientOid", &client_oid)
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
                            place_error = %error,
                            %query_error,
                            client_order_id = %intent.client_order_id,
                            "bitget unknown place result could not be confirmed by order query"
                        );
                        Err(ExchangeError::Network(format!(
                            "bitget place result is ambiguous ({error}); clientOid confirmation failed: {query_error}"
                        )))
                    }
                }
            }
            Err(error) => Err(error),
        }
    }
}

fn required_order_query_id<'a>(field: &str, value: &'a str) -> ExchangeResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(ExchangeError::Parse(format!(
            "bitget {field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

// ============== Trait 实现 ==============

#[async_trait]
impl ExchangeAdapter for Bitget {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        self.refresh_instrument_specs().await
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        self.refresh_spot_instrument_specs().await
    }

    async fn fetch_transfer_networks(&self) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        super::bitget_transfer_networks::fetch(&self.http, &self.base_url).await
    }

    async fn fetch_transfer_destination(
        &self,
        request: &crate::TransferDestinationRequest,
    ) -> ExchangeResult<crate::TransferDestinationEvidence> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed before deposit address read");
        }
        super::bitget_deposits::destination(&self.http, &self.base_url, request, |path| {
            self.build_signed_headers("GET", path, "")
        })
        .await
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        let requested = [symbol.to_owned()];
        if let Some(mut rows) =
            super::bitget_uta_ws_ticker::snapshot_funding(&self.config, Some(&requested))
        {
            if let Some(row) = rows.pop() {
                return Ok(row);
            }
        }

        let exch = self.to_exchange_symbol(symbol);
        let (rates, tickers) = tokio::try_join!(
            uta_public_rest::current_funding_rate(&self.http, &self.base_url, &exch),
            uta_public_rest::ticker(
                &self.http,
                &self.base_url,
                BitgetUtaCategory::UsdtFutures,
                &exch,
            )
        )?;
        let rate = rates
            .first()
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(exch.clone()))?;
        let volume = tickers
            .first()
            .and_then(parse_ticker)
            .map_or(0.0, |row| row.volume_24h);
        let parsed = parse_funding(rate, volume).ok_or_else(|| {
            ExchangeError::Parse(format!("bitget funding missing required fields for {exch}"))
        })?;
        super::bitget_uta_ws_ticker::seed_funding_intervals(
            &self.config,
            &HashMap::from([(exch, parsed.funding_interval)]),
        );
        Ok(parsed)
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        if let Some(rows) = super::bitget_uta_ws_ticker::snapshot_funding(&self.config, symbols) {
            return Ok(rows);
        }

        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let (tickers, instruments) = tokio::try_join!(
            uta_public_rest::tickers(&self.http, &self.base_url, BitgetUtaCategory::UsdtFutures),
            uta_public_rest::instruments(
                &self.http,
                &self.base_url,
                BitgetUtaCategory::UsdtFutures,
            )
        )?;
        let intervals: HashMap<String, u32> = instruments
            .into_iter()
            .filter_map(|item| {
                let (symbol, interval) = instrument_funding_interval(&item);
                interval.map(|interval| (symbol, interval))
            })
            .collect();
        super::bitget_uta_ws_ticker::seed_funding_intervals(&self.config, &intervals);
        let result = tickers
            .iter()
            .filter(|item| match requested.as_ref() {
                Some(symbols) => symbols.contains(&item.symbol),
                None => true,
            })
            .filter_map(|item| {
                let interval = intervals.get(&item.symbol).copied()?;
                parse_funding_from_ticker(item, interval)
            })
            .collect();
        Ok(result)
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        if let Some(row) = super::bitget_uta_ws_ticker::latest_ticker(&self.config, symbol) {
            return Ok(row);
        }
        let exch = self.to_exchange_symbol(symbol);
        let mut items = uta_public_rest::ticker(
            &self.http,
            &self.base_url,
            BitgetUtaCategory::UsdtFutures,
            &exch,
        )
        .await?;
        let item = items
            .pop()
            .ok_or_else(|| ExchangeError::Parse("bitget ticker empty".into()))?;
        parse_ticker(&item).ok_or_else(|| {
            ExchangeError::Parse(format!("bitget ticker missing required price for {symbol}"))
        })
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        if let Some(rows) = super::bitget_uta_ws_ticker::snapshot_tickers(&self.config, symbols) {
            return Ok(rows);
        }
        let items =
            uta_public_rest::tickers(&self.http, &self.base_url, BitgetUtaCategory::UsdtFutures)
                .await?;
        Ok(items.into_iter().filter_map(|t| parse_ticker(&t)).collect())
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        Ok(
            super::bitget_uta_ws_ticker::snapshot_tickers(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        Ok(
            super::bitget_uta_ws_ticker::snapshot_funding(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        Ok(
            super::bitget_uta_ws_ticker::snapshot_mark_index(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        Ok(
            super::bitget_uta_ws_spot_ticker::snapshot_spot_ticks(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    fn public_ws_spot_problem(&self, _symbol: &str) -> Option<String> {
        super::bitget_uta_ws_spot_ticker::spot_connection_problem(&self.config)
            .map(|problem| format!("Bitget Spot WS 连接失败：{problem}"))
    }

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        if let Some(rows) = super::bitget_uta_ws_ticker::snapshot_mark_index(&self.config, symbols)
        {
            return Ok(rows);
        }
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let items =
            uta_public_rest::tickers(&self.http, &self.base_url, BitgetUtaCategory::UsdtFutures)
                .await?;
        Ok(items
            .into_iter()
            .filter(|item| match requested.as_ref() {
                Some(symbols) => symbols.contains(&item.symbol),
                None => true,
            })
            .filter_map(|item| parse_mark_index(&item))
            .collect())
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let exch = self.to_exchange_symbol(symbol);
        let (body, evidence) =
            uta_public_rest::index_components(&self.http, &self.base_url, &exch).await?;
        Ok(crate::adapter::attach_payload_evidence(
            parse_index_components(body),
            evidence,
        ))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        if let Some(rows) =
            super::bitget_uta_ws_spot_ticker::snapshot_spot_ticks(&self.config, symbols)
        {
            return Ok(rows);
        }
        let items =
            uta_public_rest::tickers(&self.http, &self.base_url, BitgetUtaCategory::Spot).await?;
        Ok(items
            .into_iter()
            .filter(|item| spot_symbol_matches(&item.symbol, symbols))
            .filter_map(|item| parse_spot_tick(&item))
            .collect())
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        if let Some(book) = self.ws_orderbook(symbol, depth) {
            return Ok(book);
        }

        let exch = self.to_exchange_symbol(symbol);
        // Official UTA accepts an arbitrary `limit` from 1 through 1000.
        // <https://www.bitget.com/api-doc/uta/public/OrderBook>
        let limit = bitget_depth_limit(depth).to_string();
        let body = uta_public_rest::orderbook(
            &self.http,
            &self.base_url,
            BitgetUtaCategory::UsdtFutures,
            &exch,
            &limit,
        )
        .await?;
        let (bids, asks, timestamp) = finalize_orderbook_levels(&body);
        Ok(OrderBookInfo {
            symbol: strip_common_suffixes(&exch),
            exchange: NAME.into(),
            bids,
            asks,
            timestamp,
        })
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(self
            .ws_orderbook(symbol, depth)
            .map_or(PublicWsSnapshot::Pending, |book| {
                PublicWsSnapshot::Ready(vec![book])
            }))
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        if let Some(book) =
            super::bitget_uta_ws_spot_ticker::latest_spot_orderbook(&self.config, symbol, depth)
        {
            return Ok(book);
        }
        let exch = crate::spot::compact_pair_symbol(symbol)
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        // Official: <https://www.bitget.com/api-doc/uta/public/OrderBook> (`category=SPOT`).
        let limit = bitget_depth_limit(depth).to_string();
        let body = uta_public_rest::orderbook(
            &self.http,
            &self.base_url,
            BitgetUtaCategory::Spot,
            &exch,
            &limit,
        )
        .await?;
        let (bids, asks, timestamp) = finalize_orderbook_levels(&body);
        Ok(OrderBookInfo {
            symbol: crate::spot::native_pair_symbol(&exch, '/').unwrap_or(exch),
            exchange: NAME.into(),
            bids,
            asks,
            timestamp,
        })
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(
            match super::bitget_uta_ws_spot_ticker::latest_spot_orderbook(
                &self.config,
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
        // UTA wallet is global (no productType); coin filter happens in the parser.
        let path = "/api/v3/account/assets".to_owned();
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::balances(&self.signed_request(&path, &headers), currency).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed; using cached offset");
        }
        let target = symbol.map(|value| self.normalize_symbol(value));
        let mut payments = Vec::new();
        let mut venue_event_ids = HashSet::new();
        for funding_type in super::funding_payments::bitget_uta_funding_types() {
            let mut cursor = None;
            let mut pagination = super::funding_payments::FundingPaymentPagination::new(NAME);
            loop {
                let path = super::funding_payments::bitget_uta_financial_records_path(
                    funding_type,
                    start_time_ms,
                    end_time_ms,
                    cursor.as_deref(),
                );
                let headers = self.build_signed_headers("GET", &path, "")?;
                let page =
                    private_rest::funding_payments(&self.signed_request(&path, &headers)).await?;
                cursor = pagination.accept(page, &mut payments, &mut venue_event_ids)?;
                if cursor.is_none() {
                    break;
                }
            }
        }
        if let Some(target) = target {
            payments.retain(|row| row.symbol.eq_ignore_ascii_case(&target));
        }
        Ok(payments)
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        if let Some(symbol) = symbol {
            let identity = native_identity(symbol)?;
            return self
                .positions_for_category(identity.category, Some(&identity.native_symbol))
                .await;
        }
        let (usdt, usdc, coin) = tokio::try_join!(
            self.positions_for_category(BitgetUtaCategory::UsdtFutures, None),
            self.positions_for_category(BitgetUtaCategory::UsdcFutures, None),
            self.positions_for_category(BitgetUtaCategory::CoinFutures, None),
        )?;
        Ok(usdt.into_iter().chain(usdc).chain(coin).collect())
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        if let Some(symbol) = symbol {
            let identity = native_identity(symbol)?;
            return self
                .open_orders_for_category(identity.category, Some(&identity.native_symbol))
                .await;
        }
        let (usdt, usdc, coin) = tokio::try_join!(
            self.open_orders_for_category(BitgetUtaCategory::UsdtFutures, None),
            self.open_orders_for_category(BitgetUtaCategory::UsdcFutures, None),
            self.open_orders_for_category(BitgetUtaCategory::CoinFutures, None),
        )?;
        Ok(usdt.into_iter().chain(usdc).chain(coin).collect())
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        strip_common_suffixes(symbol)
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        let norm = strip_common_suffixes(symbol);
        format!("{norm}USDT")
    }
}

#[cfg(test)]
#[path = "bitget_tests.rs"]
mod tests;

#[path = "bitget_support.rs"]
mod support;
