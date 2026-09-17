//! Kraken Spot WebSocket v2 and Derivatives WebSocket adapter.

use super::kraken_config::{FUTURES_REST_URL, SPOT_REST_URL};
use super::kraken_futures_data::parse_instruments;
use super::kraken_futures_ws::KrakenFuturesPublicStream;
use super::kraken_spot_data::parse_asset_pairs;
#[cfg(test)]
use super::kraken_spot_data::spot_tick_to_ticker;
use super::kraken_spot_ws::KrakenSpotPublicStream;
use super::kraken_symbols::{canonical_symbol, futures_symbol, spot_symbol};
use crate::adapter::{
    checked_text_with_evidence, ExchangeAdapter, MetadataRefreshOutcome, PublicWsSnapshot,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::venue_spec::VenueId;
use arc_swap::ArcSwap;
use async_trait::async_trait;
use reqwest::Method;
use rust_decimal::Decimal;
use shared_types::{
    FundingRateData, MarkIndexInfo, OrderBookInfo, SpotTick, TickerInfo, VenueInstrument,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

pub use super::kraken_config::{
    KrakenConfig, KrakenCredentials, KrakenFuturesCredentials, KrakenSpotCredentials,
};
pub use super::kraken_spot_private_data::{KrakenSpotExecution, KrakenSpotFill};

const NAME: &str = "kraken";
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);
const INSTRUMENT_WAIT: Duration = Duration::from_secs(3);
const FUTURES_INSTRUMENT_TIMEOUT: Duration = Duration::from_secs(8);
const SPOT_INSTRUMENT_TIMEOUT: Duration = Duration::from_secs(20);
const WAIT_POLL: Duration = Duration::from_millis(50);
const SPOT_ASSET_PAIRS_PATH: &str = "/0/public/AssetPairs?assetVersion=1";
const SPOT_STOCK_PAIRS_PATH: &str =
    "/0/public/AssetPairs?assetVersion=1&aclass_base=tokenized_asset";

type InstrumentBranchResult =
    Result<ExchangeResult<Vec<VenueInstrument>>, tokio::time::error::Elapsed>;

#[derive(Debug)]
pub struct Kraken {
    pub(super) config: KrakenConfig,
    pub(super) spot_base_url: String,
    pub(super) futures_base_url: String,
    pub(super) http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    spot_stream: Arc<KrakenSpotPublicStream>,
    spot_exact_stream: Arc<KrakenSpotPublicStream>,
    futures_stream: Arc<KrakenFuturesPublicStream>,
    instruments: Arc<ArcSwap<Vec<VenueInstrument>>>,
    pub(super) spot_private_stream:
        std::sync::OnceLock<Arc<super::kraken_spot_ws_private::KrakenSpotPrivateStream>>,
    pub(super) futures_private_stream:
        std::sync::OnceLock<Arc<super::kraken_futures_ws_private::KrakenFuturesPrivateStream>>,
}

impl Kraken {
    pub fn new(config: KrakenConfig) -> ExchangeResult<Self> {
        let spot_base_url = config
            .spot_rest_url_override
            .clone()
            .unwrap_or_else(|| SPOT_REST_URL.to_owned());
        let futures_base_url = config
            .futures_rest_url_override
            .clone()
            .unwrap_or_else(|| FUTURES_REST_URL.to_owned());
        let rate_limiter = Arc::new(RateLimiter::with_shared_budget(
            NAME,
            config.qps,
            NAME,
            VenueId::Kraken.defaults().qps,
        ));
        let http = HttpClient::builder(NAME)
            .timeout_secs(config.timeout_secs)
            .rate_limiter(Arc::clone(&rate_limiter))
            .build()?;
        let spot_stream = KrakenSpotPublicStream::shared(&config);
        let spot_exact_stream = KrakenSpotPublicStream::exact(&config);
        let futures_stream = KrakenFuturesPublicStream::shared(&config);
        Ok(Self {
            config,
            spot_base_url,
            futures_base_url,
            http,
            _rate_limiter: rate_limiter,
            spot_stream,
            spot_exact_stream,
            futures_stream,
            instruments: Arc::new(ArcSwap::from_pointee(Vec::new())),
            spot_private_stream: std::sync::OnceLock::new(),
            futures_private_stream: std::sync::OnceLock::new(),
        })
    }

    async fn refresh_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        // Keep Kraken's official Spot instrument WS alive independently from
        // ticker/book demand so the registry receives the complete Spot universe.
        self.spot_stream.activate_instrument_stream();
        let (futures, spot) = self.fetch_instrument_branches().await;
        let mut rows = Vec::new();
        let mut failures = Vec::new();
        collect_instrument_branch(
            "futures",
            FUTURES_INSTRUMENT_TIMEOUT,
            futures,
            &mut rows,
            &mut failures,
        );
        collect_instrument_branch(
            "spot",
            SPOT_INSTRUMENT_TIMEOUT,
            spot,
            &mut rows,
            &mut failures,
        );
        self.commit_instrument_refresh(rows, &failures)
    }

    async fn fetch_instrument_branches(&self) -> (InstrumentBranchResult, InstrumentBranchResult) {
        tokio::join!(
            tokio::time::timeout(FUTURES_INSTRUMENT_TIMEOUT, self.fetch_futures_instruments()),
            tokio::time::timeout(SPOT_INSTRUMENT_TIMEOUT, self.fetch_spot_instruments())
        )
    }

    fn commit_instrument_refresh(
        &self,
        rows: Vec<VenueInstrument>,
        failures: &[String],
    ) -> ExchangeResult<Vec<VenueInstrument>> {
        if rows.is_empty() {
            return self.retained_instruments_or_error(failures);
        }
        if !failures.is_empty() {
            warn!(venue = NAME, failures = ?failures, rows = rows.len(), "kraken instrument refresh partially recovered");
        }
        self.instruments.store(Arc::new(rows.clone()));
        Ok(rows)
    }

    fn retained_instruments_or_error(
        &self,
        failures: &[String],
    ) -> ExchangeResult<Vec<VenueInstrument>> {
        let cached = self.instruments.load_full();
        if cached.is_empty() {
            return Err(ExchangeError::Network(format!(
                "kraken instrument refresh produced no rows: {}",
                failures.join("; ")
            )));
        }
        warn!(venue = NAME, failures = ?failures, rows = cached.len(), "kraken instrument refresh retained the last verified snapshot");
        Ok((*cached).clone())
    }

    async fn fetch_futures_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let url = format!("{}/derivatives/api/v3/instruments", self.futures_base_url);
        let response = self
            .http
            .execute_with_retry(|| self.http.request(Method::GET, &url))
            .await?;
        let (body, _) = checked_text_with_evidence(response, url).await?;
        parse_instruments(&body)
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        self.spot_stream.activate_instrument_stream();
        let deadline = Instant::now() + INSTRUMENT_WAIT;
        let mut spot = self.spot_stream.instruments();
        while spot.is_empty() && Instant::now() < deadline {
            tokio::time::sleep(WAIT_POLL).await;
            spot = self.spot_stream.instruments();
        }
        let rows = if spot.is_empty() {
            self.fetch_spot_instruments_cold_start().await?
        } else {
            spot.iter().cloned().collect::<Vec<_>>()
        };
        // A Spot-only caller must retain official wire names even before the WS snapshot arrives.
        self.instruments.rcu(|old| {
            Arc::new(
                old.iter()
                    .filter(|r| r.product_type.as_deref() != Some("spot"))
                    .cloned()
                    .chain(rows.iter().cloned())
                    .collect(),
            )
        });
        Ok(rows)
    }

    async fn fetch_spot_instruments_cold_start(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let mut rows = Vec::new();
        let mut failures = Vec::new();
        for path in [SPOT_ASSET_PAIRS_PATH, SPOT_STOCK_PAIRS_PATH] {
            match self.fetch_spot_asset_class(path).await {
                Ok(mut result) => rows.append(&mut result),
                Err(error) => failures.push(format!("{path}: {error}")),
            }
        }
        if rows.is_empty() {
            return Err(ExchangeError::Network(format!(
                "kraken spot metadata unavailable: {}",
                failures.join("; ")
            )));
        }
        if !failures.is_empty() {
            warn!(?failures, "kraken spot cold-start asset class incomplete");
        }
        Ok(rows
            .into_iter()
            .map(|row| (row.native_symbol.clone(), row))
            .collect::<std::collections::BTreeMap<_, _>>()
            .into_values()
            .collect())
    }

    async fn fetch_spot_asset_class(&self, path: &str) -> ExchangeResult<Vec<VenueInstrument>> {
        let url = format!("{}{path}", self.spot_base_url);
        let response = self
            .http
            .execute_with_retry(|| self.http.request(Method::GET, &url))
            .await?;
        let (body, _) = checked_text_with_evidence(response, url).await?;
        parse_asset_pairs(&body)
    }

    async fn futures_product_ids(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<String>> {
        if let Some(symbols) = symbols {
            return Ok(symbols
                .iter()
                .map(|symbol| futures_symbol(symbol))
                .collect());
        }
        if self.instruments.load().is_empty() {
            self.refresh_instruments().await?;
        }
        Ok(self
            .instruments
            .load()
            .iter()
            .filter(|row| row.execution_supported && row.product_type.as_deref() == Some("perp"))
            .map(|row| row.native_symbol.clone())
            .collect())
    }

    fn spot_product_ids(&self, symbols: Option<&[String]>) -> Vec<String> {
        match symbols {
            Some(symbols) => {
                let live = self.spot_stream.instruments();
                let cached = self.instruments.load_full();
                resolve_spot_ids(symbols, &cached, &live)
            }
            None => self
                .instruments
                .load()
                .iter()
                .filter(|row| row.product_type.as_deref() == Some("spot"))
                .map(|row| row.native_symbol.clone())
                .collect(),
        }
    }

    fn native_spot_symbol(&self, requested: &str) -> String {
        let normalized = spot_symbol(requested);
        let live = self.spot_stream.instruments();
        let cached = self.instruments.load_full();
        live.iter()
            .chain(cached.iter())
            .find(|row| {
                row.product_type.as_deref() == Some("spot")
                    && spot_symbol(&row.native_symbol) == normalized
            })
            .map(|row| row.native_symbol.clone())
            .unwrap_or(normalized)
    }

    async fn await_ticker(&self, product_id: &str) -> ExchangeResult<TickerInfo> {
        self.futures_stream.touch_tickers(&[product_id.to_owned()]);
        let deadline = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            if let Some(row) = self.futures_stream.latest_ticker(product_id) {
                return Ok(row);
            }
            if Instant::now() >= deadline {
                return Err(ExchangeError::WsClosed(format!(
                    "kraken futures ticker awaiting first frame for {product_id}"
                )));
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }

    async fn await_funding(&self, product_id: &str) -> ExchangeResult<FundingRateData> {
        self.await_ticker(product_id).await?;
        self.futures_stream
            .latest_funding(product_id)
            .ok_or_else(|| {
                ExchangeError::Parse(format!("kraken funding is unavailable for {product_id}"))
            })
    }

    async fn await_futures_book(
        &self,
        product_id: &str,
        depth: u32,
    ) -> ExchangeResult<OrderBookInfo> {
        self.futures_stream.touch_book(product_id);
        let multiplier = self.contract_multiplier(product_id);
        let deadline = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            if let Some(row) = self.futures_stream.latest_book(
                product_id,
                usize::try_from(depth.max(1)).unwrap_or(1),
                multiplier,
            ) {
                return Ok(row);
            }
            if Instant::now() >= deadline {
                return Err(ExchangeError::WsClosed(format!(
                    "kraken futures book awaiting snapshot for {product_id}"
                )));
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }

    fn contract_multiplier(&self, product_id: &str) -> Decimal {
        self.instruments
            .load()
            .iter()
            .find(|row| row.native_symbol == product_id)
            .and_then(|row| row.contract_size)
            .and_then(Decimal::from_f64_retain)
            .unwrap_or(Decimal::ONE)
    }
}

fn collect_instrument_branch(
    branch: &'static str,
    timeout: Duration,
    result: Result<ExchangeResult<Vec<VenueInstrument>>, tokio::time::error::Elapsed>,
    rows: &mut Vec<VenueInstrument>,
    failures: &mut Vec<String>,
) {
    match result {
        Ok(Ok(branch_rows)) => rows.extend(branch_rows),
        Ok(Err(error)) => failures.push(format!("{branch}: {error}")),
        Err(_) => failures.push(format!("{branch}: timeout after {}s", timeout.as_secs())),
    }
}

#[async_trait]
impl ExchangeAdapter for Kraken {
    async fn prepare_stock_submission(&self) -> ExchangeResult<()> {
        Kraken::prepare_stock_submission(self).await
    }
    async fn warm_stock_receipts(&self) -> ExchangeResult<()> { self.spot_private()?.warm().await }
    async fn submit_stock_order(&self, draft: shared_types::stocks::StockPeerOrderDraft, client: String) -> ExchangeResult<shared_types::stocks::StockPeerOrderReceipt> {
        Kraken::submit_stock_order(self, draft, client).await
    }
    fn track_stock_order(&self, receipt: shared_types::stocks::StockPeerOrderReceipt) -> ExchangeResult<()> {
        Kraken::track_stock_order(self, receipt)
    }
    fn stock_order_receipt(&self, client: &str) -> Option<shared_types::stocks::StockPeerOrderReceipt> {
        Kraken::stock_order_receipt(self, client)
    }
    async fn reconcile_stock_order(&self, original: &shared_types::stocks::StockPeerOrderReceipt) -> ExchangeResult<Option<shared_types::stocks::StockPeerOrderReceipt>> {
        self.read_stock_order_history(original).await
    }
    fn subscribe_stock_receipts(&self) -> ExchangeResult<tokio::sync::broadcast::Receiver<shared_types::stocks::StockPeerOrderReceipt>> {
        Kraken::subscribe_stock_receipts(self)
    }
    fn stock_account_fingerprint(&self) -> Option<String> {
        let keys = self.config.credentials.as_ref()?.spot.as_ref()?;
        if keys.api_key.is_empty() || keys.api_secret.is_empty() { return None; }
        Some(crate::ws::trade_session::session_key(&["kraken-stock-account", &self.spot_base_url, &keys.api_key, &keys.api_secret]))
    }

    async fn stock_cash_account(&self, native_symbol: &str) -> ExchangeResult<shared_types::stocks::StockPeerAccount> {
        self.read_stock_cash_account(native_symbol).await
    }

    async fn stock_funding_methods(&self, native_symbol: &str) -> ExchangeResult<Vec<shared_types::stocks::StockPeerFundingRoute>> {
        self.read_stock_funding_methods(native_symbol).await
    }

    async fn validate_stock_order(&self, draft: &shared_types::stocks::StockPeerOrderDraft) -> ExchangeResult<shared_types::stocks::StockPeerOrderCheck> {
        draft.kraken_validation("local-check",0,common::time::now_ms()).map_err(|e|ExchangeError::Parse(e.into()))?;
        self.spot_private()?.validate_stock_order(draft).await
    }
    fn name(&self) -> &'static str {
        NAME
    }

    async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
        self.refresh_instruments().await?;
        Ok(MetadataRefreshOutcome::Refreshed)
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        self.await_funding(&futures_symbol(symbol)).await
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        let ids = self.futures_product_ids(symbols).await?;
        self.futures_stream.touch_tickers(&ids);
        Ok(self.futures_stream.funding_snapshot(&ids))
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        self.await_ticker(&futures_symbol(symbol)).await
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        let ids = self.futures_product_ids(symbols).await?;
        self.futures_stream.touch_tickers(&ids);
        Ok(self.futures_stream.ticker_snapshot(&ids))
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        let ids = self.futures_product_ids(Some(symbols)).await?;
        self.futures_stream.touch_tickers(&ids);
        let rows = self.futures_stream.ticker_snapshot(&ids);
        Ok(if rows.is_empty() {
            PublicWsSnapshot::Pending
        } else {
            PublicWsSnapshot::Ready(rows)
        })
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        let ids = self.futures_product_ids(Some(symbols)).await?;
        self.futures_stream.touch_tickers(&ids);
        let rows = self.futures_stream.funding_snapshot(&ids);
        Ok(if rows.is_empty() {
            PublicWsSnapshot::Pending
        } else {
            PublicWsSnapshot::Ready(rows)
        })
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        let ids = self.futures_product_ids(Some(symbols)).await?;
        self.futures_stream.touch_tickers(&ids);
        let rows = self.futures_stream.mark_index_snapshot(&ids);
        Ok(if rows.is_empty() {
            PublicWsSnapshot::Pending
        } else {
            PublicWsSnapshot::Ready(rows)
        })
    }

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        let ids = self.futures_product_ids(symbols).await?;
        self.futures_stream.touch_tickers(&ids);
        Ok(self.futures_stream.mark_index_snapshot(&ids))
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        let ids = self.spot_product_ids(Some(symbols));
        if spot_snapshot_uses_discovery_trigger(ids.len()) {
            self.spot_stream.touch_discovery_tickers(&ids);
            let rows = self.spot_stream.ticker_snapshot(&ids);
            return Ok(if rows.is_empty() {
                PublicWsSnapshot::Pending
            } else {
                PublicWsSnapshot::Ready(rows)
            });
        }
        let rows = self.exact_spot_snapshot(&ids);
        Ok(if rows.is_empty() {
            PublicWsSnapshot::Pending
        } else {
            PublicWsSnapshot::Ready(rows)
        })
    }

    fn public_ws_spot_problem(&self, symbol: &str) -> Option<String> {
        let native = self.native_spot_symbol(symbol);
        if !self
            .spot_exact_stream
            .ticker_snapshot(std::slice::from_ref(&native))
            .is_empty()
            || !self
                .spot_stream
                .ticker_snapshot(std::slice::from_ref(&native))
                .is_empty()
        {
            return None;
        }
        self.spot_exact_stream
            .connection_problem()
            .map(|problem| format!("Kraken Spot WS 连接失败：{problem}"))
            .or_else(|| self.spot_stream.connection_problem())
            .or_else(|| self.spot_exact_stream.ticker_problem(&native))
            .or_else(|| self.spot_stream.ticker_problem(&native))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        if self.instruments.load().is_empty() {
            self.refresh_instruments().await?;
        }
        let ids = self.spot_product_ids(symbols);
        if symbols.is_some() {
            Ok(self.exact_spot_snapshot(&ids))
        } else {
            self.spot_stream.touch_discovery_tickers(&ids);
            Ok(self.spot_stream.ticker_snapshot(&ids))
        }
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        self.await_futures_book(&futures_symbol(symbol), depth)
            .await
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let product_id = futures_symbol(symbol);
        self.futures_stream.touch_book(&product_id);
        let row = self.futures_stream.latest_book(
            &product_id,
            usize::try_from(depth.max(1)).unwrap_or(1),
            self.contract_multiplier(&product_id),
        );
        Ok(row
            .map(|row| PublicWsSnapshot::Ready(vec![row]))
            .unwrap_or(PublicWsSnapshot::Pending))
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let native = self.native_spot_symbol(symbol);
        self.spot_exact_stream
            .touch_book(&native, usize::try_from(depth.max(1)).unwrap_or(1));
        let deadline = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            if let Some(row) = self
                .spot_exact_stream
                .latest_book(&native, usize::try_from(depth.max(1)).unwrap_or(1))
            {
                return Ok(row);
            }
            if let Some(problem) = self.spot_exact_stream.book_problem(&native) {
                return Err(ExchangeError::WsClosed(problem));
            }
            if Instant::now() >= deadline {
                return Err(ExchangeError::WsClosed(format!(
                    "kraken spot book awaiting snapshot for {native}"
                )));
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let native = self.native_spot_symbol(symbol);
        let depth = usize::try_from(depth.max(1)).unwrap_or(1);
        self.spot_exact_stream.touch_book(&native, depth);
        if let Some(problem) = self.spot_exact_stream.book_problem(&native) {
            return Err(ExchangeError::WsClosed(problem));
        }
        Ok(self
            .spot_exact_stream
            .latest_book(&native, depth)
            .map(|row| PublicWsSnapshot::Ready(vec![row]))
            .unwrap_or(PublicWsSnapshot::Pending))
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        self.refresh_instruments().await
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        Kraken::fetch_spot_instruments(self).await
    }

    async fn fetch_transfer_networks_for(
        &self,
        currencies: &[String],
    ) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        let credentials = self
            .config
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.spot.as_ref())
            .ok_or_else(|| {
                ExchangeError::Auth(
                    "kraken transfer methods require Spot API credentials with Funds Query permission"
                        .to_owned(),
                )
            })?;
        super::kraken_transfer_networks::fetch(
            &self.http,
            &self.spot_base_url,
            credentials,
            currencies,
        )
        .await
    }

    async fn fetch_transfer_destination(
        &self,
        request: &crate::TransferDestinationRequest,
    ) -> ExchangeResult<crate::TransferDestinationEvidence> {
        let credentials = self
            .config
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.spot.as_ref())
            .ok_or_else(|| {
                ExchangeError::Auth("Kraken Spot Funds Query credentials missing".into())
            })?;
        match request.direction {
            crate::TransferDirection::DepositToVenue => {
                super::kraken_deposits::destination(
                    &self.http,
                    &self.spot_base_url,
                    credentials,
                    request,
                )
                .await
            }
            crate::TransferDirection::WithdrawToChain => {
                super::kraken_withdrawals::destination(
                    &self.http,
                    &self.spot_base_url,
                    credentials,
                    request,
                )
                .await
            }
        }
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        canonical_symbol(symbol)
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        futures_symbol(symbol)
    }
}

impl Kraken {
    fn exact_spot_snapshot(&self, ids: &[String]) -> Vec<SpotTick> {
        // Keep the isolated BBO stream as the latency-first source. Reuse any
        // fresher row already present on the full-market socket without adding
        // a duplicate BBO subscription to that connection.
        self.spot_exact_stream.touch_tickers(ids);
        let exact = self.spot_exact_stream.ticker_snapshot(ids);
        let shared = self.spot_stream.ticker_snapshot(ids);
        merge_exact_spot_rows(ids, &exact, &shared)
    }
}

fn merge_exact_spot_rows(ids: &[String], exact: &[SpotTick], shared: &[SpotTick]) -> Vec<SpotTick> {
    ids.iter()
        .filter_map(|symbol| {
            let exact = exact
                .iter()
                .find(|row| row.symbol.eq_ignore_ascii_case(symbol));
            let shared = shared
                .iter()
                .find(|row| row.symbol.eq_ignore_ascii_case(symbol));
            match (exact, shared) {
                (Some(exact), Some(shared)) if shared.received_at_ms > exact.received_at_ms => {
                    Some(shared.clone())
                }
                (Some(exact), _) => Some(exact.clone()),
                (None, Some(shared)) => Some(shared.clone()),
                (None, None) => None,
            }
        })
        .collect()
}

fn spot_snapshot_uses_discovery_trigger(symbol_count: usize) -> bool {
    symbol_count > 50
}

fn resolve_spot_ids(
    requests: &[String],
    cached: &[VenueInstrument],
    live: &[VenueInstrument],
) -> Vec<String> {
    // Build one lookup per batch, not a full registry scan for every market.
    let lookup = cached
        .iter()
        .chain(live.iter())
        .filter(|r| r.product_type.as_deref() == Some("spot"))
        .map(|r| (spot_symbol(&r.native_symbol), r.native_symbol.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    requests
        .iter()
        .map(|s| {
            let key = spot_symbol(s);
            lookup.get(&key).map(|s| (*s).to_owned()).unwrap_or(key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "public Kraken WS probe; no credentials or orders"]
    async fn kraken_stock_native_public_quotes() {
        let venue = Kraken::new(KrakenConfig::default()).unwrap();
        venue.spot_stream.activate_instrument_stream();
        let rows = tokio::time::timeout(Duration::from_secs(10), venue.fetch_spot_instruments())
            .await
            .unwrap()
            .unwrap();
        let instrument = rows
            .into_iter()
            .find(|i| i.native_symbol == "MUx/USD")
            .expect("official MUx/USD instrument");
        assert_eq!(
            instrument.asset_class,
            shared_types::InstrumentAssetClass::Equity
        );
        assert!(!instrument.execution_supported);
        let requests = vec!["MUX/USD".into(), "USDC/USD".into()];
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let PublicWsSnapshot::Ready(rows) =
                venue.public_ws_spot_snapshot(&requests).await.unwrap()
            {
                let stock = rows.iter().find(|r| r.symbol == "MUX/USD");
                let fx = rows.iter().find(|r| r.symbol == "USDC/USD");
                if let (Some(stock), Some(fx)) = (stock, fx) {
                    assert!(stock.bid > rust_decimal::Decimal::ZERO && stock.ask >= stock.bid);
                    assert!(stock.bid_size.is_some() && stock.ask_size.is_some());
                    let capture = serde_json::json!({"instrument":instrument,"stockTick":stock,"fxTick":fx,"capturedAtMs":common::time::now_ms()});
                    println!("{capture}");
                    if let Ok(path) = std::env::var("STOCK_PEER_CAPTURE_PATH") {
                        std::fs::write(path, serde_json::to_vec_pretty(&capture).unwrap()).unwrap();
                    }
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "Kraken native stock/FX WS did not return both quotes"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    #[test]
    fn public_symbol_boundary_keeps_spot_and_futures_native_forms() {
        assert_eq!(spot_symbol("BTC"), "BTC/USD");
        assert_eq!(futures_symbol("BTC"), "PF_XBTUSD");
        assert_eq!(canonical_symbol("PI_XBTUSD"), "BTC");
        let frame = super::super::kraken_spot_data::parse_instrument_frame(include_str!(
            "../../fixtures/kraken/spot_v2_instrument_muxusd.json"
        ))
        .unwrap();
        assert_eq!(
            resolve_spot_ids(&["MUX/USD".into(), "BTC/USD".into()], &[], &frame.rows),
            vec!["MUx/USD", "BTC/USD"]
        );
    }

    #[test]
    fn spot_tick_projection_uses_same_market_timestamp() {
        let rows = super::super::kraken_spot_data::parse_ticker_frame(include_str!(
            "../../fixtures/kraken/spot_v2_ticker_btcusd.json"
        ))
        .unwrap();
        let ticker = spot_tick_to_ticker(&rows[0]);
        assert_eq!(ticker.symbol, "BTC/USD");
        assert_eq!(ticker.timestamp, rows[0].best_timestamp_ms());
    }

    #[test]
    fn bulk_spot_snapshot_uses_lightweight_discovery_trigger() {
        assert!(!spot_snapshot_uses_discovery_trigger(1));
        assert!(!spot_snapshot_uses_discovery_trigger(50));
        assert!(spot_snapshot_uses_discovery_trigger(51));
        assert!(spot_snapshot_uses_discovery_trigger(1_392));
    }

    #[test]
    fn exact_spot_rows_take_priority_and_shared_ws_fills_gaps() {
        let mut rows = super::super::kraken_spot_data::parse_ticker_frame(include_str!(
            "../../fixtures/kraken/spot_v2_ticker_btcusd.json"
        ))
        .unwrap();
        let mut exact_pups = rows.remove(0);
        exact_pups.symbol = "PUPS/USD".to_owned();
        exact_pups.received_at_ms = 3;

        let mut shared_pups = exact_pups.clone();
        shared_pups.received_at_ms = 2;
        let mut shared_sol = exact_pups.clone();
        shared_sol.symbol = "SOL/USD".to_owned();
        shared_sol.received_at_ms = 4;
        let mut exact_sol = shared_sol.clone();
        exact_sol.received_at_ms = 1;

        let ids = vec!["PUPS/USD".to_owned(), "SOL/USD".to_owned()];
        let merged =
            merge_exact_spot_rows(&ids, &[exact_pups, exact_sol], &[shared_pups, shared_sol]);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].symbol, "PUPS/USD");
        assert_eq!(merged[0].received_at_ms, 3);
        assert_eq!(merged[1].symbol, "SOL/USD");
        assert_eq!(merged[1].received_at_ms, 4);
    }
}
