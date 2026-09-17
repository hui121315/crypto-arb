//! Hyperliquid 适配器（USDC-margined perpetual on Hyperliquid L1）。
//!
//! V1 范围：公共行情 + 私有读接口（基于 EVM 地址查询，无需签名）。
//! V2 写单：通过官方 WebSocket `post` request 封装 L1 exchange action。
//!
//! ## 协议特点
//! - **单一 REST 端点** `POST /info`，通过 body 中的 `type` 字段路由不同请求
//! - 公共接口完全零签名
//! - 私有读接口（balance/positions/orders）只需公开 EVM 地址作为 `user` 参数；**写接口（V2）才需要 EIP-712 签名**
//! - Symbol 仅基础币种，如 `"BTC"`、`"ETH"`（无后缀）
//! - HIP-3 builder-deployed perps 用 `dex:SYMBOL` 命名，行情读取需带 `dex` 参数。
//! - **funding rate 是 1h** 周期（不是 8h）；`rate_8h = rate * 8`

#[path = "hyperliquid_account_read.rs"]
mod account_read;

use super::hyperliquid_ws_trade;
use crate::adapter::{unverified_index_composition, ExchangeAdapter, PublicWsSnapshot};
use crate::adapters::hyperliquid_instruments::{
    instrument_cache_from_metadata, spot_spec_from_instrument, HyperliquidInstrumentCache,
    HyperliquidInstrumentSpec,
};
use crate::adapters::hyperliquid_market_data::{
    build_predicted_map, clean_hyperliquid_symbol, funding_rows, hyperliquid_symbol_matches,
    parse_levels, parse_mark_index_for_venue, parse_ticker_for_venue, ticker_rows, AssetCtx,
    L2Book, PredictedFundingsResponse, SpotMetaWrapper, UniverseWrapper,
};
use crate::adapters::hyperliquid_private_data::{
    open_orders_need_fill_evidence, order_status_needs_fill_evidence,
    order_status_to_info_with_fills, parse_open_orders_with_fills, parse_perp_balance,
    parse_positions, ClearinghouseState, OpenOrderItem, OrderStatusPayload, UserFillItem,
};
use crate::adapters::hyperliquid_trade_data::{
    hyperliquid_cancel_action, hyperliquid_order_action, hyperliquid_venue_cloid, required_cloid,
};
use crate::adapters::hyperliquid_ws_active_ctx::ActiveAssetCtxStream as WsActiveAssetCtxStream;
use crate::adapters::hyperliquid_ws_market::MarketStream as WsMarketStream;
use crate::adapters::spot_order_contract;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{ExchangeCapabilities, LiveTradingAdapter, VenueAccountRead};
use crate::services::RateLimiter;
use async_trait::async_trait;
use common::time::now_ms;
use dashmap::DashMap;
use moka::future::Cache;
use serde_json::json;
use shared_types::instrument_registry::VenueInstrument;
use shared_types::{
    BalanceInfo, CancelOrderRequest, FeeProduct, FundingRateData, IndexCompositionSnapshot,
    MarkIndexInfo, OrderAck, OrderBookInfo, OrderInfo, OrderIntent, OrderSubmissionContext,
    PositionInfo, SpotTick, TickerInfo, VenueBalanceInfo,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub use crate::adapters::hyperliquid_config::{
    HyperliquidConfig, HyperliquidCredentials, HyperliquidMarket,
};

const NAME: &str = "hyperliquid";

type MarketContext = Arc<(UniverseWrapper, Vec<AssetCtx>)>;
type MarketContextCache = Cache<HyperliquidMetadataCacheKey, MarketContext>;

static INSTRUMENT_CACHES: OnceLock<
    DashMap<HyperliquidMetadataCacheKey, Arc<HyperliquidInstrumentCache>>,
> = OnceLock::new();
static MARKET_CONTEXTS: OnceLock<MarketContextCache> = OnceLock::new();
static PREDICTED_FUNDINGS: OnceLock<Cache<String, Arc<PredictedFundingsResponse>>> =
    OnceLock::new();

const MARKET_CONTEXT_TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HyperliquidMetadataCacheKey {
    base_url: String,
    dex: Option<String>,
}

#[derive(Debug)]
pub struct Hyperliquid {
    config: HyperliquidConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    market_stream: OnceLock<Arc<WsMarketStream>>,
    /// PR-DP-12 follow-up: per-venue `activeAssetCtx` cache for funding /
    /// mark / index price. Lazily initialised on first watchlist touch so
    /// REST-only callers do not pay the WS connection cost.
    active_ctx_stream: OnceLock<Arc<WsActiveAssetCtxStream>>,
    all_mids_stream: OnceLock<Arc<crate::adapters::hyperliquid_ws_all_mids::AllMidsStream>>,
}

/// Public, non-secret facts needed to prove that a configured Hyperliquid
/// account, signer, and optional vault are allowed to act together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidCredentialRelation {
    pub main_account: String,
    pub main_account_role: String,
    pub main_account_owner: Option<String>,
    pub signer: String,
    pub signer_role: String,
    pub signer_owner: Option<String>,
    pub vault_address: Option<String>,
    pub vault_role: Option<String>,
    pub vault_leader: Option<String>,
}

/// Official account-abstraction facts returned for the configured read
/// address. `user_dex_abstraction` is nullable when no builder-dex-specific
/// abstraction is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidAccountAbstraction {
    pub account_address: String,
    pub user_abstraction: String,
    pub user_dex_abstraction: Option<String>,
}

// ============== 实盘写单 Trait 实现 ==============

#[async_trait]
impl LiveTradingAdapter for Hyperliquid {
    fn name(&self) -> &'static str {
        self.adapter_name()
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

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        let venue_client_order_id = hyperliquid_venue_cloid(&intent.client_order_id)?;
        let action = self.compile_order_action(intent)?;
        hyperliquid_ws_trade::place_order(
            self.ws_trade_config()?,
            intent,
            venue_client_order_id,
            action,
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
        spot_order_contract::compile(NAME, intent, context)?;
        let instrument = context.instrument_spec.as_ref().ok_or_else(|| {
            ExchangeError::Parse("hyperliquid spot instrument context missing".to_owned())
        })?;
        let spec = spot_spec_from_instrument(instrument)?;
        let action = hyperliquid_order_action(&spec, intent)?;
        let venue_client_order_id = hyperliquid_venue_cloid(&intent.client_order_id)?;
        hyperliquid_ws_trade::place_order(
            self.ws_trade_config()?,
            intent,
            venue_client_order_id,
            action,
        )
        .await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.ensure_write_adapter()?;
        let spec = self.cached_instrument_spec(&request.symbol)?;
        let venue_client_order_id = hyperliquid_venue_cloid(&request.client_order_id)?;
        let action = hyperliquid_cancel_action(spec.asset_id, request)?;
        hyperliquid_ws_trade::cancel_order(
            self.ws_trade_config()?,
            request,
            venue_client_order_id,
            action,
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
        spot_order_contract::cancel_symbol(NAME, request, context)?;
        let instrument = context.instrument_spec.as_ref().ok_or_else(|| {
            ExchangeError::Parse("hyperliquid spot instrument context missing".to_owned())
        })?;
        let spec = spot_spec_from_instrument(instrument)?;
        let venue_client_order_id = hyperliquid_venue_cloid(&request.client_order_id)?;
        let action = hyperliquid_cancel_action(spec.asset_id, request)?;
        hyperliquid_ws_trade::cancel_order(
            self.ws_trade_config()?,
            request,
            venue_client_order_id,
            action,
        )
        .await
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        self.ensure_write_adapter()?;
        let user = self.require_user()?;
        let cloid = required_cloid(client_order_id)?;
        let body = json!({"type": "orderStatus", "user": user, "oid": cloid});
        let payload: OrderStatusPayload = self.post_info(body).await?;
        let fills = if order_status_needs_fill_evidence(&payload)? {
            self.user_fills(&user).await?
        } else {
            Vec::new()
        };
        let target = self.api_coin(symbol);
        order_status_to_info_with_fills(payload, &fills, &target, self.adapter_name())
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
        let target = spot_order_contract::query_symbol(NAME, symbol, context)?;
        let user = self.require_user()?;
        let cloid = required_cloid(client_order_id)?;
        self.query_order_status(&user, json!(cloid), &target).await
    }

    async fn get_order_by_exchange_order_id_with_context(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let target = if context.product == FeeProduct::Spot {
            spot_order_contract::query_symbol(NAME, symbol, context)?
        } else {
            self.api_coin(symbol)
        };
        let order_id = exchange_order_id
            .trim()
            .parse::<u64>()
            .map_err(|_| ExchangeError::Parse("hyperliquid oid must be numeric".to_owned()))?;
        let user = self.require_user()?;
        self.query_order_status(&user, json!(order_id), &target)
            .await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        ExchangeAdapter::get_open_orders(self, symbol).await
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(self.get_account_read(currency).await?.balances)
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        self.get_partial_account_read(currency).await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        ExchangeAdapter::get_positions(self, symbol).await
    }
}

// ============== 读接口 Trait 实现 ==============

#[async_trait]
impl ExchangeAdapter for Hyperliquid {
    fn name(&self) -> &'static str {
        self.adapter_name()
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let checked_at_ms = now_ms();
        let metadata = self.instrument_metadata().await?;
        let cache = Arc::new(instrument_cache_from_metadata(
            metadata,
            self.config.market,
            checked_at_ms,
        )?);
        let mut instruments = cache.instruments();
        if self.config.market.dex().is_none() {
            instruments.extend(self.spot_instruments(checked_at_ms).await?);
        }
        instrument_caches().insert(self.metadata_cache_key(), cache);
        Ok(instruments)
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        let target = self.normalize_symbol(symbol);
        if let Some(funding) = self.ws_funding(&target) {
            return Ok(funding);
        }
        let rates = self.get_funding_rates(None).await?;
        rates
            .into_iter()
            .find(|r| r.symbol == target)
            .ok_or(ExchangeError::UnsupportedSymbol(target))
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        if let Some(rows) = self.ws_funding_snapshot(symbols) {
            return Ok(rows);
        }
        // 修复 P1 8.1 / 8.5：并行调 `metaAndAssetCtxs`（当期费率 + 量）+ `predictedFundings`
        // （下期预测费率 + nextFundingTime），按 coin 名匹配，避免下游策略选不到 HL。
        // 文档：<https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-predicted-fundings>
        let (pair_res, predicted_res): (
            ExchangeResult<Arc<(UniverseWrapper, Vec<AssetCtx>)>>,
            ExchangeResult<Arc<PredictedFundingsResponse>>,
        ) = tokio::join!(self.market_context(), self.predicted_fundings());
        let pair = pair_res?;
        let (meta, ctxs) = pair.as_ref();
        // predictedFundings 失败时不阻塞主路径（保持当期 rate 可用）
        let predicted_map = match predicted_res {
            Ok(data) => build_predicted_map(data.as_ref()),
            Err(e) => {
                tracing::warn!(error = %e, "hyperliquid: predictedFundings fetch failed; predicted_rate=None");
                HashMap::new()
            }
        };

        Ok(funding_rows(
            self.adapter_name(),
            meta,
            ctxs,
            &predicted_map,
        ))
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        let target = self.normalize_symbol(symbol);
        if let Some(ticker) = self.ws_ticker(&target) {
            return Ok(ticker);
        }
        let pair = self.market_context().await?;
        let (meta, ctxs) = pair.as_ref();
        let idx = meta
            .universe
            .iter()
            .position(|e| hyperliquid_symbol_matches(&e.name, &target))
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(target.clone()))?;
        let ctx = ctxs
            .get(idx)
            .ok_or_else(|| ExchangeError::Parse("hyperliquid asset ctx out of bound".into()))?;
        parse_ticker_for_venue(self.adapter_name(), &target, ctx).ok_or_else(|| {
            ExchangeError::Parse(format!(
                "hyperliquid ticker missing required price for {target}"
            ))
        })
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        if let Some(rows) = self.ws_ticker_snapshot(symbols) {
            return Ok(rows);
        }
        let pair = self.market_context().await?;
        let (meta, ctxs) = pair.as_ref();
        Ok(ticker_rows(self.adapter_name(), meta, ctxs))
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

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        if let Some(rows) = self.ws_mark_index_snapshot(symbols) {
            return Ok(rows);
        }
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.normalize_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let pair = self.market_context().await?;
        let (meta, ctxs) = pair.as_ref();
        let mut out = Vec::with_capacity(meta.universe.len());
        for (index, entry) in meta.universe.iter().enumerate() {
            let symbol = clean_hyperliquid_symbol(&entry.name);
            if let Some(symbols) = requested.as_ref() {
                if !symbols.contains(&symbol) {
                    continue;
                }
            }
            let Some(ctx) = ctxs.get(index) else {
                continue;
            };
            if let Some(row) = parse_mark_index_for_venue(self.adapter_name(), &symbol, ctx) {
                out.push(row);
            }
        }
        Ok(out)
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let normalized = self.normalize_symbol(symbol);
        Ok(unverified_index_composition(
            self.adapter_name(),
            &normalized,
            self.api_coin(symbol),
            "hyperliquid oracle methodology only; per-contract component list not exposed",
            "Hyperliquid exposes oracle/asset context methodology in this product path, not a verified per-contract component list.",
        ))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        self.spot_ticks(symbols).await
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        self.ws_spot_snapshot(symbols).await
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        self.spot_orderbook(symbol, depth).await
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        self.ws_spot_orderbook_snapshot(symbol, depth).await
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let coin = self.api_coin(symbol);
        if let Some(book) = self.ws_orderbook(&coin, depth) {
            return Ok(book);
        }

        let normalized = self.normalize_symbol(symbol);
        // 修复 P2 8.6：HL `l2Book` 端点固定返回 20 档（不接受档数参数，仅可选 nSigFigs/mantissa）；
        // 上层若请求 depth ≤ 20，客户端截断即可，避免请求方误以为返回 depth 档而下游错算。
        // depth = 0 视为"不限"，直接保留 20 档全部。
        // 文档：<https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#l2-book-snapshot>
        let body = json!({"type": "l2Book", "coin": coin});
        let book: L2Book = self.post_info(body).await?;
        let mut bids = parse_levels(&book.levels[0]);
        let mut asks = parse_levels(&book.levels[1]);
        if depth > 0 {
            let cap = depth as usize;
            bids.truncate(cap);
            asks.truncate(cap);
        }
        Ok(OrderBookInfo {
            symbol: normalized,
            exchange: self.adapter_name().into(),
            bids,
            asks,
            timestamp: book.time,
        })
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let coin = self.api_coin(symbol);
        Ok(self
            .ws_orderbook(&coin, depth)
            .map_or(PublicWsSnapshot::Pending, |book| {
                PublicWsSnapshot::Ready(vec![book])
            }))
    }

    async fn get_balance(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        let user = self.require_user()?;
        let body = self.private_state_body("clearinghouseState", &user);
        let state: ClearinghouseState = self.post_info(body).await?;
        parse_perp_balance(&state, currency)
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        let user = self.require_user()?;
        let state_body = self.private_state_body("clearinghouseState", &user);
        let state: ClearinghouseState = self.post_info(state_body).await?;
        let target = symbol.map(|s| self.api_coin(s));
        if !state.has_position_rows() {
            return parse_positions(
                state,
                target.as_deref(),
                &HashMap::new(),
                self.adapter_name(),
            );
        }

        // `clearinghouseState` costs two official weight units. The mark context costs twenty,
        // so fetch it only for an account that actually has position rows. `market_context` also
        // supplies the correct builder DEX scope and shares the existing single-flight cache.
        let pair = self.market_context().await.map_err(|error| {
            ExchangeError::Parse(format!(
                "hyperliquid metaAndAssetCtxs required for position mark price: {error}"
            ))
        })?;
        let (meta, ctxs) = pair.as_ref();
        let mark_map = meta
            .universe
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                ctxs.get(index)
                    .and_then(|context| context.mark_px.parse::<f64>().ok())
                    .filter(|price| price.is_finite() && *price > 0.0)
                    .map(|price| (entry.name.clone(), price))
            })
            .collect();
        parse_positions(state, target.as_deref(), &mark_map, self.adapter_name())
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        let user = self.require_user()?;
        // 修复 P2 8.7：用 `frontendOpenOrders` 而非 `openOrders`，返回 orderType / tif /
        // reduceOnly / isTrigger 等字段，便于精确识别 Limit/Market/PostOnly/StopLoss/TakeProfit。
        // 文档：<https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#frontend-open-orders>
        let body = self.private_state_body("frontendOpenOrders", &user);
        let orders: Vec<OpenOrderItem> = self.post_info(body).await?;
        let fills = if open_orders_need_fill_evidence(&orders)? {
            self.user_fills(&user).await?
        } else {
            Vec::new()
        };
        let target = symbol.map(|s| self.api_coin(s));
        parse_open_orders_with_fills(orders, &fills, target.as_deref(), self.adapter_name())
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        // Hyperliquid 已是基础币种；先用通用 strip 工具，确保兼容外部传入的旧后缀
        crate::adapter::strip_common_suffixes(&clean_hyperliquid_symbol(symbol))
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        self.api_coin(symbol)
    }
}

impl Hyperliquid {
    async fn user_fills(&self, user: &str) -> ExchangeResult<Vec<UserFillItem>> {
        // Official `userFills` rows carry oid/px/sz/fee; aggregation stays false
        // so weighted price and total fee are reconstructed without losing rows.
        // <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#retrieve-a-users-fills>
        self.post_info(json!({
            "type": "userFills",
            "user": user,
            "aggregateByTime": false
        }))
        .await
    }

    async fn market_context(&self) -> ExchangeResult<MarketContext> {
        let key = self.metadata_cache_key();
        let fetch = self.post_info(self.meta_body());
        market_contexts()
            .try_get_with(key, async { fetch.await.map(Arc::new) })
            .await
            .map_err(|error| ExchangeError::Api {
                exchange: self.adapter_name().to_owned(),
                code: "market_context_fetch_failed".to_owned(),
                message: error.to_string(),
            })
    }

    async fn predicted_fundings(&self) -> ExchangeResult<Arc<PredictedFundingsResponse>> {
        let key = self.base_url.clone();
        let fetch = self.post_info(json!({"type": "predictedFundings"}));
        predicted_fundings()
            .try_get_with(key, async { fetch.await.map(Arc::new) })
            .await
            .map_err(|error| ExchangeError::Api {
                exchange: self.adapter_name().to_owned(),
                code: "predicted_fundings_fetch_failed".to_owned(),
                message: error.to_string(),
            })
    }

    fn metadata_cache_key(&self) -> HyperliquidMetadataCacheKey {
        HyperliquidMetadataCacheKey {
            base_url: self.base_url.clone(),
            dex: self.config.market.dex().map(str::to_owned),
        }
    }

    fn cached_instrument_spec(&self, symbol: &str) -> ExchangeResult<HyperliquidInstrumentSpec> {
        let key = self.metadata_cache_key();
        let cache = instrument_caches()
            .get(&key)
            .map(|entry| Arc::clone(entry.value()));
        let cache = cache.ok_or_else(|| ExchangeError::Api {
            exchange: self.adapter_name().to_owned(),
            code: "metadata_cache_missing".to_owned(),
            message: "hyperliquid order metadata cache is cold; refresh instruments before trading"
                .to_owned(),
        })?;
        cache.resolve(&self.normalize_symbol(symbol), now_ms())
    }

    fn compile_order_action(&self, intent: &OrderIntent) -> ExchangeResult<serde_json::Value> {
        let spec = self.cached_instrument_spec(&intent.symbol)?;
        hyperliquid_order_action(&spec, intent)
    }

    async fn query_order_status(
        &self,
        user: &str,
        oid: serde_json::Value,
        target: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let body = json!({"type": "orderStatus", "user": user, "oid": oid});
        let payload: OrderStatusPayload = self.post_info(body).await?;
        let fills = if order_status_needs_fill_evidence(&payload)? {
            self.user_fills(user).await?
        } else {
            Vec::new()
        };
        order_status_to_info_with_fills(payload, &fills, target, self.adapter_name())
    }
}

fn instrument_caches(
) -> &'static DashMap<HyperliquidMetadataCacheKey, Arc<HyperliquidInstrumentCache>> {
    INSTRUMENT_CACHES.get_or_init(DashMap::new)
}

fn market_contexts() -> &'static MarketContextCache {
    MARKET_CONTEXTS.get_or_init(|| {
        Cache::builder()
            .max_capacity(32)
            .time_to_live(MARKET_CONTEXT_TTL)
            .build()
    })
}

fn predicted_fundings() -> &'static Cache<String, Arc<PredictedFundingsResponse>> {
    PREDICTED_FUNDINGS.get_or_init(|| {
        Cache::builder()
            .max_capacity(8)
            .time_to_live(MARKET_CONTEXT_TTL)
            .build()
    })
}

#[path = "hyperliquid_spot_ws.rs"]
mod spot_ws;
#[path = "hyperliquid_support.rs"]
mod support;
#[path = "hyperliquid_ws_cache.rs"]
mod ws_cache;

#[cfg(test)]
#[path = "hyperliquid_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "hyperliquid_compiler_tests.rs"]
mod compiler_tests;
