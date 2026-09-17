use super::{Okx, OkxConfig, OkxCredentials, NAME};
use crate::adapters::okx_config::{FUNDING_RATE_CONCURRENCY, PROD_BASE, SWAP_SUFFIX};
use crate::adapters::okx_funding::FundingRateItem;
use crate::adapters::okx_instruments::OkxOrderSizing;
use crate::adapters::okx_live_config::OkxTdMode;
use crate::adapters::okx_private_rest as private_rest;
use crate::adapters::okx_public_rest as public_rest;
use crate::adapters::okx_trade_data::{pre_check_order_body_json, OkxPositionMode};
use crate::adapters::okx_ws_funding::FundingStream as WsFundingStream;
use crate::adapters::okx_ws_mark_index::MarkIndexStream as WsMarkIndexStream;
use crate::adapters::okx_ws_market::MarketStream as WsMarketStream;
use crate::adapters::okx_ws_spot_ticker::SpotTickerStream as WsSpotTickerStream;
use crate::adapters::okx_ws_ticker::TickerStream as WsTickerStream;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::signing::okx as sign;
use crate::venue_spec::VenueId;
use common::time::now_ms;
use futures::stream::{self, StreamExt};
use shared_types::{
    ExecutionMode, MarginMode, OrderBookInfo, OrderIntent, OrderSide, OrderSource, OrderType,
    TimeInForce, VenueAccountModeInfo,
};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

const TIME_SYNC_INTERVAL_MS: i64 = 5 * 60 * 1000;
const SAFE_ORDER_PRECHECK_SYMBOL: &str = "BTC";
const SAFE_ORDER_PRECHECK_SIZE: &str = "1";
const SAFE_ORDER_PRECHECK_PRICE: &str = "100000";

impl Okx {
    pub fn new(config: OkxConfig) -> ExchangeResult<Self> {
        let base_url = config
            .base_url_override
            .clone()
            .unwrap_or_else(|| PROD_BASE.to_owned());

        let rate_limiter = Arc::new(RateLimiter::with_shared_budget(
            NAME,
            config.qps,
            NAME,
            VenueId::Okx.defaults().qps,
        ));
        let http = HttpClient::builder(NAME)
            .timeout_secs(config.timeout_secs)
            .rate_limiter(Arc::clone(&rate_limiter))
            .build()?;

        Ok(Self {
            config,
            base_url,
            http,
            _rate_limiter: rate_limiter,
            time_offset_ms: AtomicI64::new(0),
            time_synced_at_ms: AtomicI64::new(0),
            funding_stream: OnceLock::new(),
            mark_index_stream: OnceLock::new(),
            market_stream: OnceLock::new(),
            ticker_stream: OnceLock::new(),
            spot_ticker_stream: OnceLock::new(),
            contract_values: Default::default(),
        })
    }

    pub(super) fn ws_funding(&self) -> Option<Arc<WsFundingStream>> {
        if self.config.base_url_override.is_some() {
            return None;
        }
        Some(Arc::clone(
            self.funding_stream.get_or_init(WsFundingStream::new),
        ))
    }

    pub(super) fn ws_ticker(&self) -> Option<Arc<WsTickerStream>> {
        if self.config.base_url_override.is_some() {
            return None;
        }
        Some(Arc::clone(
            self.ticker_stream.get_or_init(WsTickerStream::new),
        ))
    }

    pub(super) fn ws_mark_index(&self) -> Option<Arc<WsMarkIndexStream>> {
        if self.config.base_url_override.is_some() {
            return None;
        }
        Some(Arc::clone(
            self.mark_index_stream.get_or_init(WsMarkIndexStream::new),
        ))
    }

    pub(super) fn ws_spot_ticker(&self) -> Option<Arc<WsSpotTickerStream>> {
        if self.config.base_url_override.is_some() {
            return None;
        }
        Some(Arc::clone(
            self.spot_ticker_stream.get_or_init(WsSpotTickerStream::new),
        ))
    }

    pub(super) fn sync_instrument_updates(&self) {
        if self.config.base_url_override.is_some() {
            return;
        }
        let stream = crate::adapters::okx_ws_instruments::production_stream();
        self.contract_values.apply_ws_rules(stream.rules());
    }

    pub(super) fn ws_orderbook(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        if self.config.base_url_override.is_some() || !ws_orderbook_depth_supported(depth) {
            return None;
        }
        let stream = self.market_stream.get_or_init(WsMarketStream::new);
        stream.touch(symbol);
        stream.latest(symbol, depth)
    }

    pub(super) fn build_signed_headers(
        &self,
        method: &str,
        request_path: &str,
        body: &str,
    ) -> ExchangeResult<private_rest::SignedHeaders> {
        let creds = self.require_credentials()?;
        let timestamp = self.signed_timestamp();
        let signature = sign::sign(
            creds.api_secret.as_bytes(),
            &timestamp,
            method,
            request_path,
            body,
        );
        Ok([
            ("OK-ACCESS-KEY".into(), creds.api_key.clone()),
            ("OK-ACCESS-SIGN".into(), signature),
            ("OK-ACCESS-TIMESTAMP".into(), timestamp),
            ("OK-ACCESS-PASSPHRASE".into(), creds.passphrase.clone()),
        ])
    }

    pub(super) fn signed_request<'a>(
        &'a self,
        path: &'a str,
        headers: &'a private_rest::SignedHeaders,
    ) -> private_rest::SignedRequest<'a> {
        private_rest::SignedRequest {
            http: &self.http,
            base_url: &self.base_url,
            path,
            headers,
        }
    }

    pub(super) async fn sync_server_time_best_effort(&self) {
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(exchange = NAME, error = %error, "okx server time sync failed; using local timestamp");
        }
    }

    pub async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        self.sync_server_time_best_effort().await;
        let path = "/api/v5/account/config";
        let headers = self.build_signed_headers("GET", path, "")?;
        let mode =
            private_rest::account_position_mode(&self.signed_request(path, &headers)).await?;
        Ok(Some(VenueAccountModeInfo {
            venue: NAME.to_owned(),
            mode: mode.as_account_mode().to_owned(),
            source: "okx.GET /api/v5/account/config".to_owned(),
            checked_at_ms: now_ms(),
            freshness_ms: Some(0),
            account_scope: None,
        }))
    }

    pub async fn validate_safe_order_pre_check_permission(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        self.sync_server_time_best_effort().await;
        let intent = Self::safe_order_pre_check_intent();
        let inst_id = format!("{SAFE_ORDER_PRECHECK_SYMBOL}{SWAP_SUFFIX}");
        let sizing = OkxOrderSizing {
            sz: SAFE_ORDER_PRECHECK_SIZE.to_owned(),
            px: Some(SAFE_ORDER_PRECHECK_PRICE.to_owned()),
        };
        let body = pre_check_order_body_json(
            &intent,
            inst_id,
            OkxTdMode::Cross,
            OkxPositionMode::Net,
            sizing,
        )?;
        let path = "/api/v5/trade/order-precheck";
        let headers = self.build_signed_headers("POST", path, &body)?;
        private_rest::pre_check_order(&self.signed_request(path, &headers), body).await
    }

    fn safe_order_pre_check_intent() -> OrderIntent {
        let now = now_ms();
        let client_order_id = format!("xlineprecheck{now}");
        OrderIntent {
            id: client_order_id.clone(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: NAME.to_owned(),
            symbol: SAFE_ORDER_PRECHECK_SYMBOL.to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(100_000.0),
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

    pub(super) async fn fetch_funding_rate_items_fast(
        &self,
        inst_ids: &[String],
    ) -> ExchangeResult<Vec<FundingRateItem>> {
        let rows = stream::iter(inst_ids.iter().cloned())
            .map(|id| async move {
                let result = public_rest::funding_rate_fast(&self.http, &self.base_url, &id).await;
                (id, result)
            })
            .buffer_unordered(FUNDING_RATE_CONCURRENCY);
        let collected: Vec<(String, ExchangeResult<FundingRateItem>)> = rows.collect().await;
        self.retry_failed_funding_items(collected).await
    }

    pub(super) fn require_credentials(&self) -> ExchangeResult<&OkxCredentials> {
        self.config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("okx: missing credentials".into()))
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

        let server_ms = public_rest::server_time(&self.http, &self.base_url).await?;
        let local_after = now_ms();
        let local_midpoint = start.saturating_add((local_after - start) / 2);
        self.time_offset_ms
            .store(server_ms.saturating_sub(local_midpoint), Ordering::Relaxed);
        self.time_synced_at_ms.store(local_after, Ordering::Relaxed);
        Ok(())
    }

    async fn retry_failed_funding_items(
        &self,
        collected: Vec<(String, ExchangeResult<FundingRateItem>)>,
    ) -> ExchangeResult<Vec<FundingRateItem>> {
        let mut ok_items = Vec::with_capacity(collected.len());
        let mut retry_ids = Vec::new();
        for (id, item) in collected {
            match item {
                Ok(it) => ok_items.push(it),
                Err(e) => {
                    tracing::warn!(exchange = NAME, inst_id = %id, error = %e, "funding-rate single fetch failed; will retry once");
                    retry_ids.push(id);
                }
            }
        }
        if !retry_ids.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            ok_items.extend(self.fetch_retry_funding_items(retry_ids).await);
        }
        Ok(ok_items)
    }

    async fn fetch_retry_funding_items(&self, retry_ids: Vec<String>) -> Vec<FundingRateItem> {
        let retry_stream = stream::iter(retry_ids)
            .map(|id| async move {
                let result = public_rest::funding_rate_fast(&self.http, &self.base_url, &id).await;
                (id, result)
            })
            .buffer_unordered(FUNDING_RATE_CONCURRENCY);
        retry_stream
            .filter_map(|(id, item)| async move {
                match item {
                    Ok(it) => Some(it),
                    Err(e) => {
                        tracing::warn!(exchange = NAME, inst_id = %id, error = %e, "funding-rate single fetch failed after retry");
                        None
                    }
                }
            })
            .collect()
            .await
    }
}

fn ws_orderbook_depth_supported(depth: u32) -> bool {
    (1..=400).contains(&depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn books_channel_covers_execution_depth_without_accepting_larger_books() {
        assert!(ws_orderbook_depth_supported(20));
        assert!(ws_orderbook_depth_supported(400));
        assert!(!ws_orderbook_depth_supported(0));
        assert!(!ws_orderbook_depth_supported(401));
    }
}
