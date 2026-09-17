pub use super::bybit_config::{BybitAccountType, BybitConfig, BybitCredentials};
use super::bybit_market_data::{
    clamp_orderbook_limit, funding_interval_minutes_to_hours, is_usdm_perp, is_usdt_perp,
    linear_stream_symbol, parse_funding, parse_index_components, parse_mark_index, parse_spot_tick,
    parse_ticker, spot_symbol_matches,
};
use super::bybit_private_rest as private_rest;
use super::bybit_public_rest as public_rest;
use super::bybit_trade_data::{
    ack_from_row, bybit_order_link_id, cancel_order_body_json, place_order_body_json,
    pre_check_order_body_json, OrderAckRow,
};
use super::bybit_ws_trade;
use super::spot_order_contract;
use crate::adapter::{strip_common_suffixes, ExchangeAdapter, PublicWsSnapshot};
use crate::adapters::bybit_ws_market::MarketStream as WsMarketStream;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{ExchangeCapabilities, LiveTradingAdapter, VenueAccountRead};
use crate::services::RateLimiter;
use async_trait::async_trait;
use common::time::now_ms;
use shared_types::instrument_registry::VenueInstrument;
use shared_types::{
    BalanceInfo, CancelOrderRequest, ExecutionMode, FeeProduct, FundingPaymentData,
    FundingRateData, IndexCompositionSnapshot, LiveOrderState, MarginMode, MarkIndexInfo, OrderAck,
    OrderBookInfo, OrderInfo, OrderIntent, OrderSide, OrderSource, OrderSubmissionContext,
    OrderType, PositionInfo, SpotTick, TickerInfo, TimeInForce, VenueAccountModeInfo,
    VenueBalanceInfo,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, OnceLock};

const PROD_BASE: &str = "https://api.bybit.com";
const TESTNET_BASE: &str = "https://api-testnet.bybit.com";
const PROD_WS_TRADE: &str = "wss://stream.bybit.com/v5/trade";
const TESTNET_WS_TRADE: &str = "wss://stream-testnet.bybit.com/v5/trade";
const NAME: &str = "bybit";
const SAFE_ORDER_PRECHECK_SYMBOL: &str = "BTC";
const SAFE_ORDER_PRECHECK_QUANTITY: f64 = 0.001;
const SAFE_ORDER_PRECHECK_PRICE: f64 = 100_000.0;
pub(super) const PLACE_ORDER_PATH: &str = "/v5/order/create";
pub(super) const CANCEL_ORDER_PATH: &str = "/v5/order/cancel";

const TIME_SYNC_INTERVAL_MS: i64 = 5 * 60 * 1000; // 5min

#[derive(Debug)]
pub struct Bybit {
    config: BybitConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    time_offset_ms: AtomicI64,
    time_synced_at_ms: AtomicI64,
    market_stream: OnceLock<Arc<WsMarketStream>>,
}

#[async_trait]
impl LiveTradingAdapter for Bybit {
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
        self.ensure_write_adapter()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed; using cached offset");
        }
        match self.account_mode_info().await {
            Ok(info) => Ok(Some(info)),
            Err(ExchangeError::Parse(message))
                if message == "bybit position mode evidence empty" =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn get_exchange_symbol_account_mode(
        &self,
        _exchange: &str,
        symbol: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.ensure_write_adapter()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed; using cached offset");
        }
        Ok(Some(self.symbol_account_mode_info(symbol).await?))
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        // 修复 P2 3.11：下单前校时（5min TTL，多数情况是 noop），失败不阻塞主路径。
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed; using cached offset");
        }
        let symbol = self.to_exchange_symbol(&intent.symbol);
        let position_idx = self.position_idx_for_order(intent, &symbol).await?;
        if self.config.base_url_override.is_none() {
            return bybit_ws_trade::place_order(
                self.ws_trade_config()?,
                intent,
                symbol,
                position_idx,
            )
            .await;
        }

        let body = place_order_body_json(intent, symbol, position_idx)?;
        let venue_client_order_id = bybit_order_link_id(&intent.client_order_id)?;
        let headers = self.build_signed_headers(&body)?;
        let row = private_rest::post_signed::<OrderAckRow>(
            &self.http,
            &self.base_url,
            PLACE_ORDER_PATH,
            body,
            &headers,
            "place order",
        )
        .await?;
        Ok(ack_from_row(
            intent.id.clone(),
            intent.client_order_id.clone(),
            venue_client_order_id,
            row,
            LiveOrderState::Accepted,
            None,
        ))
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
            tracing::warn!(%error, "bybit time sync failed; using cached offset");
        }
        let compiled = spot_order_contract::compile(NAME, intent, context)?;
        if self.config.base_url_override.is_none() {
            return bybit_ws_trade::place_spot_order(
                self.ws_trade_config()?,
                intent,
                compiled.native_symbol,
            )
            .await;
        }
        Err(ExchangeError::NotImplemented(
            "bybit spot REST override write fixture",
        ))
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        let symbol = self.to_exchange_symbol(&request.symbol);
        if self.config.base_url_override.is_none() {
            return bybit_ws_trade::cancel_order(self.ws_trade_config()?, request, symbol).await;
        }

        let body = cancel_order_body_json(request, symbol)?;
        let venue_client_order_id = bybit_order_link_id(&request.client_order_id)?;
        let headers = self.build_signed_headers(&body)?;
        let row = private_rest::post_signed::<OrderAckRow>(
            &self.http,
            &self.base_url,
            CANCEL_ORDER_PATH,
            body,
            &headers,
            "cancel order",
        )
        .await?;
        Ok(ack_from_row(
            request.internal_order_id.clone(),
            request.client_order_id.clone(),
            venue_client_order_id,
            row,
            LiveOrderState::CancelRequested,
            Some("bybit cancel accepted; final state requires order query".to_owned()),
        ))
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
            return bybit_ws_trade::cancel_spot_order(self.ws_trade_config()?, request, symbol)
                .await;
        }
        Err(ExchangeError::NotImplemented(
            "bybit spot REST override cancel fixture",
        ))
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        let exch = self.to_exchange_symbol(symbol);
        let venue_client_order_id = bybit_order_link_id(client_order_id)?;
        // 修复 P2 3.9：原 `format!` 直接拼接 client_order_id 未做 URL encode。
        // Bybit `orderLinkId` 用户可控，可能含 `&` `=` `+` 空格等需 percent encoding 的字符。
        // `Serializer` 非 `Send`，限制到 block 内不跨越 await。
        let query = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer
                .append_pair("category", "linear")
                .append_pair("symbol", &exch)
                .append_pair("orderLinkId", &venue_client_order_id);
            serializer.finish()
        };
        let headers = self.build_get_headers(&query)?;
        private_rest::get_order(&self.http, &self.base_url, &query, &headers).await
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
        let venue_client_order_id = bybit_order_link_id(client_order_id)?;
        self.query_order("spot", &native, "orderLinkId", &venue_client_order_id)
            .await
    }

    async fn get_order_by_exchange_order_id_with_context(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let category = if context.product == FeeProduct::Spot {
            "spot"
        } else {
            "linear"
        };
        let native = if context.product == FeeProduct::Spot {
            spot_order_contract::query_symbol(NAME, symbol, context)?
        } else {
            self.to_exchange_symbol(symbol)
        };
        self.query_order(category, &native, "orderId", exchange_order_id)
            .await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        ExchangeAdapter::get_open_orders(self, symbol).await
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(self.get_account_read(currency).await?.balances)
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        let query = format!("accountType={}", self.config.account_type.as_str());
        let headers = self.build_get_headers(&query)?;
        private_rest::account_read(&self.http, &self.base_url, &query, &headers, currency).await
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
        !self.config.testnet && self.config.account_type == BybitAccountType::Unified
    }

    async fn withdrawal_source_balance(
        &self,
        request: &crate::WithdrawalSourceBalanceRequest,
    ) -> ExchangeResult<crate::WithdrawalSourceBalance> {
        self.ensure_withdrawal_wallet(request.wallet_type)?;
        self.require_credentials()?;
        self.sync_server_time().await?;
        super::bybit_withdrawals::source_balance(&self.http, &self.base_url, request, |query| {
            self.build_get_headers(query)
        }).await
    }

    async fn submit_withdrawal(
        &self,
        request: &crate::WithdrawalSubmitRequest,
    ) -> ExchangeResult<crate::WithdrawalSubmission> {
        self.ensure_withdrawal_wallet(request.wallet_type)?;
        self.ensure_write_adapter()?;
        self.require_credentials()?;
        self.sync_server_time().await?;
        let timestamp = now_ms().saturating_add(self.time_offset_ms.load(std::sync::atomic::Ordering::Relaxed));
        super::bybit_withdrawals::submit(&self.http, &self.base_url, request, timestamp, |body| {
            self.build_signed_headers(body)
        }).await
    }

    async fn withdrawal_status(
        &self,
        request: &crate::WithdrawalStatusRequest,
    ) -> ExchangeResult<Option<crate::WithdrawalStatusEvidence>> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability("bybit testnet withdrawals"));
        }
        self.require_credentials()?;
        self.sync_server_time().await?;
        super::bybit_withdrawals::status(&self.http, &self.base_url, request, |query| {
            self.build_get_headers(query)
        }).await
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
                "bybit testnet deposit_status",
            ));
        }
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed before deposit history read");
        }
        super::bybit_deposits::status(&self.http, &self.base_url, request, |query| {
            self.build_get_headers(query)
        })
        .await
    }
}

impl Bybit {
    fn ensure_withdrawal_wallet(&self, wallet: crate::WithdrawalWalletType) -> ExchangeResult<()> {
        if self.config.testnet || (wallet == crate::WithdrawalWalletType::Spot
            && self.config.account_type != BybitAccountType::Unified) {
            return Err(ExchangeError::UnsupportedCapability("bybit withdrawal requires mainnet UTA or explicit funding wallet"));
        }
        Ok(())
    }

    async fn query_order(
        &self,
        category: &str,
        symbol: &str,
        key: &str,
        value: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        let query = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer
                .append_pair("category", category)
                .append_pair("symbol", symbol)
                .append_pair(key, value);
            serializer.finish()
        };
        let headers = self.build_get_headers(&query)?;
        private_rest::get_order(&self.http, &self.base_url, &query, &headers).await
    }
}

#[async_trait]
impl ExchangeAdapter for Bybit {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let checked_at_ms = now_ms();
        let (rows, spot) = tokio::try_join!(
            public_rest::instruments_rest(&self.http, &self.base_url),
            super::spot_instruments::bybit(&self.http, &self.base_url, checked_at_ms),
        )?;
        Ok(
            crate::adapters::bybit_instruments::instruments_from_rows(rows, checked_at_ms)
                .into_iter()
                .chain(spot)
                .collect(),
        )
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        super::spot_instruments::bybit(&self.http, &self.base_url, now_ms()).await
    }

    async fn fetch_transfer_networks(&self) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "bybit testnet transfer_networks",
            ));
        }
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed before transfer metadata read");
        }
        super::bybit_transfer_networks::fetch(&self.http, &self.base_url, || {
            self.build_get_headers("")
        })
        .await
    }

    async fn fetch_transfer_destination(
        &self,
        request: &crate::TransferDestinationRequest,
    ) -> ExchangeResult<crate::TransferDestinationEvidence> {
        if self.config.testnet {
            return Err(ExchangeError::UnsupportedCapability(
                "bybit testnet transfer_destination",
            ));
        }
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed before deposit address read");
        }
        if request.direction == crate::TransferDirection::WithdrawToChain {
            return super::bybit_withdrawals::destination(&self.http, &self.base_url, request, |query| {
                self.build_get_headers(query)
            }).await;
        }
        super::bybit_deposits::destination(&self.http, &self.base_url, request, |query| {
            self.build_get_headers(query)
        })
        .await
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        if let Some(row) = super::bybit_ws_ticker::latest_funding(&self.config, symbol) {
            return Ok(row);
        }
        let exch = self.to_exchange_symbol(symbol);
        let (item, server_time) =
            public_rest::funding_rate(&self.http, &self.base_url, &exch).await?;
        parse_funding(&item, 8, 0.0, server_time).ok_or_else(|| {
            ExchangeError::Parse(format!(
                "bybit funding {exch} missing rate or next funding time"
            ))
        })
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        if let Some(rows) = super::bybit_ws_ticker::snapshot_funding(&self.config, symbols) {
            return Ok(rows);
        }
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let (tickers, server_time, infos) =
            public_rest::funding_rates_and_instruments(&self.http, &self.base_url).await?;

        let interval_map: HashMap<String, u32> = infos
            .into_iter()
            .filter(|i| matches!(i.settle_coin.to_ascii_uppercase().as_str(), "USDT" | "USDC"))
            .map(|i| {
                let hours = funding_interval_minutes_to_hours(i.funding_interval);
                (i.symbol, hours)
            })
            .collect();

        let result = tickers
            .into_iter()
            .filter(|t| is_usdm_perp(&t.symbol) && interval_map.contains_key(&t.symbol))
            .filter(|t| include_discovery_perp(&t.symbol, requested.as_ref()))
            .filter_map(|t| {
                let interval = *interval_map.get(&t.symbol).unwrap_or(&8);
                let v24 = t.turnover24h.parse::<f64>().unwrap_or(0.0);
                parse_funding(&t, interval, v24, server_time)
            })
            .collect();
        Ok(result)
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        if let Some(row) = super::bybit_ws_ticker::latest_ticker(&self.config, symbol) {
            return Ok(row);
        }
        let exch = self.to_exchange_symbol(symbol);
        let item = public_rest::ticker(&self.http, &self.base_url, &exch).await?;
        parse_ticker(&item).ok_or_else(|| {
            ExchangeError::Parse(format!("bybit ticker {exch} missing bid/ask/last price"))
        })
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        if let Some(rows) = super::bybit_ws_ticker::snapshot_tickers(&self.config, symbols) {
            return Ok(rows);
        }
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let items = public_rest::tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|item| include_discovery_perp(&item.symbol, requested.as_ref()))
            .filter_map(|t| parse_ticker(&t))
            .collect())
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        Ok(
            super::bybit_ws_ticker::snapshot_tickers(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        Ok(
            super::bybit_ws_ticker::snapshot_funding(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        Ok(
            super::bybit_ws_ticker::snapshot_mark_index(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        Ok(
            super::bybit_ws_spot_ticker::snapshot_spot_ticks(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    fn public_ws_spot_problem(&self, _symbol: &str) -> Option<String> {
        super::bybit_ws_spot_ticker::spot_connection_problem(&self.config)
            .map(|problem| format!("Bybit Spot WS 连接失败：{problem}"))
    }

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        if let Some(rows) = super::bybit_ws_ticker::snapshot_mark_index(&self.config, symbols) {
            return Ok(rows);
        }
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let items = public_rest::tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|item| include_discovery_perp(&item.symbol, requested.as_ref()))
            .filter_map(|item| parse_mark_index(&item, now_ms()))
            .collect())
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let index_id = self.to_exchange_symbol(symbol);
        let (body, evidence) =
            public_rest::index_components(&self.http, &self.base_url, &index_id).await?;
        Ok(crate::adapter::attach_payload_evidence(
            parse_index_components(body),
            evidence,
        ))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        if let Some(rows) = super::bybit_ws_spot_ticker::snapshot_spot_ticks(&self.config, symbols)
        {
            return Ok(rows);
        }
        let (items, server_time_ms) = public_rest::spot_tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|item| spot_symbol_matches(&item.symbol, symbols))
            .filter_map(|item| parse_spot_tick(&item, server_time_ms))
            .collect())
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        if let Some(book) = self.ws_orderbook(symbol, depth) {
            return Ok(book);
        }

        let exch = self.to_exchange_symbol(symbol);
        // Current V5 REST orderbook supports up to 1,000 levels for linear contracts.
        let limit = clamp_orderbook_limit("linear", depth).to_string();
        let body = public_rest::orderbook(
            &self.http,
            &self.base_url,
            "linear",
            &exch,
            &limit,
            "orderbook",
        )
        .await?;

        Ok(OrderBookInfo {
            symbol: strip_common_suffixes(&body.s),
            exchange: NAME.into(),
            bids: parse_levels(body.b),
            asks: parse_levels(body.a),
            timestamp: if body.ts == 0 { now_ms() } else { body.ts },
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
            super::bybit_ws_spot_ticker::latest_spot_orderbook(&self.config, symbol, depth)
        {
            return Ok(book);
        }
        let exch = crate::spot::compact_pair_symbol(symbol)
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        // Official: <https://bybit-exchange.github.io/docs/v5/market/orderbook>
        let limit = clamp_orderbook_limit("spot", depth).to_string();
        let body = public_rest::orderbook(
            &self.http,
            &self.base_url,
            "spot",
            &exch,
            &limit,
            "spot orderbook",
        )
        .await?;
        Ok(OrderBookInfo {
            symbol: crate::spot::native_pair_symbol(&body.s, '/').unwrap_or(body.s),
            exchange: NAME.into(),
            bids: parse_levels(body.b),
            asks: parse_levels(body.a),
            timestamp: if body.ts == 0 { now_ms() } else { body.ts },
        })
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(
            match super::bybit_ws_spot_ticker::latest_spot_orderbook(&self.config, symbol, depth) {
                Some(book) => PublicWsSnapshot::Ready(vec![book]),
                None => PublicWsSnapshot::Pending,
            },
        )
    }

    async fn get_balance(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        // 修复 P1 3.1：accountType 从硬编码 UNIFIED 改为运行时配置。
        // 用 `format!` 拼接 enum，避免任何 user-controllable 输入污染 query 字符串。
        let query = format!("accountType={}", self.config.account_type.as_str());
        let headers = self.build_get_headers(&query)?;
        private_rest::balances(&self.http, &self.base_url, &query, &headers, currency).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed before funding payment read");
        }
        let base_coin = symbol.map(strip_common_suffixes);
        let mut cursor = None;
        let mut pagination = super::funding_payments::FundingPaymentPagination::new(NAME);
        let mut payments = Vec::new();
        let mut venue_event_ids = HashSet::new();
        loop {
            let query = super::funding_payments::bybit_transaction_log_query(
                self.config.account_type.as_str(),
                base_coin.as_deref(),
                start_time_ms,
                end_time_ms,
                cursor.as_deref(),
            );
            let headers = self.build_get_headers(&query)?;
            let page = private_rest::funding_payments(&self.http, &self.base_url, &query, &headers)
                .await?;
            cursor = pagination.accept(page, &mut payments, &mut venue_event_ids)?;
            if cursor.is_none() {
                return Ok(payments);
            }
        }
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        if let Some(symbol) = symbol {
            let query = format!("category=linear&symbol={}", self.to_exchange_symbol(symbol));
            let headers = self.build_get_headers(&query)?;
            return private_rest::positions(&self.http, &self.base_url, &query, &headers).await;
        }
        let usdt_query = "category=linear&settleCoin=USDT";
        let usdc_query = "category=linear&settleCoin=USDC";
        let usdt_headers = self.build_get_headers(usdt_query)?;
        let usdc_headers = self.build_get_headers(usdc_query)?;
        let (mut rows, usdc) = tokio::try_join!(
            private_rest::positions(&self.http, &self.base_url, usdt_query, &usdt_headers),
            private_rest::positions(&self.http, &self.base_url, usdc_query, &usdc_headers),
        )?;
        rows.extend(usdc);
        Ok(rows)
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        if let Some(symbol) = symbol {
            let query = format!("category=linear&symbol={}", self.to_exchange_symbol(symbol));
            let headers = self.build_get_headers(&query)?;
            return private_rest::open_orders(&self.http, &self.base_url, &query, &headers).await;
        }
        let usdt_query = "category=linear&settleCoin=USDT";
        let usdc_query = "category=linear&settleCoin=USDC";
        let usdt_headers = self.build_get_headers(usdt_query)?;
        let usdc_headers = self.build_get_headers(usdc_query)?;
        let (mut rows, usdc) = tokio::try_join!(
            private_rest::open_orders(&self.http, &self.base_url, usdt_query, &usdt_headers),
            private_rest::open_orders(&self.http, &self.base_url, usdc_query, &usdc_headers),
        )?;
        rows.extend(usdc);
        Ok(rows)
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        strip_common_suffixes(symbol)
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        linear_stream_symbol(symbol)
    }
}

fn include_discovery_perp(symbol: &str, requested: Option<&HashSet<String>>) -> bool {
    is_usdm_perp(symbol)
        && requested.map_or_else(|| is_usdt_perp(symbol), |symbols| symbols.contains(symbol))
}

impl Bybit {
    pub async fn credential_account_mode_info(
        &self,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed; using cached offset");
        }
        let headers = self.build_get_headers("")?;
        let info = private_rest::account_info(&self.http, &self.base_url, &headers).await?;
        Ok(Some(info.into_mode_info()?))
    }

    pub async fn validate_api_order_permission_status(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed; using cached offset");
        }
        let headers = self.build_get_headers("")?;
        private_rest::api_key_info(&self.http, &self.base_url, &headers)
            .await?
            .validate_linear_order_permission()
    }

    pub async fn validate_safe_order_pre_check_permission(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bybit time sync failed; using cached offset");
        }
        let intent = Self::safe_order_pre_check_intent();
        let symbol = self.to_exchange_symbol(&intent.symbol);
        let body = pre_check_order_body_json(&intent, symbol, 0)?;
        let headers = self.build_signed_headers(&body)?;
        private_rest::pre_check_order(&self.http, &self.base_url, body, &headers).await
    }

    fn safe_order_pre_check_intent() -> OrderIntent {
        let now = now_ms();
        let client_order_id = format!("xline-precheck-{now}");
        OrderIntent {
            id: client_order_id.clone(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: NAME.to_owned(),
            symbol: SAFE_ORDER_PRECHECK_SYMBOL.to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: SAFE_ORDER_PRECHECK_QUANTITY,
            price: Some(SAFE_ORDER_PRECHECK_PRICE),
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
        self.account_mode_info_for_query(
            "category=linear&settleCoin=USDT",
            "linear_usdt".to_owned(),
        )
        .await
    }

    async fn symbol_account_mode_info(&self, symbol: &str) -> ExchangeResult<VenueAccountModeInfo> {
        let exchange_symbol = self.to_exchange_symbol(symbol);
        let query = format!("category=linear&symbol={exchange_symbol}");
        let account_scope = format!("linear_usdt:{exchange_symbol}");
        self.account_mode_info_for_query(&query, account_scope)
            .await
    }

    async fn account_mode_info_for_query(
        &self,
        query: &str,
        account_scope: String,
    ) -> ExchangeResult<VenueAccountModeInfo> {
        let headers = self.build_get_headers(query)?;
        let mode = private_rest::position_mode(&self.http, &self.base_url, query, &headers).await?;
        Ok(VenueAccountModeInfo {
            venue: NAME.to_owned(),
            mode: mode.as_str().to_owned(),
            source: "bybit.GET /v5/position/list positionIdx".to_owned(),
            checked_at_ms: now_ms(),
            freshness_ms: Some(0),
            account_scope: Some(account_scope),
        })
    }

    async fn position_idx_for_order(
        &self,
        intent: &OrderIntent,
        exchange_symbol: &str,
    ) -> ExchangeResult<u8> {
        let query = format!("category=linear&symbol={exchange_symbol}");
        let headers = self.build_get_headers(&query)?;
        private_rest::position_idx_for_order(
            &self.http,
            &self.base_url,
            &query,
            &headers,
            intent.side,
            intent.reduce_only,
        )
        .await
    }
}

fn parse_levels(levels: Vec<[String; 2]>) -> Vec<[f64; 2]> {
    levels
        .into_iter()
        .filter_map(|[p, q]| Some([p.parse().ok()?, q.parse().ok()?]))
        .collect()
}

#[cfg(test)]
#[path = "bybit_tests.rs"]
mod tests;

#[path = "bybit_support.rs"]
mod support;
