//! KuCoin adapter construction, signing, WS and metadata support helpers.

use super::{
    Kucoin, KucoinConfig, KucoinCredentials, NAME, PROD_BASE, SPOT_PROD_BASE, TIME_SYNC_INTERVAL_MS,
};
use crate::adapters::kucoin_contracts::KucoinContractMultipliers;
use crate::adapters::kucoin_private_rest as private_rest;
use crate::adapters::kucoin_public_rest as public_rest;
use crate::adapters::kucoin_ws_market::MarketStream as WsMarketStream;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::signing::kucoin as sign;
use crate::venue_spec::VenueId;
use common::time::now_ms;
use shared_types::OrderBookInfo;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

impl Kucoin {
    pub fn new(config: KucoinConfig) -> ExchangeResult<Self> {
        let base_url = config
            .base_url_override
            .clone()
            .unwrap_or_else(|| PROD_BASE.to_owned());
        let rate_limiter = Arc::new(RateLimiter::with_shared_budget(
            NAME,
            config.qps,
            NAME,
            VenueId::Kucoin.defaults().qps,
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
            position_mode_code: AtomicI64::new(super::POSITION_MODE_UNKNOWN),
            position_mode_fetched_at_ms: AtomicI64::new(0),
            contract_multipliers: KucoinContractMultipliers::default(),
            market_stream: OnceLock::new(),
        })
    }

    pub(super) fn ws_orderbook(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        if self.config.base_url_override.is_some() || !ws_orderbook_depth_supported(depth) {
            return None;
        }
        let stream = self.market_stream.get_or_init(|| {
            let Ok(http) = HttpClient::builder("kucoin-ws")
                .timeout_secs(self.config.timeout_secs)
                .build()
            else {
                tracing::warn!("kucoin ws http client init failed");
                return None;
            };
            Some(WsMarketStream::new(http))
        });
        let stream = stream.as_ref()?;
        stream.touch(symbol);
        stream.latest(symbol, depth)
    }

    pub(super) async fn sync_server_time(&self) -> ExchangeResult<()> {
        let last = self.time_synced_at_ms.load(Ordering::Relaxed);
        let now = now_ms();
        if last != 0 && now.saturating_sub(last) < TIME_SYNC_INTERVAL_MS {
            return Ok(());
        }
        let local_before = now_ms();
        let server_ms = public_rest::server_time(&self.http, &self.base_url).await?;
        let local_after = now_ms();
        let local_at_server = (local_before + local_after) / 2;
        let offset = server_ms.saturating_sub(local_at_server);
        self.time_offset_ms.store(offset, Ordering::Relaxed);
        self.time_synced_at_ms.store(now_ms(), Ordering::Relaxed);
        tracing::debug!(offset_ms = offset, "kucoin server time synced");
        Ok(())
    }

    pub(super) fn require_credentials(&self) -> ExchangeResult<&KucoinCredentials> {
        self.config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("kucoin: missing credentials".into()))
    }

    pub(super) fn spot_base_url(&self) -> &str {
        self.config
            .base_url_override
            .as_deref()
            .unwrap_or(SPOT_PROD_BASE)
    }

    pub(super) fn build_signed_headers(
        &self,
        method: &str,
        request_path: &str,
        body: &str,
    ) -> ExchangeResult<private_rest::SignedHeaders> {
        let creds = self.require_credentials()?;
        let offset = self.time_offset_ms.load(Ordering::Relaxed);
        let timestamp = now_ms().saturating_add(offset).to_string();
        let signature = sign::sign(
            creds.api_secret.as_bytes(),
            &timestamp,
            method,
            request_path,
            body,
        );
        let encrypted_pp = sign::encrypt_passphrase(creds.api_secret.as_bytes(), &creds.passphrase);
        Ok([
            ("KC-API-KEY".into(), creds.api_key.clone()),
            ("KC-API-SIGN".into(), signature),
            ("KC-API-TIMESTAMP".into(), timestamp),
            ("KC-API-PASSPHRASE".into(), encrypted_pp),
            ("KC-API-KEY-VERSION".into(), sign::KEY_VERSION.into()),
        ])
    }

    pub(super) async fn contract_order_unit(&self, symbol: &str) -> ExchangeResult<f64> {
        self.contract_multipliers
            .order_unit(&self.http, &self.base_url, symbol)
            .await
    }

    pub(super) async fn contract_native_symbol(&self, symbol: &str) -> ExchangeResult<String> {
        self.contract_multipliers
            .native_symbol(&self.http, &self.base_url, symbol)
            .await
    }

    pub(super) async fn refresh_contract_multipliers(&self) -> ExchangeResult<()> {
        self.contract_multipliers
            .refresh_all(&self.http, &self.base_url)
            .await
    }

    pub(super) fn ensure_write_adapter(&self) -> ExchangeResult<()> {
        if self.config.allow_live_writes {
            Ok(())
        } else {
            Err(ExchangeError::Auth(
                "kucoin live trading disabled; enable live writes explicitly".into(),
            ))
        }
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

    pub(super) fn signed_spot_request<'a>(
        &'a self,
        path: &'a str,
        headers: &'a private_rest::SignedHeaders,
    ) -> private_rest::SignedRequest<'a> {
        private_rest::SignedRequest {
            http: &self.http,
            base_url: self.spot_base_url(),
            path,
            headers,
        }
    }
}

fn ws_orderbook_depth_supported(depth: u32) -> bool {
    (1..=50).contains(&depth)
}

#[cfg(test)]
mod tests {
    use super::ws_orderbook_depth_supported;

    #[test]
    fn depth50_channel_covers_execution_depth_without_accepting_larger_books() {
        assert!(ws_orderbook_depth_supported(20));
        assert!(ws_orderbook_depth_supported(50));
        assert!(!ws_orderbook_depth_supported(0));
        assert!(!ws_orderbook_depth_supported(51));
    }
}
