//! Gate `CrossEx` public market adapter with underlying-route identity preserved.

use super::gate_crossex_config::CROSSEX_REST_URL;
use super::gate_crossex_data::{parse_instruments, SYMBOLS_SOURCE_URL};
use super::gate_crossex_private_data::{
    compile_order, parse_account_text, parse_order_text, parse_orders_text, parse_positions_text,
};
use super::gate_crossex_rest::{
    fetch_account, fetch_funding_intervals, fetch_open_orders, fetch_order, fetch_positions,
};
use super::gate_crossex_symbols::{route_from_scoped_symbol, CrossExBusiness, CrossExRoute};
use super::gate_crossex_ws::GateCrossExPublicStream;
use super::gate_crossex_ws_private::GateCrossExPrivateStream;
use crate::adapter::{
    checked_text_with_evidence, ExchangeAdapter, MetadataRefreshOutcome, PublicWsSnapshot,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::{
    ExchangeCapabilities, LiveTradingAdapter, PrivateWsRuntimeStatus, VenueAccountRead,
};
use crate::services::RateLimiter;
use crate::venue_spec::VenueId;
use arc_swap::ArcSwap;
use async_trait::async_trait;
use reqwest::Method;
use rust_decimal::Decimal;
use shared_types::{
    CancelOrderRequest, FeeProduct, FundingRateData, GateCrossExRouteQuote, MarkIndexInfo,
    OrderAck, OrderBookInfo, OrderInfo, OrderIntent, OrderSubmissionContext, PositionInfo,
    SpotTick, TickerInfo, VenueAccountModeInfo, VenueBalanceInfo, VenueInstrument,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use super::gate_crossex_config::{GateCrossExConfig, GateCrossExCredentials};

const NAME: &str = "gate_crossex";
const DEFAULT_UNDERLYING: &str = "GATE";
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);
const WAIT_POLL: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub struct GateCrossEx {
    rest_base_url: String,
    credentials: Option<GateCrossExCredentials>,
    allow_live_writes: bool,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    stream: Arc<GateCrossExPublicStream>,
    private_stream: Option<Arc<GateCrossExPrivateStream>>,
    instruments: Arc<ArcSwap<Vec<VenueInstrument>>>,
    funding_intervals: Arc<ArcSwap<HashMap<String, u32>>>,
    depth_multipliers: Arc<ArcSwap<HashMap<String, Decimal>>>,
}

impl GateCrossEx {
    pub fn new(config: GateCrossExConfig) -> ExchangeResult<Self> {
        let rest_base_url = config
            .rest_url_override
            .clone()
            .unwrap_or_else(|| CROSSEX_REST_URL.to_owned());
        let limiter = Arc::new(RateLimiter::with_shared_budget(
            NAME,
            config.qps,
            NAME,
            VenueId::GateCrossEx.defaults().qps,
        ));
        let http = HttpClient::builder(NAME)
            .timeout_secs(config.timeout_secs)
            .rate_limiter(Arc::clone(&limiter))
            .build()?;
        let stream = GateCrossExPublicStream::shared(&config);
        let private_stream = config
            .credentials
            .as_ref()
            .map(|_| GateCrossExPrivateStream::shared(&config))
            .transpose()?;
        Ok(Self {
            rest_base_url,
            credentials: config.credentials,
            allow_live_writes: config.allow_live_writes,
            http,
            _rate_limiter: limiter,
            stream,
            private_stream,
            instruments: Arc::new(ArcSwap::from_pointee(Vec::new())),
            funding_intervals: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            depth_multipliers: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        })
    }

    async fn refresh_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let url = format!("{}/crossex/rule/symbols", self.rest_base_url);
        let response = self
            .http
            .execute_with_retry(|| self.http.request(Method::GET, &url))
            .await?;
        let (body, _) = checked_text_with_evidence(response, SYMBOLS_SOURCE_URL).await?;
        let rows = parse_instruments(&body)?;
        self.depth_multipliers
            .store(Arc::new(safe_depth_multipliers(&rows)));
        self.instruments.store(Arc::new(rows.clone()));
        Ok(rows)
    }

    async fn ensure_instruments(&self) -> ExchangeResult<()> {
        if self.instruments.load().is_empty() {
            self.refresh_instruments().await?;
        }
        Ok(())
    }

    async fn routes(
        &self,
        symbols: Option<&[String]>,
        business: CrossExBusiness,
        active_channel: &str,
    ) -> ExchangeResult<Vec<String>> {
        self.ensure_instruments().await?;
        let Some(symbols) = symbols else {
            return Ok(self
                .stream
                .subscribed_symbols(active_channel)
                .into_iter()
                .filter(|symbol| route_matches_business(symbol, business))
                .collect());
        };
        let instruments = self.instruments.load();
        let mut routes = Vec::new();
        for requested in symbols {
            let requested = requested.trim();
            let scoped = requested.split_once(':');
            for row in instruments.iter() {
                if row.product_type.as_deref() != Some(business.product_type()) {
                    continue;
                }
                let native_match = row.native_symbol.eq_ignore_ascii_case(requested);
                let canonical_match = row.canonical_symbol.eq_ignore_ascii_case(requested);
                let scoped_match = scoped.is_some_and(|(underlying, base)| {
                    row.venue
                        .rsplit_once(':')
                        .is_some_and(|(_, route)| route.eq_ignore_ascii_case(underlying))
                        && row.canonical_symbol.eq_ignore_ascii_case(base)
                });
                if native_match || canonical_match || scoped_match {
                    routes.push(row.native_symbol.clone());
                }
            }
        }
        routes.sort_unstable();
        routes.dedup();
        Ok(routes)
    }

    async fn exact_native_routes(&self, symbols: &[String]) -> ExchangeResult<Vec<String>> {
        self.ensure_instruments().await?;
        let instruments = self.instruments.load();
        let mut routes = Vec::with_capacity(symbols.len());
        for requested in symbols {
            let route = CrossExRoute::parse(requested)?;
            let known = instruments
                .iter()
                .any(|row| row.native_symbol.eq_ignore_ascii_case(&route.native_symbol));
            if !known {
                return Err(ExchangeError::UnsupportedSymbol(route.native_symbol));
            }
            routes.push(route.native_symbol);
        }
        routes.sort_unstable();
        routes.dedup();
        Ok(routes)
    }

    async fn single_route(
        &self,
        symbol: &str,
        business: CrossExBusiness,
    ) -> ExchangeResult<CrossExRoute> {
        let requested = [symbol.to_owned()];
        let routes = self.routes(Some(&requested), business, "ticker").await?;
        match routes.as_slice() {
            [route] => CrossExRoute::parse(route),
            [] => Err(ExchangeError::UnsupportedSymbol(symbol.to_owned())),
            _ => Err(ExchangeError::UnsupportedSymbol(format!(
                "ambiguous CrossEx symbol {symbol:?}; use underlying:base or native route"
            ))),
        }
    }

    async fn await_ticker(&self, route: &CrossExRoute) -> ExchangeResult<TickerInfo> {
        let symbols = [route.native_symbol.clone()];
        self.stream.touch_tickers(&symbols);
        let deadline = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            if let Some(row) = self.stream.ticker_snapshot(&symbols).into_iter().next() {
                return Ok(row);
            }
            if Instant::now() >= deadline {
                return Err(ExchangeError::WsClosed(format!(
                    "Gate CrossEx ticker awaiting first frame for {}",
                    route.native_symbol
                )));
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }

    async fn await_book(&self, route: &CrossExRoute, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let multiplier = self.depth_multiplier(route)?;
        if !self
            .stream
            .touch_book(route, usize::try_from(depth.max(1)).unwrap_or(1))
        {
            return Err(ExchangeError::UnsupportedCapability(
                "gate_crossex_orderbook_route",
            ));
        }
        let deadline = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            if let Some(row) = self.stream.latest_book(
                route,
                usize::try_from(depth.max(1)).unwrap_or(1),
                multiplier,
            ) {
                return Ok(row);
            }
            if Instant::now() >= deadline {
                return Err(ExchangeError::WsClosed(format!(
                    "Gate CrossEx book awaiting safe snapshot for {}",
                    route.native_symbol
                )));
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }

    fn depth_multiplier(&self, route: &CrossExRoute) -> ExchangeResult<Decimal> {
        self.depth_multipliers
            .load()
            .get(&route.native_symbol)
            .copied()
            .ok_or(ExchangeError::UnsupportedCapability(
                "gate_crossex_depth_quantity_unit_unverified",
            ))
    }

    fn credentials(&self) -> ExchangeResult<&GateCrossExCredentials> {
        self.credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("Gate CrossEx credentials missing".to_owned()))
    }

    fn require_funding_interval_metadata(&self) -> ExchangeResult<()> {
        if !self.funding_intervals.load().is_empty() {
            return Ok(());
        }
        if self.credentials.is_none() {
            return Err(ExchangeError::UnsupportedCapability(
                "Gate CrossEx funding interval metadata requires GATE_CROSSEX_API_KEY and \
                 GATE_CROSSEX_API_SECRET",
            ));
        }
        Err(ExchangeError::UnsupportedCapability(
            "gate_crossex_funding_interval_metadata_unavailable",
        ))
    }

    fn private_stream(&self) -> ExchangeResult<&Arc<GateCrossExPrivateStream>> {
        self.private_stream.as_ref().ok_or_else(|| {
            ExchangeError::Auth("Gate CrossEx private WS is not configured".to_owned())
        })
    }

    pub async fn warm_private_ws(&self) -> ExchangeResult<PrivateWsRuntimeStatus> {
        let private = self.private_stream()?;
        private.warm().await?;
        Ok(private.runtime_status())
    }

    pub fn private_ws_runtime_status(&self) -> ExchangeResult<PrivateWsRuntimeStatus> {
        self.private_stream()
            .map(|private| private.runtime_status())
    }

    fn ensure_live_writes(&self) -> ExchangeResult<()> {
        if self.allow_live_writes {
            self.credentials().map(|_| ())
        } else {
            Err(ExchangeError::UnsupportedCapability(
                "gate_crossex_live_writes_disabled",
            ))
        }
    }

    async fn trade_route(
        &self,
        exchange: &str,
        symbol: &str,
        product: FeeProduct,
    ) -> ExchangeResult<CrossExRoute> {
        let business = match product {
            FeeProduct::Spot => CrossExBusiness::Spot,
            FeeProduct::Perp => CrossExBusiness::Future,
            FeeProduct::Margin => CrossExBusiness::Margin,
            FeeProduct::Unknown => {
                return Err(ExchangeError::UnsupportedCapability(
                    "gate_crossex_order_product_unknown",
                ))
            }
        };
        if let Ok(route) = CrossExRoute::parse(symbol) {
            if route.business != business {
                return Err(ExchangeError::UnsupportedSymbol(format!(
                    "CrossEx route {} does not match {product:?}",
                    route.native_symbol
                )));
            }
            return self.single_route(&route.native_symbol, business).await;
        }
        let underlying = exchange
            .strip_prefix("gate_crossex:")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ExchangeError::UnsupportedSymbol(format!(
                    "CrossEx write requires gate_crossex:<underlying> for symbol {symbol}"
                ))
            })?;
        let route =
            route_from_scoped_symbol(&format!("{underlying}:{symbol}"), business, underlying)?;
        self.single_route(&route.native_symbol, business).await
    }
}

#[async_trait]
impl LiveTradingAdapter for GateCrossEx {
    fn name(&self) -> &'static str {
        NAME
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_testnet: false,
            supports_live: self.allow_live_writes,
            supports_spot: true,
            supports_perp: true,
            supports_limit_orders: true,
            supports_market_orders: true,
            supports_post_only: true,
            supports_reduce_only: true,
        }
    }

    fn exchange_capabilities(&self, exchange: &str) -> ExchangeResult<ExchangeCapabilities> {
        let mut capabilities = self.capabilities();
        let underlying = exchange
            .strip_prefix("gate_crossex:")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(underlying.as_str(), "kraken" | "hyperliquid" | "deribit") {
            capabilities.supports_spot = false;
        }
        Ok(capabilities)
    }

    async fn get_exchange_account_mode(
        &self,
        _exchange: &str,
    ) -> ExchangeResult<Option<VenueAccountModeInfo>> {
        let observed_at_ms = common::time::now_ms();
        let body = fetch_account(&self.http, &self.rest_base_url, self.credentials()?).await?;
        let (_, summary) = parse_account_text(&body, observed_at_ms)?;
        Ok(Some(VenueAccountModeInfo {
            venue: NAME.to_owned(),
            mode: summary.account_type,
            source: summary.source,
            checked_at_ms: observed_at_ms,
            freshness_ms: Some(0),
            account_scope: Some("crossex_unified".to_owned()),
        }))
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.place_order_with_context(
            intent,
            &OrderSubmissionContext {
                product: FeeProduct::Perp,
                ..Default::default()
            },
        )
        .await
    }

    async fn place_order_with_context(
        &self,
        intent: &OrderIntent,
        context: &OrderSubmissionContext,
    ) -> ExchangeResult<OrderAck> {
        self.ensure_live_writes()?;
        let route = self
            .trade_route(&intent.exchange, &intent.symbol, context.product)
            .await?;
        let compiled = compile_order(intent, Some(context), &route)?;
        self.private_stream()?.place_order(intent, &compiled).await
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.ensure_live_writes()?;
        self.private_stream()?.cancel_order(request).await
    }

    async fn get_order(
        &self,
        _symbol: &str,
        client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let private = self.private_stream()?;
        private.warm().await?;
        let venue_client_id =
            crate::client_order_id_policy::required_venue_client_order_id(NAME, client_order_id)?;
        if let Some(order) = private.order_by_client_id(&venue_client_id) {
            return Ok(Some(order));
        }
        let Some(body) = fetch_order(
            &self.http,
            &self.rest_base_url,
            self.credentials()?,
            &venue_client_id,
        )
        .await?
        else {
            return Ok(None);
        };
        parse_order_text(&body).map(Some)
    }

    async fn get_order_by_exchange_order_id(
        &self,
        _symbol: &str,
        exchange_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        let private = self.private_stream()?;
        private.warm().await?;
        if let Some(order) = private.order_by_exchange_id(exchange_order_id) {
            return Ok(Some(order));
        }
        let Some(body) = fetch_order(
            &self.http,
            &self.rest_base_url,
            self.credentials()?,
            exchange_order_id,
        )
        .await?
        else {
            return Ok(None);
        };
        parse_order_text(&body).map(Some)
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        let private = self.private_stream()?;
        private.warm().await?;
        let cached = private.open_orders(symbol);
        if private.has_order_sample() {
            return Ok(cached);
        }
        let native_symbol = symbol.filter(|value| CrossExRoute::parse(value).is_ok());
        let body = fetch_open_orders(
            &self.http,
            &self.rest_base_url,
            self.credentials()?,
            native_symbol,
        )
        .await?;
        let mut rows = parse_orders_text(&body)?;
        if let Some(symbol) = symbol {
            rows.retain(|row| row.symbol.eq_ignore_ascii_case(symbol) || native_symbol.is_some());
        }
        Ok(rows)
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        let private = self.private_stream()?;
        private.warm().await?;
        let cached = private.balances(currency);
        if private.has_balance_sample() {
            return Ok(cached);
        }
        let body = fetch_account(&self.http, &self.rest_base_url, self.credentials()?).await?;
        let (mut balances, _) = parse_account_text(&body, common::time::now_ms())?;
        if let Some(currency) = currency {
            balances.retain(|row| row.currency.eq_ignore_ascii_case(currency));
        }
        Ok(balances)
    }

    async fn get_account_read(&self, currency: Option<&str>) -> ExchangeResult<VenueAccountRead> {
        self.private_stream()?.warm().await?;
        let body = fetch_account(&self.http, &self.rest_base_url, self.credentials()?).await?;
        let (mut balances, summary) = parse_account_text(&body, common::time::now_ms())?;
        let private_balances = self.private_stream()?.balances(currency);
        if self.private_stream()?.has_balance_sample() {
            balances = private_balances;
        } else if let Some(currency) = currency {
            balances.retain(|row| row.currency.eq_ignore_ascii_case(currency));
        }
        Ok(VenueAccountRead {
            balances,
            summaries: vec![summary],
            asset_valuations: Vec::new(),
            issues: Vec::new(),
        })
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        let private = self.private_stream()?;
        private.warm().await?;
        let cached = private.positions(symbol);
        if private.has_position_sample() {
            return Ok(cached);
        }
        let native_symbol = symbol.filter(|value| CrossExRoute::parse(value).is_ok());
        let body = fetch_positions(
            &self.http,
            &self.rest_base_url,
            self.credentials()?,
            native_symbol,
        )
        .await?;
        let mut rows = parse_positions_text(&body)?;
        if let Some(symbol) = symbol {
            rows.retain(|row| row.symbol.eq_ignore_ascii_case(symbol) || native_symbol.is_some());
        }
        Ok(rows)
    }
}

#[async_trait]
impl ExchangeAdapter for GateCrossEx {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
        self.refresh_instruments().await?;
        if let Some(credentials) = &self.credentials {
            let intervals =
                fetch_funding_intervals(&self.http, &self.rest_base_url, credentials).await?;
            self.funding_intervals.store(Arc::new(intervals));
        }
        Ok(MetadataRefreshOutcome::Refreshed)
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        self.require_funding_interval_metadata()?;
        let route = self.single_route(symbol, CrossExBusiness::Future).await?;
        let symbols = [route.native_symbol];
        self.stream.touch_tickers(&symbols);
        self.stream.touch_funding(&symbols);
        let deadline = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            if let Some(row) = self
                .stream
                .funding_snapshot(&symbols, &self.funding_intervals.load())
                .into_iter()
                .next()
            {
                return Ok(row);
            }
            if Instant::now() >= deadline {
                return Err(ExchangeError::UnsupportedCapability(
                    "gate_crossex_funding_interval_metadata",
                ));
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        self.require_funding_interval_metadata()?;
        let routes = self
            .routes(symbols, CrossExBusiness::Future, "funding_rate")
            .await?;
        self.stream.touch_tickers(&routes);
        self.stream.touch_funding(&routes);
        Ok(self
            .stream
            .funding_snapshot(&routes, &self.funding_intervals.load()))
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        let route = self.single_route(symbol, CrossExBusiness::Future).await?;
        self.await_ticker(&route).await
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        let routes = self
            .routes(symbols, CrossExBusiness::Future, "ticker")
            .await?;
        self.stream.touch_tickers(&routes);
        Ok(self.stream.ticker_snapshot(&routes))
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        let routes = self
            .routes(Some(symbols), CrossExBusiness::Future, "ticker")
            .await?;
        self.stream.touch_tickers(&routes);
        let rows = self.stream.ticker_snapshot(&routes);
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
        self.require_funding_interval_metadata()?;
        let routes = self
            .routes(Some(symbols), CrossExBusiness::Future, "funding_rate")
            .await?;
        self.stream.touch_funding(&routes);
        let rows = self
            .stream
            .funding_snapshot(&routes, &self.funding_intervals.load());
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
        let routes = self
            .routes(Some(symbols), CrossExBusiness::Future, "mark_price")
            .await?;
        self.stream.touch_references(&routes);
        let rows = self.stream.reference_snapshot(&routes);
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
        let routes = self
            .routes(symbols, CrossExBusiness::Future, "mark_price")
            .await?;
        self.stream.touch_references(&routes);
        Ok(self.stream.reference_snapshot(&routes))
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        let routes = self
            .routes(Some(symbols), CrossExBusiness::Spot, "ticker")
            .await?;
        self.stream.touch_tickers(&routes);
        let rows = self.stream.spot_snapshot(&routes);
        Ok(if rows.is_empty() {
            PublicWsSnapshot::Pending
        } else {
            PublicWsSnapshot::Ready(rows)
        })
    }

    async fn public_ws_route_quote_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<GateCrossExRouteQuote>> {
        let routes = self.exact_native_routes(symbols).await?;
        self.stream.touch_tickers(&routes);
        let rows = self.stream.route_quote_snapshot(&routes);
        Ok(if rows.is_empty() {
            PublicWsSnapshot::Pending
        } else {
            PublicWsSnapshot::Ready(rows)
        })
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        let routes = self
            .routes(symbols, CrossExBusiness::Spot, "ticker")
            .await?;
        self.stream.touch_tickers(&routes);
        Ok(self.stream.spot_snapshot(&routes))
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let route = self.single_route(symbol, CrossExBusiness::Future).await?;
        self.await_book(&route, depth).await
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let route = self.single_route(symbol, CrossExBusiness::Future).await?;
        let multiplier = self.depth_multiplier(&route)?;
        if !self
            .stream
            .touch_book(&route, usize::try_from(depth.max(1)).unwrap_or(1))
        {
            return Ok(PublicWsSnapshot::Unsupported);
        }
        Ok(self
            .stream
            .latest_book(
                &route,
                usize::try_from(depth.max(1)).unwrap_or(1),
                multiplier,
            )
            .map(|row| PublicWsSnapshot::Ready(vec![row]))
            .unwrap_or(PublicWsSnapshot::Pending))
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let route = self.single_route(symbol, CrossExBusiness::Spot).await?;
        self.await_book(&route, depth).await
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let route = self.single_route(symbol, CrossExBusiness::Spot).await?;
        let multiplier = self.depth_multiplier(&route)?;
        if !self
            .stream
            .touch_book(&route, usize::try_from(depth.max(1)).unwrap_or(1))
        {
            return Ok(PublicWsSnapshot::Unsupported);
        }
        Ok(self
            .stream
            .latest_book(
                &route,
                usize::try_from(depth.max(1)).unwrap_or(1),
                multiplier,
            )
            .map(|row| PublicWsSnapshot::Ready(vec![row]))
            .unwrap_or(PublicWsSnapshot::Pending))
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        self.refresh_instruments().await
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        CrossExRoute::parse(symbol).map_or_else(
            |_| {
                symbol
                    .trim()
                    .split_once(':')
                    .map_or(symbol.trim(), |(_, base)| base)
                    .to_ascii_uppercase()
            },
            |route| route.base,
        )
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        super::gate_crossex_symbols::route_from_scoped_symbol(
            symbol,
            CrossExBusiness::Future,
            DEFAULT_UNDERLYING,
        )
        .map_or_else(
            |_| symbol.trim().to_ascii_uppercase(),
            |route| route.native_symbol,
        )
    }
}

fn route_matches_business(symbol: &str, business: CrossExBusiness) -> bool {
    CrossExRoute::parse(symbol).is_ok_and(|route| route.business == business)
}

fn safe_depth_multipliers(rows: &[VenueInstrument]) -> HashMap<String, Decimal> {
    rows.iter()
        .filter_map(|row| {
            let route = CrossExRoute::parse(&row.native_symbol).ok()?;
            let verified_base_units = route.business == CrossExBusiness::Spot
                || matches!(
                    route.underlying_venue.as_str(),
                    "BINANCE" | "BYBIT" | "KRAKEN" | "HYPERLIQUID"
                );
            verified_base_units.then_some((row.native_symbol.clone(), Decimal::ONE))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bare_symbol_uses_explicit_default_only_for_legacy_conversion() {
        let adapter = GateCrossEx::new(GateCrossExConfig::default()).unwrap();
        assert_eq!(adapter.to_exchange_symbol("BTC"), "GATE_FUTURE_BTC_USDT");
        assert_eq!(adapter.to_exchange_symbol("okx:BTC"), "OKX_FUTURE_BTC_USDT");
    }

    #[test]
    fn unsafe_native_contract_depth_units_remain_blocked() {
        let rows = parse_instruments(include_str!(
            "../../fixtures/gate_crossex/symbols_routes.json"
        ))
        .unwrap();
        let multipliers = safe_depth_multipliers(&rows);
        assert!(!multipliers.contains_key("GATE_FUTURE_BTC_USDT"));
        assert_eq!(
            multipliers.get("KRAKEN_FUTURE_BTC_USD"),
            Some(&Decimal::ONE)
        );
        assert_eq!(
            multipliers.get("BINANCE_SPOT_BTC_USDT"),
            Some(&Decimal::ONE)
        );
    }

    #[tokio::test]
    async fn funding_interval_metadata_requires_cross_ex_credentials() {
        let adapter = GateCrossEx::new(GateCrossExConfig::default()).unwrap();
        let error = adapter.require_funding_interval_metadata().unwrap_err();
        assert!(matches!(error, ExchangeError::UnsupportedCapability(_)));
        assert!(error
            .to_string()
            .contains("GATE_CROSSEX_API_KEY and GATE_CROSSEX_API_SECRET"));
    }
}
