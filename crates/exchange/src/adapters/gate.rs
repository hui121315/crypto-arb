//! Gate.io v4 USDT-margined 永续适配器。
//!
//! 公共行情、私有读与实盘写单接口。

use super::gate_market_data::{
    parse_funding, parse_funding_from_ticker_schedule, parse_index_constituents, parse_levels,
    parse_mark_index, parse_spot_tick, parse_ticker, snap_gate_depth, spot_symbol_matches,
};
use super::gate_private_data::{
    open_order_contract, open_order_text_matches, parse_open_order_with_contract_unit,
    parse_positions,
};
use super::gate_private_rest as private_rest;
use super::gate_public_rest as public_rest;
use super::gate_spot_ws_trade;
use super::gate_trade_data::gate_text;
use super::gate_ws_trade;
use super::spot_order_contract;
use crate::adapter::{
    strip_common_suffixes, ExchangeAdapter, MetadataRefreshOutcome, PublicWsSnapshot,
};
use crate::adapters::contract_orderbook::normalize_contract_book;
use crate::adapters::gate_contracts::{native_symbol_hint, GateContractCache};
use crate::adapters::gate_fee_evidence::GateFuturesFeeCache;
use crate::adapters::gate_ws_market::MarketStream as WsMarketStream;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{venue_balance_rows, ExchangeCapabilities, LiveTradingAdapter, VenueAccountRead};
use crate::services::RateLimiter;
use async_trait::async_trait;
use common::time::now_ms;
use shared_types::instrument_registry::VenueInstrument;
use shared_types::{
    BalanceInfo, CancelOrderRequest, FeeProduct, FundingPaymentData, FundingRateData,
    IndexCompositionSnapshot, MarkIndexInfo, OrderAck, OrderBookInfo, OrderInfo, OrderIntent,
    OrderSubmissionContext, PositionInfo, SpotTick, TickerInfo, VenueAccountModeInfo,
    VenueBalanceInfo,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, OnceLock};

pub use crate::adapters::gate_config::{GateConfig, GateCredentials};

const NAME: &str = "gate";
pub(super) const FUTURES_ORDERS_PATH: &str = "/api/v4/futures/usdt/orders";

const TIME_SYNC_INTERVAL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug)]
pub struct Gate {
    config: GateConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    contract_cache: GateContractCache,
    fee_cache: GateFuturesFeeCache,
    time_offset_secs: AtomicI64,
    time_synced_at_ms: AtomicI64,
    market_stream: OnceLock<Arc<WsMarketStream>>,
}

#[async_trait]
impl LiveTradingAdapter for Gate {
    fn name(&self) -> &'static str {
        NAME
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            // 修复 P2 5.4：Gate.io 提供 testnet（开关：`GateConfig::testnet`）。
            supports_testnet: true,
            supports_live: self.config.allow_live_writes,
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
        let path = "/api/v4/futures/usdt/accounts";
        let headers = self.build_signed_headers("GET", path, "", "")?;
        let mode =
            private_rest::account_position_mode(&self.signed_request(path, "", &headers)).await?;
        Ok(Some(VenueAccountModeInfo {
            venue: NAME.to_owned(),
            mode,
            source: "gate.GET /api/v4/futures/usdt/accounts".to_owned(),
            checked_at_ms: now_ms(),
            freshness_ms: Some(0),
            account_scope: Some("usdt_futures".to_owned()),
        }))
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        let params = self.place_order_params(intent).await?;
        // 修复 P2 5.9：下单前校时（5min TTL，多数情况是 noop），失败不阻塞下单。
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "gate: server time sync failed; falling back to local clock");
        }
        gate_ws_trade::place_order(self.ws_trade_config()?, intent, params).await
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
            tracing::warn!(%error, "gate server time sync failed; using local clock");
        }
        let compiled = spot_order_contract::compile(NAME, intent, context)?;
        gate_spot_ws_trade::place_order(self.spot_ws_trade_config()?, intent, &compiled).await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        self.executable_native_symbol(&request.symbol).await?;
        if let Err(e) = self.sync_server_time().await {
            tracing::warn!(error = %e, "gate: server time sync failed; falling back to local clock");
        }
        let params = self.cancel_order_params(request)?;
        gate_ws_trade::cancel_order(self.ws_trade_config()?, request, params).await
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
        let native = spot_order_contract::cancel_symbol(NAME, request, context)?;
        gate_spot_ws_trade::cancel_order(self.spot_ws_trade_config()?, request, &native).await
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        let native_symbol = self.executable_native_symbol(symbol).await?;
        let contract_unit = self.contract_market_unit(&native_symbol).await?;
        let text = gate_text(client_order_id)?;
        if let Some(order) = self.fetch_order_row_ws_first(&text).await? {
            return parse_gate_order_row(&order, &native_symbol, contract_unit).map(Some);
        }
        match self
            .fetch_open_order_rows_ws_first(Some(&native_symbol))
            .await?
            .into_iter()
            .find(|order| open_order_text_matches(order, &text))
        {
            Some(order) => parse_gate_order_row(&order, &native_symbol, contract_unit).map(Some),
            None => Ok(None),
        }
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
        let venue_client_order_id = gate_text(client_order_id)?;
        gate_spot_ws_trade::get_order(
            self.spot_ws_trade_config()?,
            &venue_client_order_id,
            &native,
        )
        .await
    }

    async fn get_order_by_exchange_order_id(
        &self,
        symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        let native_symbol = self.executable_native_symbol(symbol).await?;
        let contract_unit = self.contract_market_unit(&native_symbol).await?;
        let Some(row) = self
            .fetch_order_row_by_exchange_id(exchange_order_id)
            .await?
        else {
            return Ok(None);
        };
        let mut order = parse_gate_order_row(&row, &native_symbol, contract_unit)?;
        self.enrich_order_with_fill_evidence(&mut order).await?;
        Ok(Some(order))
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
        gate_spot_ws_trade::get_order(self.spot_ws_trade_config()?, exchange_order_id, &native)
            .await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        ExchangeAdapter::get_open_orders(self, symbol).await
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        let balances = ExchangeAdapter::get_balance(self, currency).await?;
        Ok(venue_balance_rows(NAME, balances))
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        let path = "/api/v4/futures/usdt/accounts";
        let headers = self.build_signed_headers("GET", path, "", "")?;
        private_rest::account_read(&self.signed_request(path, "", &headers), currency, now_ms())
            .await
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

// ============== Trait 实现 ==============

#[async_trait]
impl ExchangeAdapter for Gate {
    fn name(&self) -> &'static str {
        NAME
    }

    /// PR-DP-04 follow-up: 冷启动 metadata prewarm。
    /// 调一次 Gate 官方 contracts endpoint 填 `GateContractCache`
    /// （native identity + 完整下单规格，24h TTL），让 hot path 上的
    /// `ws_funding` / `get_positions` 不再首次承担全量 REST 拉取，
    /// funding schedule 也不需要用 8h fallback 兜默认值。
    async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
        self.refresh_contract_cache().await?;
        Ok(MetadataRefreshOutcome::Refreshed)
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let checked_at_ms = now_ms();
        let (_, spot) = tokio::try_join!(
            self.refresh_contract_cache(),
            super::spot_instruments::gate(&self.http, &self.base_url, checked_at_ms),
        )?;
        Ok(self
            .contract_cache
            .venue_instruments(checked_at_ms)
            .into_iter()
            .chain(spot)
            .collect())
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        super::spot_instruments::gate(&self.http, &self.base_url, now_ms()).await
    }

    async fn fetch_transfer_networks(&self) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        super::gate_transfer_networks::fetch(&self.http, &self.base_url).await
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        let exch = self.to_exchange_symbol(symbol);
        if let Some(funding) = self.ws_funding(&exch).await {
            return Ok(funding);
        }
        let item = public_rest::contract(&self.http, &self.base_url, &exch).await?;
        parse_funding(&item, 0.0).ok_or_else(|| {
            ExchangeError::Parse(format!(
                "gate funding {exch} missing rate, interval, or next funding time"
            ))
        })
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        if let Some(rows) = self.ws_funding_snapshot(symbols).await {
            return Ok(rows);
        }
        self.refresh_contract_cache().await?;
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let observed_at_ms = now_ms();
        let result = public_rest::tickers(&self.http, &self.base_url)
            .await?
            .into_iter()
            .filter(|ticker| {
                requested
                    .as_ref()
                    .is_none_or(|symbols| symbols.contains(&ticker.contract))
            })
            .filter_map(|ticker| {
                let (interval_hours, next_funding_time) = self
                    .contract_cache
                    .funding_schedule(&ticker.contract, observed_at_ms)?;
                parse_funding_from_ticker_schedule(&ticker, interval_hours, next_funding_time)
            })
            .collect();
        Ok(result)
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        if let Some(row) = super::gate_ws_ticker::latest_ticker(&self.config, symbol) {
            return Ok(row);
        }
        let exch = self.to_exchange_symbol(symbol);
        let mut tickers = public_rest::ticker(&self.http, &self.base_url, &exch).await?;
        let item = tickers
            .pop()
            .ok_or_else(|| ExchangeError::Parse("gate ticker empty".into()))?;
        parse_ticker(&item).ok_or_else(|| {
            ExchangeError::Parse(format!("gate ticker {exch} missing bid/ask/last price"))
        })
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        if let Some(rows) = super::gate_ws_ticker::snapshot_tickers(&self.config, symbols) {
            return Ok(rows);
        }
        let items = public_rest::tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|item| item.contract.ends_with("_USDT"))
            .filter_map(|item| parse_ticker(&item))
            .collect())
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        Ok(
            super::gate_ws_ticker::snapshot_tickers(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        Ok(self
            .ws_funding_snapshot(Some(symbols))
            .await
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        Ok(
            super::gate_ws_ticker::snapshot_mark_index(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        Ok(
            super::gate_ws_spot_ticker::snapshot_spot_ticks(&self.config, Some(symbols))
                .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready),
        )
    }

    fn public_ws_spot_problem(&self, _symbol: &str) -> Option<String> {
        super::gate_ws_spot_ticker::spot_connection_problem(&self.config)
            .map(|problem| format!("Gate Spot WS 连接失败：{problem}"))
    }

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        if let Some(rows) = super::gate_ws_ticker::snapshot_mark_index(&self.config, symbols) {
            return Ok(rows);
        }
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let timestamp = now_ms();
        let items = public_rest::tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|item| match requested.as_ref() {
                Some(symbols) => symbols.contains(&item.contract),
                None => item.contract.ends_with("_USDT"),
            })
            .filter_map(|item| parse_mark_index(&item, timestamp))
            .collect())
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let index = self.to_exchange_symbol(symbol);
        let (body, evidence) =
            public_rest::index_constituents(&self.http, &self.base_url, &index).await?;
        Ok(crate::adapter::attach_payload_evidence(
            parse_index_constituents(body),
            evidence,
        ))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        if let Some(rows) = super::gate_ws_spot_ticker::snapshot_spot_ticks(&self.config, symbols) {
            return Ok(rows);
        }
        let items = public_rest::spot_tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|item| spot_symbol_matches(&item.currency_pair, symbols))
            .filter_map(|item| parse_spot_tick(&item))
            .collect())
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let ws_book = self.ws_orderbook(symbol, depth);
        let contract_size = self.contract_market_unit(symbol).await?;
        if let Some(book) = ws_book {
            return normalize_contract_book(book, contract_size);
        }

        let exch = self.to_exchange_symbol(symbol);
        // 修复 P2 5.5：Gate.io V4 `limit` 离散值集合（与文档一致：
        // <https://www.gate.com/docs/developers/apiv4/#futures-order-book>，valid: 0, 1, 5, 10, 20, 50, 100）。
        // 注：0 表示返回全部档位（最大开销），适配器禁用 0，避免误用。
        let limit = snap_gate_depth(depth).to_string();
        let body = public_rest::orderbook(&self.http, &self.base_url, &exch, &limit).await?;
        normalize_contract_book(
            OrderBookInfo {
                symbol: strip_common_suffixes(&exch),
                exchange: NAME.into(),
                bids: parse_levels(body.bids),
                asks: parse_levels(body.asks),
                timestamp: if body.update > 0.0 {
                    (body.update * 1000.0) as i64
                } else {
                    now_ms()
                },
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
        let contract_size = self.contract_market_unit(symbol).await?;
        Ok(PublicWsSnapshot::Ready(vec![normalize_contract_book(
            book,
            contract_size,
        )?]))
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        if let Some(book) =
            super::gate_ws_spot_ticker::latest_spot_orderbook(&self.config, symbol, depth)
        {
            return Ok(book);
        }
        let pair = crate::spot::native_pair_symbol(symbol, '_')
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        // Official: <https://www.gate.com/docs/developers/apiv4/#retrieve-order-book>
        let limit = depth.clamp(1, 100).to_string();
        let body = public_rest::spot_orderbook(&self.http, &self.base_url, &pair, &limit).await?;
        Ok(OrderBookInfo {
            symbol: crate::spot::native_pair_symbol(&pair, '/').unwrap_or(pair),
            exchange: NAME.into(),
            bids: parse_levels(body.bids),
            asks: parse_levels(body.asks),
            timestamp: if body.update > 0.0 {
                (body.update * 1000.0) as i64
            } else {
                now_ms()
            },
        })
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(
            match super::gate_ws_spot_ticker::latest_spot_orderbook(&self.config, symbol, depth) {
                Some(book) => PublicWsSnapshot::Ready(vec![book]),
                None => PublicWsSnapshot::Pending,
            },
        )
    }

    async fn get_balance(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        // 修复 P2 5.10：Gate.io USDT-margined futures 账户只有 USDT 余额。
        // 调用者请求 non-USDT 时直接 short-circuit，避免无意义 HTTP 调用 + 日志提示。
        if let Some(want) = currency {
            if !want.eq_ignore_ascii_case("USDT") {
                tracing::debug!(
                    requested = want,
                    "gate: USDT-margined futures has no non-USDT balance; returning empty map"
                );
                return Ok(HashMap::new());
            }
        }
        let path = "/api/v4/futures/usdt/accounts";
        let headers = self.build_signed_headers("GET", path, "", "")?;
        private_rest::balance(&self.signed_request(path, "", &headers), currency).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        let contract = match symbol {
            Some(value) => Some(self.executable_native_symbol(value).await?),
            None => None,
        };
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "gate: server time sync failed; falling back to local clock");
        }
        let path = super::funding_payments::GATE_ACCOUNT_BOOK_PATH;
        let query = super::funding_payments::gate_account_book_query(
            contract.as_deref(),
            start_time_ms,
            end_time_ms,
        );
        let headers = self.build_signed_headers("GET", path, &query, "")?;
        private_rest::funding_payments(&self.signed_request(path, &query, &headers), "USDT").await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        let target = match symbol {
            Some(value) => Some(self.executable_native_symbol(value).await?),
            None => None,
        };
        let path = "/api/v4/futures/usdt/positions";
        let headers = self.build_signed_headers("GET", path, "", "")?;
        let rows = private_rest::position_rows(&self.signed_request(path, "", &headers)).await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        // Gate positions are contract counts. A missing official contract multiplier
        // must never be projected as base-asset quantity.
        if let Err(e) = self.refresh_contract_cache().await {
            tracing::warn!(error = %e, "gate: refresh contract cache failed, using stale metadata only");
        }
        parse_positions(&rows, target.as_deref(), |contract| {
            self.contract_cache.get_unit(contract).ok_or_else(|| {
                ExchangeError::Api {
                    exchange: NAME.to_owned(),
                    code: "instrument_metadata_missing".to_owned(),
                    message: format!(
                        "gate position {contract} cannot be normalized without official quanto_multiplier"
                    ),
                }
            })
        })
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        let native_symbol = match symbol {
            Some(value) => Some(self.executable_native_symbol(value).await?),
            None => None,
        };
        let rows = self
            .fetch_open_order_rows_ws_first(native_symbol.as_deref())
            .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        if let Err(error) = self.refresh_contract_cache().await {
            tracing::warn!(%error, "gate open-order contract refresh failed; using stale metadata only");
        }
        rows.iter()
            .map(|row| {
                let contract = open_order_contract(row)?;
                let contract_unit = self.contract_cache.get_unit(contract).ok_or_else(|| {
                    ExchangeError::Api {
                        exchange: NAME.to_owned(),
                        code: "instrument_metadata_missing".to_owned(),
                        message: format!(
                            "gate order {contract} cannot be normalized without official quanto_multiplier"
                        ),
                    }
                })?;
                parse_open_order_with_contract_unit(row, contract_unit)
            })
            .collect()
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        strip_common_suffixes(symbol)
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        self.contract_cache
            .cached_native_symbol(symbol)
            .or_else(|| native_symbol_hint(symbol))
            .unwrap_or_else(|| format!("{}_USDT", strip_common_suffixes(symbol)))
    }
}

fn parse_gate_order_row(
    row: &super::gate_private_data::OpenOrderItem,
    expected_native_symbol: &str,
    contract_unit: f64,
) -> ExchangeResult<OrderInfo> {
    let order = parse_open_order_with_contract_unit(row, contract_unit)?;
    if order.symbol != strip_common_suffixes(expected_native_symbol) {
        return Err(ExchangeError::Parse(format!(
            "gate order {} contract mismatch: expected {expected_native_symbol}, returned {}",
            order.order_id, order.symbol
        )));
    }
    Ok(order)
}

#[path = "gate_ws_cache.rs"]
mod ws_cache;

#[path = "gate_trade_support.rs"]
mod trade_support;

#[path = "gate_support.rs"]
mod support;

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
