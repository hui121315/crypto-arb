use super::{Binance, BinanceConfig, BinanceCredentials, NAME};
use crate::adapters::binance_config::{
    PROD_BASE, SPOT_PROD_BASE, TESTNET_BASE, TIME_SYNC_INTERVAL_MS,
};
use crate::adapters::binance_exchange_info::BinanceInstrumentSpec;
use crate::adapters::binance_format::serialize_query;
use crate::adapters::{
    binance_metadata, binance_time, binance_user_stream as user_stream, binance_ws_trade,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::signing::binance as sign;
use crate::venue_spec::VenueId;
use common::time::now_ms;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

impl Binance {
    pub fn new(config: BinanceConfig) -> ExchangeResult<Self> {
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
            VenueId::Binance.defaults().qps,
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
            exchange_info_cache: Default::default(),
            funding_intervals: Default::default(),
            time_offset_ms: AtomicI64::new(0),
            time_synced_at_ms: AtomicI64::new(0),
            position_mode_code: AtomicI64::new(super::POSITION_MODE_UNKNOWN),
            position_mode_source_code: AtomicI64::new(0),
            position_mode_fetched_at_ms: AtomicI64::new(0),
            market_stream: OnceLock::new(),
            listed_usdm: Arc::new(dashmap::DashMap::new()),
        })
    }

    pub(super) fn require_credentials(&self) -> ExchangeResult<&BinanceCredentials> {
        self.config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("binance: missing credentials".into()))
    }

    pub(super) fn signed_query(&self, params: &[(&str, &str)]) -> ExchangeResult<(String, String)> {
        let creds = self.require_credentials()?;
        let offset = self.time_offset_ms.load(Ordering::Relaxed);
        let timestamp = now_ms().saturating_add(offset).to_string();
        let signed = signed_query_at(params, &timestamp, &creds.api_secret);
        Ok((signed, creds.api_key.clone()))
    }

    pub(super) async fn sync_server_time(&self) -> ExchangeResult<()> {
        binance_time::sync_server_time(
            &self.http,
            &self.base_url,
            &self.time_offset_ms,
            &self.time_synced_at_ms,
            TIME_SYNC_INTERVAL_MS,
        )
        .await
    }

    pub(super) fn spot_base_url(&self) -> &str {
        self.config
            .base_url_override
            .as_deref()
            .unwrap_or(SPOT_PROD_BASE)
    }

    pub async fn start_user_data_stream(&self) -> ExchangeResult<String> {
        let api_key = self.require_credentials()?.api_key.clone();
        if self.use_ws_request_api() {
            match binance_ws_trade::start_user_data_stream(self.ws_trade_config()?).await {
                Ok(listen_key) => return Ok(listen_key),
                Err(error) => tracing::warn!(
                    %error,
                    operation = "userDataStream.start",
                    "binance ws user-stream start failed; falling back to REST"
                ),
            }
        }
        user_stream::start(&self.http, &self.base_url, &api_key).await
    }

    pub async fn keepalive_user_data_stream(&self) -> ExchangeResult<String> {
        let api_key = self.require_credentials()?.api_key.clone();
        if self.use_ws_request_api() {
            match binance_ws_trade::keepalive_user_data_stream(self.ws_trade_config()?).await {
                Ok(listen_key) => return Ok(listen_key),
                Err(error) => tracing::warn!(
                    %error,
                    operation = "userDataStream.ping",
                    "binance ws user-stream keepalive failed; falling back to REST"
                ),
            }
        }
        user_stream::keepalive(&self.http, &self.base_url, &api_key).await
    }

    pub async fn close_user_data_stream(&self) -> ExchangeResult<()> {
        let api_key = self.require_credentials()?.api_key.clone();
        if self.use_ws_request_api() {
            match binance_ws_trade::close_user_data_stream(self.ws_trade_config()?).await {
                Ok(()) => return Ok(()),
                Err(error) => tracing::warn!(
                    %error,
                    operation = "userDataStream.stop",
                    "binance ws user-stream close failed; falling back to REST"
                ),
            }
        }
        user_stream::close(&self.http, &self.base_url, &api_key).await
    }

    pub(super) async fn refresh_funding_intervals(&self) -> ExchangeResult<()> {
        binance_metadata::refresh_funding_intervals(
            &self.http,
            &self.base_url,
            &self.funding_intervals,
        )
        .await
    }

    pub(super) fn funding_interval_for(&self, symbol: &str) -> u32 {
        binance_metadata::funding_interval_for(&self.funding_intervals, symbol)
    }

    pub(super) async fn instrument_spec(
        &self,
        symbol: &str,
    ) -> ExchangeResult<BinanceInstrumentSpec> {
        binance_metadata::instrument_spec(
            &self.http,
            &self.base_url,
            &self.exchange_info_cache,
            symbol,
        )
        .await
    }

    pub(super) async fn fetch_instruments_inner(
        &self,
    ) -> ExchangeResult<Vec<shared_types::instrument_registry::VenueInstrument>> {
        binance_metadata::fetch_instruments(&self.http, &self.base_url).await
    }
}

fn signed_query_at(params: &[(&str, &str)], timestamp: &str, secret: &str) -> String {
    let mut query: Vec<(&str, &str)> = params.to_vec();
    query.push(("timestamp", timestamp));
    let qs = serialize_query(&query);
    let signature = sign::sign_query(secret.as_bytes(), &qs);
    format!("{qs}&signature={signature}")
}

#[cfg(test)]
mod tests {
    use super::signed_query_at;

    #[test]
    fn signed_query_matches_official_hmac_example() {
        let secret = "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j";
        let params = [
            ("symbol", "LTCBTC"),
            ("side", "BUY"),
            ("type", "LIMIT"),
            ("timeInForce", "GTC"),
            ("quantity", "1"),
            ("price", "0.1"),
            ("recvWindow", "5000"),
        ];
        let signed = signed_query_at(&params, "1499827319559", secret);

        assert_eq!(signed, "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559&signature=c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71");
    }
}
