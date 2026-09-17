//! Bybit adapter construction, signing and WS support helpers.

use super::{
    Bybit, BybitConfig, BybitCredentials, NAME, PROD_BASE, PROD_WS_TRADE, TESTNET_BASE,
    TESTNET_WS_TRADE, TIME_SYNC_INTERVAL_MS,
};
use crate::adapters::bybit_public_rest as public_rest;
use crate::adapters::bybit_ws_market::MarketStream as WsMarketStream;
use crate::adapters::bybit_ws_trade::WsTradeConfig;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::signing::bybit as sign;
use crate::venue_spec::VenueId;
use common::time::now_ms;
use shared_types::OrderBookInfo;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

impl Bybit {
    pub fn new(config: BybitConfig) -> ExchangeResult<Self> {
        let base_url = config.base_url_override.clone().unwrap_or_else(|| {
            if config.testnet {
                TESTNET_BASE.to_owned()
            } else {
                PROD_BASE.to_owned()
            }
        });
        let rate_limiter = Arc::new(RateLimiter::with_shared_budget(
            NAME,
            config.qps,
            NAME,
            VenueId::Bybit.defaults().qps,
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
            market_stream: OnceLock::new(),
        })
    }

    pub(super) fn ws_orderbook(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        if self.config.testnet || self.config.base_url_override.is_some() || depth > 50 {
            return None;
        }
        let manager = crate::adapters::bybit_ws_ticker::shared_manager(&self.config)?;
        let stream = self
            .market_stream
            .get_or_init(|| WsMarketStream::new(manager));
        stream.touch(symbol);
        stream.latest(symbol, depth)
    }

    pub(super) fn require_credentials(&self) -> ExchangeResult<&BybitCredentials> {
        self.config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("bybit: missing credentials".into()))
    }

    pub(super) async fn sync_server_time(&self) -> ExchangeResult<()> {
        let last = self.time_synced_at_ms.load(Ordering::Relaxed);
        let now = now_ms();
        if last != 0 && now.saturating_sub(last) < TIME_SYNC_INTERVAL_MS {
            return Ok(());
        }

        let local_before = now_ms();
        let server_time = public_rest::server_time(&self.http, &self.base_url).await?;
        let local_after = now_ms();
        let local_at_server = (local_before + local_after) / 2;
        let offset = server_time.saturating_sub(local_at_server);

        self.time_offset_ms.store(offset, Ordering::Relaxed);
        self.time_synced_at_ms.store(now_ms(), Ordering::Relaxed);
        tracing::debug!(offset_ms = offset, "bybit server time synced");
        Ok(())
    }

    pub(super) fn build_signed_headers(
        &self,
        payload: &str,
    ) -> ExchangeResult<[(String, String); 4]> {
        let creds = self.require_credentials()?;
        let offset = self.time_offset_ms.load(Ordering::Relaxed);
        let timestamp = now_ms().saturating_add(offset).to_string();
        let signature = sign::sign(
            creds.api_secret.as_bytes(),
            &timestamp,
            &creds.api_key,
            &self.config.recv_window,
            payload,
        );
        Ok([
            ("X-BAPI-API-KEY".into(), creds.api_key.clone()),
            ("X-BAPI-TIMESTAMP".into(), timestamp),
            ("X-BAPI-RECV-WINDOW".into(), self.config.recv_window.clone()),
            ("X-BAPI-SIGN".into(), signature),
        ])
    }

    pub(super) fn build_get_headers(&self, query: &str) -> ExchangeResult<[(String, String); 4]> {
        self.build_signed_headers(query)
    }

    pub(super) fn ensure_write_adapter(&self) -> ExchangeResult<()> {
        if self.config.testnet
            || self.config.allow_live_writes
            || self.config.base_url_override.is_some()
        {
            Ok(())
        } else {
            Err(ExchangeError::Auth(
                "bybit live trading disabled; enable live writes explicitly".into(),
            ))
        }
    }

    fn ws_trade_url(&self) -> &'static str {
        if self.config.testnet {
            TESTNET_WS_TRADE
        } else {
            PROD_WS_TRADE
        }
    }

    pub(super) fn ws_trade_config(&self) -> ExchangeResult<WsTradeConfig<'_>> {
        let credentials = self.require_credentials()?;
        Ok(WsTradeConfig {
            url: self.ws_trade_url(),
            api_key: &credentials.api_key,
            api_secret: &credentials.api_secret,
            recv_window: &self.config.recv_window,
            timeout_secs: self.config.timeout_secs,
        })
    }
}
