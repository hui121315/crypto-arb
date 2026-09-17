//! Gate adapter construction, signing and cache support helpers.

use super::{
    gate_ws_trade, Gate, GateConfig, GateCredentials, FUTURES_ORDERS_PATH, NAME,
    TIME_SYNC_INTERVAL_MS,
};
use crate::adapters::gate_config::{PROD_BASE, TESTNET_BASE};
use crate::adapters::gate_contracts::GateContractCache;
use crate::adapters::gate_fee_evidence::GateFuturesFeeCache;
use crate::adapters::gate_fill_evidence::GateFuturesFillEvidence;
use crate::adapters::gate_private_data::OpenOrderItem;
use crate::adapters::gate_private_rest as private_rest;
use crate::adapters::gate_public_rest as public_rest;
use crate::adapters::gate_trade_data::{cancel_order_params, gate_text, place_order_params};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::signing::gate as sign;
use crate::venue_spec::VenueId;
use common::time::{now_ms, now_secs};
use serde_json::Value;
use shared_types::{CancelOrderRequest, OrderInfo, OrderIntent};
use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

impl Gate {
    pub fn new(config: GateConfig) -> ExchangeResult<Self> {
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
            VenueId::Gate.defaults().qps,
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
            contract_cache: GateContractCache::default(),
            fee_cache: GateFuturesFeeCache::default(),
            time_offset_secs: AtomicI64::new(0),
            time_synced_at_ms: AtomicI64::new(0),
            market_stream: OnceLock::new(),
        })
    }

    pub(super) fn require_credentials(&self) -> ExchangeResult<&GateCredentials> {
        self.config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("gate: missing credentials".into()))
    }

    pub(super) async fn sync_server_time(&self) -> ExchangeResult<()> {
        let last = self.time_synced_at_ms.load(Ordering::Relaxed);
        let now = now_ms();
        if last != 0 && now.saturating_sub(last) < TIME_SYNC_INTERVAL_MS {
            return Ok(());
        }
        let local_before_ms = now_ms();
        let server_time = public_rest::server_time(&self.http, &self.base_url).await?;
        let local_after_ms = now_ms();
        let local_at_server_ms = (local_before_ms + local_after_ms) / 2;
        let offset_secs = (server_time - local_at_server_ms) / 1000;
        self.time_offset_secs.store(offset_secs, Ordering::Relaxed);
        self.time_synced_at_ms.store(now_ms(), Ordering::Relaxed);
        tracing::debug!(offset_secs, "gate server time synced");
        Ok(())
    }

    pub(super) fn build_signed_headers(
        &self,
        method: &str,
        url_path: &str,
        query: &str,
        body: &str,
    ) -> ExchangeResult<private_rest::SignedHeaders> {
        let creds = self.require_credentials()?;
        let offset = self.time_offset_secs.load(Ordering::Relaxed);
        let timestamp = now_secs().saturating_add(offset).to_string();
        let signature = sign::sign(
            creds.api_secret.as_bytes(),
            method,
            url_path,
            query,
            body,
            &timestamp,
        );
        Ok([
            ("KEY".into(), creds.api_key.clone()),
            ("Timestamp".into(), timestamp),
            ("SIGN".into(), signature),
        ])
    }

    pub(super) fn signed_request<'a>(
        &'a self,
        path: &'a str,
        query: &'a str,
        headers: &'a private_rest::SignedHeaders,
    ) -> private_rest::SignedRequest<'a> {
        private_rest::SignedRequest {
            http: &self.http,
            base_url: &self.base_url,
            path,
            query,
            headers,
        }
    }

    pub(super) async fn contract_order_unit(&self, symbol: &str) -> ExchangeResult<f64> {
        self.contract_cache
            .order_unit(&self.http, &self.base_url, symbol)
            .await
    }

    pub(super) async fn contract_market_unit(&self, symbol: &str) -> ExchangeResult<f64> {
        self.contract_cache
            .market_unit(&self.http, &self.base_url, symbol)
            .await
    }

    pub(super) async fn executable_native_symbol(&self, symbol: &str) -> ExchangeResult<String> {
        self.contract_cache
            .verified_native_symbol(&self.http, &self.base_url, symbol)
            .await
    }

    pub(super) async fn refresh_contract_cache(&self) -> ExchangeResult<()> {
        self.contract_cache
            .refresh_all(&self.http, &self.base_url)
            .await
    }

    pub(super) async fn place_order_params(&self, intent: &OrderIntent) -> ExchangeResult<Value> {
        let symbol = self.executable_native_symbol(&intent.symbol).await?;
        self.ensure_account_fee_evidence(&symbol).await?;
        let unit = self.contract_order_unit(&symbol).await?;
        place_order_params(intent, symbol, unit)
    }

    pub(super) async fn ensure_account_fee_evidence(
        &self,
        native_symbol: &str,
    ) -> ExchangeResult<()> {
        let now = now_ms();
        if self.fee_cache.fresh("USDT", native_symbol, now).is_some() {
            return Ok(());
        }
        let path = "/api/v4/futures/usdt/fee";
        let headers = self.build_signed_headers("GET", path, "", "")?;
        let rows = private_rest::futures_fee_evidence(
            &self.signed_request(path, "", &headers),
            "usdt",
            now,
        )
        .await?;
        self.fee_cache.replace(rows);
        self.fee_cache
            .fresh("USDT", native_symbol, now_ms())
            .map(|_| ())
            .ok_or_else(|| {
                ExchangeError::Parse(format!(
                    "gate account fee response missing contract {native_symbol}"
                ))
            })
    }

    pub(super) fn cancel_order_params(
        &self,
        request: &CancelOrderRequest,
    ) -> ExchangeResult<Value> {
        cancel_order_params(request)
    }

    pub(super) async fn fetch_open_order_rows(
        &self,
        symbol: Option<&str>,
    ) -> ExchangeResult<Vec<OpenOrderItem>> {
        let path = FUTURES_ORDERS_PATH;
        let mut query = String::from("status=open");
        if let Some(s) = symbol {
            query.push_str("&contract=");
            query.push_str(s);
        }
        let headers = self.build_signed_headers("GET", path, &query, "")?;
        private_rest::open_order_rows(&self.signed_request(path, &query, &headers)).await
    }

    pub(super) async fn fetch_open_order_rows_ws_first(
        &self,
        symbol: Option<&str>,
    ) -> ExchangeResult<Vec<OpenOrderItem>> {
        if let Some(rows) = self.try_fetch_open_order_rows_ws(symbol).await? {
            return Ok(rows);
        }
        self.fetch_open_order_rows(symbol).await
    }

    async fn try_fetch_open_order_rows_ws(
        &self,
        symbol: Option<&str>,
    ) -> ExchangeResult<Option<Vec<OpenOrderItem>>> {
        if self.config.base_url_override.is_some() {
            return Ok(None);
        }
        self.sync_ws_query_time("order-list").await;
        match gate_ws_trade::get_open_orders(self.ws_trade_config()?, symbol).await {
            Ok(rows) => Ok(Some(rows)),
            Err(error) => {
                tracing::warn!(%error, "gate futures.order_list failed; falling back to REST");
                Ok(None)
            }
        }
    }

    pub(super) async fn fetch_order_row(
        &self,
        order_id: &str,
    ) -> ExchangeResult<Option<OpenOrderItem>> {
        let path = format!("{FUTURES_ORDERS_PATH}/{order_id}");
        let headers = self.build_signed_headers("GET", &path, "", "")?;
        private_rest::order_row(&self.signed_request(&path, "", &headers)).await
    }

    pub(super) async fn fetch_order_row_ws_first(
        &self,
        order_id: &str,
    ) -> ExchangeResult<Option<OpenOrderItem>> {
        if let Some(row) = self.try_fetch_order_row_ws(order_id).await? {
            return Ok(Some(row));
        }
        self.fetch_order_row(order_id).await
    }

    async fn try_fetch_order_row_ws(
        &self,
        order_id: &str,
    ) -> ExchangeResult<Option<OpenOrderItem>> {
        if self.config.base_url_override.is_some() {
            return Ok(None);
        }
        self.sync_ws_query_time("order-status").await;
        match gate_ws_trade::get_order(self.ws_trade_config()?, order_id).await {
            Ok(row) => Ok(Some(row)),
            Err(error) => {
                tracing::warn!(%error, order_id, "gate futures.order_status failed; falling back to REST");
                Ok(None)
            }
        }
    }

    async fn sync_ws_query_time(&self, operation: &'static str) {
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, operation, "gate WS query time sync failed; using cached offset");
        }
    }

    pub(super) async fn fetch_order_row_by_exchange_id(
        &self,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OpenOrderItem>> {
        let exchange_order_id = validated_exchange_order_id(exchange_order_id)?;
        self.fetch_order_row_ws_first(&exchange_order_id).await
    }

    async fn fetch_order_fills(
        &self,
        exchange_order_id: &str,
    ) -> ExchangeResult<Vec<GateFuturesFillEvidence>> {
        let exchange_order_id = validated_exchange_order_id(exchange_order_id)?;
        let path = "/api/v4/futures/usdt/my_trades";
        let query = private_rest::my_trades_query(&exchange_order_id)?;
        let headers = self.build_signed_headers("GET", path, &query, "")?;
        private_rest::my_trades(
            &self.signed_request(path, &query, &headers),
            "usdt",
            &exchange_order_id,
        )
        .await
    }

    pub(super) async fn enrich_order_with_fill_evidence(
        &self,
        order: &mut OrderInfo,
    ) -> ExchangeResult<()> {
        if order.filled_quantity <= 0.0 {
            return Ok(());
        }
        let fills = self.fetch_order_fills(&order.order_id).await?;
        if fills.is_empty() {
            return Err(ExchangeError::Parse(format!(
                "gate order {} reports fills without my_trades evidence",
                order.order_id
            )));
        }
        let mut ids = HashSet::with_capacity(fills.len());
        let mut quantity = 0.0;
        let mut notional = 0.0;
        let mut fees = 0.0;
        for fill in fills {
            if !ids.insert(fill.trade_id.clone()) {
                return Err(ExchangeError::Parse(format!(
                    "gate order {} repeats my_trades id {}",
                    order.order_id, fill.trade_id
                )));
            }
            let fill_quantity = fill.size.abs();
            quantity += fill_quantity;
            notional += fill_quantity * fill.price;
            fees += fill.fee;
            if fill.point_fee != 0.0 {
                tracing::warn!(
                    order_id = %order.order_id,
                    trade_id = %fill.trade_id,
                    point_fee = fill.point_fee,
                    "gate point fee preserved in venue evidence but excluded from settlement fee"
                );
            }
        }
        if quantity <= 0.0 || !quantity.is_finite() || !notional.is_finite() || !fees.is_finite() {
            return Err(ExchangeError::Parse(format!(
                "gate order {} has invalid my_trades aggregate",
                order.order_id
            )));
        }
        order.filled_price = notional / quantity;
        order.fees = fees;
        Ok(())
    }

    pub async fn validate_safe_order_cancel_no_match_permission(&self) -> ExchangeResult<()> {
        self.require_credentials()?;
        if let Err(error) = self.sync_server_time().await {
            tracing::warn!(%error, "gate time sync failed; using cached offset");
        }
        let text = gate_text(&format!("xline-gate-probe-{}", now_ms()))?;
        let path = format!("{FUTURES_ORDERS_PATH}/{text}");
        let headers = self.build_signed_headers("DELETE", &path, "", "")?;
        private_rest::safe_cancel_probe(&self.signed_request(&path, "", &headers)).await
    }
}

fn validated_exchange_order_id(exchange_order_id: &str) -> ExchangeResult<String> {
    let parsed = exchange_order_id.parse::<u64>().map_err(|_| {
        ExchangeError::Parse(format!(
            "gate exchange order id must be a positive integer: {exchange_order_id}"
        ))
    })?;
    if parsed == 0 {
        return Err(ExchangeError::Parse(
            "gate exchange order id must be greater than zero".into(),
        ));
    }
    Ok(parsed.to_string())
}
