//! Bitget UTA adapter construction, signing and WS support helpers.

use super::{Bitget, BitgetConfig, BitgetCredentials, NAME, TIME_SYNC_INTERVAL_MS};
use crate::adapters::bitget_order_compiler::BitgetPositionMode;
use crate::adapters::bitget_uta_config::BitgetUtaCategory;
use crate::adapters::bitget_uta_config::PROD_BASE;
use crate::adapters::bitget_uta_private_rest as private_rest;
use crate::adapters::bitget_uta_public_rest as uta_public_rest;
use crate::adapters::bitget_uta_trade_data::cancel_order_body_json_for;
use crate::adapters::bitget_uta_ws_market::MarketStream as WsMarketStream;
use crate::adapters::bitget_uta_ws_trade::WsTradeConfig;
use crate::adapters::bitget_uta_ws_user;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::signing::bitget as sign;
use crate::venue_spec::VenueId;
use common::time::now_ms;
use shared_types::{CancelOrderRequest, OrderBookInfo};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

const SAFE_ORDER_CANCEL_SYMBOL: &str = "BTC";
const POSITION_MODE_TTL_MS: i64 = 30_000;

impl Bitget {
    pub fn new(config: BitgetConfig) -> ExchangeResult<Self> {
        let base_url = config
            .base_url_override
            .clone()
            .unwrap_or_else(|| PROD_BASE.to_owned());
        let rate_limiter = Arc::new(RateLimiter::with_shared_budget(
            NAME,
            config.qps,
            NAME,
            VenueId::Bitget.defaults().qps,
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
            instrument_cache: Default::default(),
            position_mode: Default::default(),
            position_mode_checked_at_ms: AtomicI64::new(0),
            market_stream: OnceLock::new(),
        })
    }

    pub(super) fn ws_orderbook(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        if self.config.base_url_override.is_some() || depth > 100 {
            return None;
        }
        let stream = self.market_stream.get_or_init(WsMarketStream::new);
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
        let Some(server_time) = uta_public_rest::server_time(&self.http, &self.base_url).await?
        else {
            return Ok(());
        };
        let local_after = now_ms();
        let local_at_server = (local_before + local_after) / 2;
        let offset = server_time.saturating_sub(local_at_server);

        self.time_offset_ms.store(offset, Ordering::Relaxed);
        self.time_synced_at_ms.store(now_ms(), Ordering::Relaxed);
        tracing::debug!(offset_ms = offset, "bitget server time synced");
        Ok(())
    }

    pub(super) fn require_credentials(&self) -> ExchangeResult<&BitgetCredentials> {
        self.config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("bitget: missing credentials".into()))
    }

    pub(super) async fn refresh_instrument_specs(
        &self,
    ) -> ExchangeResult<Vec<shared_types::instrument_registry::VenueInstrument>> {
        let (usdt, usdc, coin, spot) = tokio::try_join!(
            uta_public_rest::instruments_rest(
                &self.http,
                &self.base_url,
                BitgetUtaCategory::UsdtFutures,
            ),
            uta_public_rest::instruments_rest(
                &self.http,
                &self.base_url,
                BitgetUtaCategory::UsdcFutures,
            ),
            uta_public_rest::instruments_rest(
                &self.http,
                &self.base_url,
                BitgetUtaCategory::CoinFutures,
            ),
            uta_public_rest::instruments_rest(&self.http, &self.base_url, BitgetUtaCategory::Spot,),
        )?;
        let checked_at_ms = now_ms();
        let rows = usdt
            .into_iter()
            .chain(usdc)
            .chain(coin)
            .chain(spot)
            .collect();
        let (instruments, specs) =
            crate::adapters::bitget_instruments::instruments_and_specs_from_rows(
                rows,
                checked_at_ms,
            );
        // Spot and USDT futures can share the same native symbol (`BTCUSDT`).
        // Spot execution is ticket-bound, so keep this legacy hot-path cache
        // perpetual-only instead of allowing Spot metadata to shadow it.
        self.instrument_cache.replace(
            specs
                .into_iter()
                .filter(|spec| spec.category != BitgetUtaCategory::Spot)
                .collect(),
            checked_at_ms,
        );
        Ok(instruments)
    }

    pub(super) async fn refresh_spot_instrument_specs(
        &self,
    ) -> ExchangeResult<Vec<shared_types::instrument_registry::VenueInstrument>> {
        let rows =
            uta_public_rest::instruments_rest(&self.http, &self.base_url, BitgetUtaCategory::Spot)
                .await?;
        let (instruments, _) =
            crate::adapters::bitget_instruments::instruments_and_specs_from_rows(rows, now_ms());
        Ok(instruments)
    }

    pub(super) async fn resolve_instrument_spec(
        &self,
        requested: &str,
    ) -> ExchangeResult<crate::adapters::bitget_instruments::BitgetInstrumentSpec> {
        if !self.instrument_cache.is_fresh(now_ms()) {
            self.refresh_instrument_specs().await?;
        }
        self.instrument_cache.resolve(requested)
    }

    pub(super) async fn account_position_mode(&self) -> ExchangeResult<BitgetPositionMode> {
        let checked_at = self.position_mode_checked_at_ms.load(Ordering::Relaxed);
        let now = now_ms();
        if checked_at > 0 && now.saturating_sub(checked_at) < POSITION_MODE_TTL_MS {
            match self.position_mode.load(Ordering::Relaxed) {
                1 => return Ok(BitgetPositionMode::OneWay),
                2 => return Ok(BitgetPositionMode::Hedge),
                _ => {}
            }
        }

        let headers = self.build_signed_headers("GET", super::ACCOUNT_SETTINGS_PATH, "")?;
        let raw = private_rest::account_hold_mode(
            &self.signed_request(super::ACCOUNT_SETTINGS_PATH, &headers),
        )
        .await?;
        let mode = BitgetPositionMode::parse(&raw)?;
        self.position_mode.store(
            match mode {
                BitgetPositionMode::OneWay => 1,
                BitgetPositionMode::Hedge => 2,
            },
            Ordering::Relaxed,
        );
        self.position_mode_checked_at_ms
            .store(now_ms(), Ordering::Relaxed);
        Ok(mode)
    }

    pub(super) async fn positions_for_category(
        &self,
        category: BitgetUtaCategory,
        symbol: Option<&str>,
    ) -> ExchangeResult<Vec<shared_types::PositionInfo>> {
        let mut path = format!(
            "/api/v3/position/current-position?category={}",
            category.as_query()
        );
        if let Some(symbol) = symbol {
            path.push_str("&symbol=");
            path.push_str(symbol);
        }
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::positions(&self.signed_request(&path, &headers), symbol).await
    }

    pub(super) async fn open_orders_for_category(
        &self,
        category: BitgetUtaCategory,
        symbol: Option<&str>,
    ) -> ExchangeResult<Vec<shared_types::OrderInfo>> {
        let mut path = format!(
            "{}?category={}",
            super::OPEN_ORDERS_PATH,
            category.as_query()
        );
        if let Some(symbol) = symbol {
            path.push_str("&symbol=");
            path.push_str(symbol);
        }
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::open_orders(&self.signed_request(&path, &headers)).await
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
        Ok([
            ("ACCESS-KEY".into(), creds.api_key.clone()),
            ("ACCESS-SIGN".into(), signature),
            ("ACCESS-TIMESTAMP".into(), timestamp),
            ("ACCESS-PASSPHRASE".into(), creds.passphrase.clone()),
            ("Content-Type".into(), "application/json".into()),
        ])
    }

    pub(super) fn ensure_write_adapter(&self) -> ExchangeResult<()> {
        if self.config.allow_live_writes || self.config.base_url_override.is_some() {
            Ok(())
        } else {
            Err(ExchangeError::Auth(
                "bitget live trading disabled; enable live writes explicitly".into(),
            ))
        }
    }

    pub(super) fn ws_trade_config(&self) -> ExchangeResult<WsTradeConfig<'_>> {
        let credentials = self.require_credentials()?;
        Ok(WsTradeConfig {
            url: bitget_uta_ws_user::BITGET_PRIVATE_WS_URL,
            api_key: &credentials.api_key,
            api_secret: &credentials.api_secret,
            passphrase: &credentials.passphrase,
            timeout_secs: self.config.timeout_secs,
        })
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

    pub async fn validate_safe_order_cancel_no_match_permission(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed; using cached offset");
        }
        let client_order_id = format!("xline-cancel-probe-{}", now_ms());
        let request = CancelOrderRequest {
            exchange: NAME.to_owned(),
            symbol: SAFE_ORDER_CANCEL_SYMBOL.to_owned(),
            internal_order_id: client_order_id.clone(),
            exchange_order_id: None,
            client_order_id,
        };
        let body = cancel_order_body_json_for(
            &request,
            BitgetUtaCategory::UsdtFutures,
            format!("{SAFE_ORDER_CANCEL_SYMBOL}USDT"),
        )?;
        let path = "/api/v3/trade/cancel-order";
        let headers = self.build_signed_headers("POST", path, &body)?;
        private_rest::safe_cancel_probe(&self.signed_request(path, &headers), body).await
    }

    pub async fn validate_api_order_permission_status(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "bitget time sync failed; using cached offset");
        }
        let path = "/api/v3/account/info";
        let headers = self.build_signed_headers("GET", path, "")?;
        private_rest::validate_order_permissions(&self.signed_request(path, &headers)).await
    }
}
