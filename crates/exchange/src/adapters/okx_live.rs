//! OKX trading adapter for demo and gated live execution.
//!
//! OKX does not use a separate REST host for demo trading; write requests carry
//! `x-simulated-trading: 1`. Live mode omits that header and must be gated by
//! the API trading service before this adapter is selected.

use super::funding_payments::{okx_funding_bills_path, parse_okx_funding_payments, OkxBillRow};
use super::okx_instruments::{sizing_from_instrument, OkxInstrumentRow, OkxInstrumentRule};
use super::okx_response::data_from_text;
use super::okx_trade_data::{
    ack_from_item, cancel_order_body_json, parse_position_mode, place_order_body_json,
    place_spot_order_body_json, AccountConfigRow, OkxPositionMode, OrderAckItem,
};
use super::okx_ws_trade::{self, WsTradeConfig};
use super::spot_order_contract;
use crate::adapter::strip_common_suffixes;
use crate::adapters::okx_live_config::{
    PROD_BASE, PROD_WS_PRIVATE, SWAP_SUFFIX, TESTNET_WS_PRIVATE, TIME_SYNC_INTERVAL_MS,
};
use crate::adapters::okx_live_data::{
    parse_account_read, parse_balances, parse_order, parse_positions, AccountBalanceItem, OrderRow,
    PositionRow,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{ExchangeCapabilities, LiveTradingAdapter, VenueAccountRead};
use crate::services::RateLimiter;
use crate::signing::okx as sign;
use crate::venue_spec::VenueId;
use async_trait::async_trait;
use common::time::now_ms;
use dashmap::DashMap;
use reqwest::Method;
use serde::Deserialize;
use shared_types::{
    CancelOrderRequest, FeeProduct, FundingPaymentData, MarginMode, OrderAck, OrderInfo,
    OrderIntent, OrderSubmissionContext, PositionInfo, VenueAccountModeInfo, VenueBalanceInfo,
};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

pub use crate::adapters::okx_live_config::{OkxLiveConfig, OkxLiveCredentials, OkxTdMode};

const NAME: &str = "okx";
const POSITION_MODE_UNKNOWN: i64 = -1;
const POSITION_MODE_CACHE_TTL_MS: i64 = 30_000;
const INSTRUMENT_RULE_CACHE_TTL_MS: i64 = 60_000;
pub(super) const PLACE_ORDER_PATH: &str = "/api/v5/trade/order";
pub(super) const CANCEL_ORDER_PATH: &str = "/api/v5/trade/cancel-order";
pub(super) const GET_ORDER_PATH: &str = "/api/v5/trade/order";
pub(super) const OPEN_ORDERS_PATH: &str = "/api/v5/trade/orders-pending";

#[derive(Debug)]
pub struct OkxLive {
    cfg: OkxLiveConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    time_offset_ms: AtomicI64,
    time_synced_at_ms: AtomicI64,
    position_mode_code: AtomicI64,
    position_mode_fetched_at_ms: AtomicI64,
    instrument_rules: DashMap<String, (i64, OkxInstrumentRule)>,
}

impl OkxLive {
    pub fn new(cfg: OkxLiveConfig) -> ExchangeResult<Self> {
        let base_url = cfg
            .base_url_override
            .clone()
            .unwrap_or_else(|| PROD_BASE.to_owned());
        let rate_limiter = Arc::new(RateLimiter::with_shared_budget(
            "okx-live",
            cfg.qps,
            NAME,
            VenueId::Okx.defaults().qps,
        ));
        let http = HttpClient::builder("okx-live")
            .timeout_secs(cfg.timeout_secs)
            .rate_limiter(Arc::clone(&rate_limiter))
            .build()?;
        Ok(Self {
            cfg,
            base_url,
            http,
            _rate_limiter: rate_limiter,
            time_offset_ms: AtomicI64::new(0),
            time_synced_at_ms: AtomicI64::new(0),
            position_mode_code: AtomicI64::new(POSITION_MODE_UNKNOWN),
            position_mode_fetched_at_ms: AtomicI64::new(0),
            instrument_rules: DashMap::new(),
        })
    }

    fn headers(&self, method: &str, request_path: &str, body: &str) -> Vec<(String, String)> {
        let timestamp = self.signed_timestamp();
        let signature = sign::sign(
            self.cfg.credentials.api_secret.as_bytes(),
            &timestamp,
            method,
            request_path,
            body,
        );
        let mut headers = vec![
            ("OK-ACCESS-KEY".into(), self.cfg.credentials.api_key.clone()),
            ("OK-ACCESS-SIGN".into(), signature),
            ("OK-ACCESS-TIMESTAMP".into(), timestamp),
            (
                "OK-ACCESS-PASSPHRASE".into(),
                self.cfg.credentials.passphrase.clone(),
            ),
        ];
        if self.cfg.testnet {
            headers.push(("x-simulated-trading".into(), "1".into()));
        }
        headers
    }

    fn signed_timestamp(&self) -> String {
        let offset = self.time_offset_ms.load(Ordering::Relaxed);
        sign::iso8601_from_millis(now_ms().saturating_add(offset)).unwrap_or_else(sign::iso8601_now)
    }

    /// 校时：拉取 OKX `GET /api/v5/public/time` 计算 `server_time - local_time`。
    ///
    /// 官方文档：<https://www.okx.com/docs-v5/en/#public-data-rest-api-get-system-time>
    async fn sync_server_time(&self) -> ExchangeResult<()> {
        let last = self.time_synced_at_ms.load(Ordering::Relaxed);
        let start = now_ms();
        if start.saturating_sub(last) < TIME_SYNC_INTERVAL_MS {
            return Ok(());
        }

        #[derive(Debug, Deserialize)]
        struct TimeItem {
            ts: String,
        }

        let url = format!("{}/api/v5/public/time", self.base_url);
        let resp = self
            .http
            .execute_with_retry(|| self.http.request(Method::GET, &url))
            .await?;
        let local_after = now_ms();
        let text = resp
            .text()
            .await
            .map_err(|error| ExchangeError::Network(error.to_string()))?;
        let mut items = data_from_text::<TimeItem>(&text, "public time")?;
        let item = items
            .pop()
            .ok_or_else(|| ExchangeError::Parse("okx public time empty".into()))?;
        let server_ms = item
            .ts
            .parse::<i64>()
            .map_err(|error| ExchangeError::Parse(format!("okx public time ts: {error}")))?;
        let local_midpoint = start.saturating_add((local_after - start) / 2);
        self.time_offset_ms
            .store(server_ms.saturating_sub(local_midpoint), Ordering::Relaxed);
        self.time_synced_at_ms.store(local_after, Ordering::Relaxed);
        Ok(())
    }

    async fn sync_server_time_best_effort(&self) {
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(exchange = NAME, error = %error, "okx server time sync failed; using local timestamp");
        }
    }

    async fn request_data<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        request_path: &str,
        body: Option<String>,
        context: &str,
    ) -> ExchangeResult<Vec<T>> {
        self.sync_server_time_best_effort().await;
        let method_str = method.as_str();
        let body_str = body.as_deref().unwrap_or("");
        let headers = self.headers(method_str, request_path, body_str);
        let url = format!("{}{}", self.base_url, request_path);
        let resp = self
            .http
            .execute_with_retry(|| {
                let mut req = self.http.request(method.clone(), &url);
                for (k, v) in &headers {
                    req = req.header(k, v);
                }
                if let Some(body) = &body {
                    req = req
                        .header("Content-Type", "application/json")
                        .body(body.clone());
                }
                req
            })
            .await?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ExchangeError::Network(e.to_string()))?;
        if !status.is_success() {
            return Err(ExchangeError::Http {
                status: status.as_u16(),
                body: text,
            });
        }
        data_from_text(&text, context)
    }

    async fn request_one<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        request_path: &str,
        body: Option<String>,
        context: &str,
    ) -> ExchangeResult<T> {
        let mut rows = self
            .request_data(method, request_path, body, context)
            .await?;
        rows.pop()
            .ok_or_else(|| ExchangeError::Parse(format!("okx {context} empty response")))
    }

    async fn request_public_one<T: serde::de::DeserializeOwned>(
        &self,
        request_path: &str,
        context: &str,
    ) -> ExchangeResult<T> {
        let url = format!("{}{}", self.base_url, request_path);
        let resp = self
            .http
            .execute_with_retry(|| self.http.request(Method::GET, &url))
            .await?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|error| ExchangeError::Network(error.to_string()))?;
        if !status.is_success() {
            return Err(ExchangeError::Http {
                status: status.as_u16(),
                body: text,
            });
        }
        let mut rows = data_from_text::<T>(&text, context)?;
        rows.pop()
            .ok_or_else(|| ExchangeError::Parse(format!("okx {context} empty response")))
    }

    async fn request_optional_one<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        request_path: &str,
        body: Option<String>,
        context: &str,
    ) -> ExchangeResult<Option<T>> {
        let mut rows = self
            .request_data(method, request_path, body, context)
            .await?;
        Ok(rows.pop())
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        let norm = strip_common_suffixes(symbol);
        format!("{norm}{SWAP_SUFFIX}")
    }

    fn ws_trade_url(&self) -> &'static str {
        if self.cfg.testnet {
            TESTNET_WS_PRIVATE
        } else {
            PROD_WS_PRIVATE
        }
    }

    /// 是否使用 WebSocket trade endpoint 下单 / 撤单（修复 P2 2.11）。
    ///
    /// 业务约定：
    /// - 默认（生产 / testnet）走 **WS** 下单（更低延迟、更高吞吐）。
    /// - 当 `base_url_override` 被设置（典型为 mock REST URL，集成测试场景），
    ///   降级到 REST 路径，方便 wiremock 等工具拦截 HTTP 请求验证签名。
    ///
    /// 该 helper 让"REST vs WS"判断有自描述的名字，避免在调用点暴露
    /// `base_url_override.is_none()` 这类含糊条件。
    fn use_ws_trade(&self) -> bool {
        self.cfg.base_url_override.is_none()
    }

    fn ws_trade_config(&self) -> WsTradeConfig<'_> {
        WsTradeConfig {
            url: self.ws_trade_url(),
            api_key: &self.cfg.credentials.api_key,
            api_secret: &self.cfg.credentials.api_secret,
            passphrase: &self.cfg.credentials.passphrase,
            timeout_secs: self.cfg.timeout_secs,
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
            mode: mode.as_account_mode().to_owned(),
            source: "okx.GET /api/v5/account/config".to_owned(),
            checked_at_ms,
            freshness_ms,
            account_scope: None,
        })
    }

    async fn position_mode(&self) -> ExchangeResult<OkxPositionMode> {
        if let Some(mode) = self.cached_position_mode() {
            return Ok(mode);
        }
        let row = self
            .request_one::<AccountConfigRow>(
                Method::GET,
                "/api/v5/account/config",
                None,
                "account config",
            )
            .await?;
        let mode = parse_position_mode(&row)?;
        self.position_mode_code
            .store(position_mode_code(mode), Ordering::Relaxed);
        self.position_mode_fetched_at_ms
            .store(now_ms(), Ordering::Relaxed);
        Ok(mode)
    }

    fn cached_position_mode(&self) -> Option<OkxPositionMode> {
        let fetched_at = self.position_mode_fetched_at_ms.load(Ordering::Relaxed);
        if fetched_at == 0 || now_ms().saturating_sub(fetched_at) > POSITION_MODE_CACHE_TTL_MS {
            return None;
        }
        position_mode_from_code(self.position_mode_code.load(Ordering::Relaxed))
    }

    async fn instrument_rule(&self, inst_id: &str) -> ExchangeResult<OkxInstrumentRule> {
        if self.cfg.base_url_override.is_none() && !self.cfg.testnet {
            let stream = super::okx_ws_instruments::production_stream();
            if let Some(rule) = stream.latest_rule(inst_id) {
                return Ok(rule);
            }
        }
        if let Some(rule) = self.cached_instrument_rule(inst_id) {
            return Ok(rule);
        }
        let path = instrument_rule_path(inst_id);
        let row = self
            .request_public_one::<OkxInstrumentRow>(&path, "instrument rule")
            .await?;
        let rule = OkxInstrumentRule::from_row(row)?;
        self.instrument_rules
            .insert(inst_id.to_owned(), (now_ms(), rule.clone()));
        Ok(rule)
    }

    fn cached_instrument_rule(&self, inst_id: &str) -> Option<OkxInstrumentRule> {
        self.instrument_rules.get(inst_id).and_then(|entry| {
            let (fetched_at_ms, rule) = entry.value();
            (now_ms().saturating_sub(*fetched_at_ms) <= INSTRUMENT_RULE_CACHE_TTL_MS)
                .then(|| rule.clone())
        })
    }

    async fn query_order(
        &self,
        inst_id: &str,
        identity_key: &str,
        identity_value: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let path = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer
                .append_pair("instId", inst_id)
                .append_pair(identity_key, identity_value);
            format!("{}?{}", GET_ORDER_PATH, serializer.finish())
        };
        match self
            .request_optional_one::<OrderRow>(Method::GET, &path, None, "get order")
            .await
        {
            Ok(row) => row.map(parse_order).transpose(),
            Err(error) if okx_order_not_found(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[async_trait]
impl LiveTradingAdapter for OkxLive {
    fn name(&self) -> &'static str {
        "okx"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_testnet: self.cfg.testnet,
            supports_live: !self.cfg.testnet,
            supports_spot: true,
            supports_perp: true,
            supports_limit_orders: true,
            supports_market_orders: true,
            supports_post_only: true,
            supports_reduce_only: true,
        }
    }

    fn order_margin_modes(&self) -> Vec<MarginMode> {
        match self.cfg.td_mode {
            OkxTdMode::Cross => vec![MarginMode::Cross],
            OkxTdMode::Isolated => vec![MarginMode::Isolated],
            OkxTdMode::Cash | OkxTdMode::SpotIsolated => Vec::new(),
        }
    }

    async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        Ok(Some(self.account_mode_info().await?))
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        let inst_id = self.to_exchange_symbol(&intent.symbol);
        let instrument = self.instrument_rule(&inst_id).await?;
        let sizing = sizing_from_instrument(intent, &instrument)?;
        let position_mode = self.position_mode().await?;
        if self.use_ws_trade() {
            let inst_id_code = instrument.ws_inst_id_code()?;
            return okx_ws_trade::place_order(
                self.ws_trade_config(),
                intent,
                inst_id_code,
                self.cfg.td_mode,
                position_mode,
                sizing,
            )
            .await;
        }

        let body = place_order_body_json(intent, inst_id, self.cfg.td_mode, position_mode, sizing)?;
        let item: OrderAckItem = self
            .request_one(Method::POST, PLACE_ORDER_PATH, Some(body), "place order")
            .await?;
        Ok(ack_from_item(
            intent.id.clone(),
            intent.client_order_id.clone(),
            item,
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
        let compiled = spot_order_contract::compile(NAME, intent, context)?;
        if self.use_ws_trade() {
            return okx_ws_trade::place_spot_order(
                self.ws_trade_config(),
                intent,
                compiled.native_symbol,
                compiled.quantity,
                compiled.price,
            )
            .await;
        }
        let body = place_spot_order_body_json(
            intent,
            compiled.native_symbol,
            compiled.quantity,
            compiled.price,
        )?;
        let item: OrderAckItem = self
            .request_one(
                Method::POST,
                PLACE_ORDER_PATH,
                Some(body),
                "spot place order",
            )
            .await?;
        Ok(ack_from_item(
            intent.id.clone(),
            intent.client_order_id.clone(),
            item,
        ))
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        let inst_id = self.to_exchange_symbol(&request.symbol);
        if self.use_ws_trade() {
            let instrument = self.instrument_rule(&inst_id).await?;
            return okx_ws_trade::cancel_order(
                self.ws_trade_config(),
                request,
                instrument.ws_inst_id_code()?,
            )
            .await;
        }

        let body = cancel_order_body_json(request, inst_id)?;
        let item: OrderAckItem = self
            .request_one(Method::POST, CANCEL_ORDER_PATH, Some(body), "cancel order")
            .await?;
        Ok(ack_from_item(
            request.internal_order_id.clone(),
            request.client_order_id.clone(),
            item,
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
        let inst_id = spot_order_contract::cancel_symbol(NAME, request, context)?;
        if self.use_ws_trade() {
            return okx_ws_trade::cancel_spot_order(self.ws_trade_config(), request, inst_id).await;
        }
        let body = cancel_order_body_json(request, inst_id)?;
        let item: OrderAckItem = self
            .request_one(
                Method::POST,
                CANCEL_ORDER_PATH,
                Some(body),
                "spot cancel order",
            )
            .await?;
        Ok(ack_from_item(
            request.internal_order_id.clone(),
            request.client_order_id.clone(),
            item,
        ))
    }

    async fn get_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        // 修复 P2 2.8：原 `format!` 直接拼接 client_order_id，未做 URL encode。
        // OKX `clOrdId` 用户可控，可能含 `&` `=` `+` 等需要 percent encoding 的字符；
        // 用 `url::form_urlencoded` 标准化，避免签名 path 与实际请求 query 不一致。
        // 注意：`Serializer` 非 `Send`，必须在 await 前限制其生命周期到 block 内。
        let path = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer
                .append_pair("instId", &self.to_exchange_symbol(symbol))
                .append_pair("clOrdId", client_order_id);
            format!("{}?{}", GET_ORDER_PATH, serializer.finish())
        };
        match self
            .request_optional_one::<OrderRow>(Method::GET, &path, None, "get order")
            .await
        {
            Ok(row) => row.map(parse_order).transpose(),
            Err(error) if okx_order_not_found(&error) => Ok(None),
            Err(error) => Err(error),
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
        let inst_id = spot_order_contract::query_symbol(NAME, symbol, context)?;
        self.query_order(&inst_id, "clOrdId", client_order_id).await
    }

    async fn get_order_by_exchange_order_id_with_context(
        &self,
        symbol: &str,
        exchange_order_id: &str,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let inst_id = if context.product == FeeProduct::Spot {
            spot_order_contract::query_symbol(NAME, symbol, context)?
        } else {
            self.to_exchange_symbol(symbol)
        };
        self.query_order(&inst_id, "ordId", exchange_order_id).await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        // 修复 P2 2.8：与 `get_order` 一致地用 `url::form_urlencoded` 编码 instId。
        // 虽然 instId 本身（如 `BTC-USDT-SWAP`）不含特殊字符，统一编码减少认知负担。
        // `Serializer` 非 `Send`，限制到 block 内不跨越 await。
        let path = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("instType", "SWAP");
            if let Some(symbol) = symbol {
                serializer.append_pair("instId", &self.to_exchange_symbol(symbol));
            }
            format!("{}?{}", OPEN_ORDERS_PATH, serializer.finish())
        };
        let rows = self
            .request_data::<OrderRow>(Method::GET, &path, None, "orders pending")
            .await?;
        rows.into_iter().map(parse_order).collect()
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        let rows = self
            .request_data::<AccountBalanceItem>(
                Method::GET,
                "/api/v5/account/balance",
                None,
                "account balance",
            )
            .await?;
        parse_balances(rows, currency)
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        let rows = self
            .request_data::<AccountBalanceItem>(
                Method::GET,
                "/api/v5/account/balance",
                None,
                "account balance",
            )
            .await?;
        parse_account_read(rows, currency, now_ms())
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        let rows = self
            .request_data::<PositionRow>(
                Method::GET,
                "/api/v5/account/positions?instType=SWAP",
                None,
                "positions",
            )
            .await?;
        let target = symbol.map(|s| self.to_exchange_symbol(s));
        parse_positions(&rows, target.as_deref())
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        let inst_id = symbol.map(|value| self.to_exchange_symbol(value));
        let path = okx_funding_bills_path(inst_id.as_deref(), start_time_ms, end_time_ms);
        let rows = self
            .request_data::<OkxBillRow>(Method::GET, &path, None, "funding payments")
            .await?;
        parse_okx_funding_payments(rows)
    }
}

fn position_mode_code(mode: OkxPositionMode) -> i64 {
    match mode {
        OkxPositionMode::Net => 0,
        OkxPositionMode::LongShort => 1,
    }
}

fn position_mode_from_code(value: i64) -> Option<OkxPositionMode> {
    match value {
        0 => Some(OkxPositionMode::Net),
        1 => Some(OkxPositionMode::LongShort),
        _ => None,
    }
}

fn instrument_rule_path(inst_id: &str) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("instType", "SWAP")
        .append_pair("instId", inst_id);
    format!("/api/v5/public/instruments?{}", serializer.finish())
}

fn okx_order_not_found(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::Api { exchange, code, .. }
            if exchange.eq_ignore_ascii_case(NAME) && code == "51603"
    )
}

#[cfg(test)]
#[path = "okx_live_tests.rs"]
mod tests;
