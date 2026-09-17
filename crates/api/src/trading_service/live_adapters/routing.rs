use super::*;

pub(crate) const KUCOIN_FUNDING_SYMBOL_LIMIT: usize = 32;

pub(super) fn empty_capabilities() -> ExchangeCapabilities {
    ExchangeCapabilities {
        supports_testnet: false,
        supports_live: false,
        supports_spot: false,
        supports_perp: false,
        supports_limit_orders: false,
        supports_market_orders: false,
        supports_post_only: false,
        supports_reduce_only: false,
    }
}

pub(super) fn merge_capabilities(
    mut merged: ExchangeCapabilities,
    adapter: &LiveAdapter,
) -> ExchangeCapabilities {
    let next = adapter.capabilities();
    merged.supports_testnet |= next.supports_testnet;
    merged.supports_live |= next.supports_live;
    merged.supports_spot |= next.supports_spot;
    merged.supports_perp |= next.supports_perp;
    merged.supports_limit_orders |= next.supports_limit_orders;
    merged.supports_market_orders |= next.supports_market_orders;
    merged.supports_post_only |= next.supports_post_only;
    merged.supports_reduce_only |= next.supports_reduce_only;
    merged
}

pub(super) fn normalized_venue(venue: &str) -> String {
    normalized_venue_name(venue)
}

pub(super) fn route_family(venue: &str) -> Option<&str> {
    let (family, _) = venue.split_once(':')?;
    (!family.is_empty()).then_some(family)
}

pub(super) fn route_family_route(venue: &str) -> Option<&str> {
    if is_hyperliquid_builder_venue(venue) {
        None
    } else {
        route_family(venue)
    }
}

pub(super) fn balance_route_key(route: &str) -> &str {
    route_family(route).unwrap_or(route)
}

pub(super) fn live_routes_from_credentials(
    credentials: AdapterCredentials,
) -> Result<LiveRouteMap, ExchangeError> {
    let mut routes = BTreeMap::new();
    if let Some((api_key, api_secret)) = credentials.binance_live {
        routes.insert("binance".into(), binance_live_adapter(api_key, api_secret)?);
    }
    if let Some((api_key, api_secret)) = credentials.bybit_live {
        routes.insert("bybit".into(), bybit_live_adapter(api_key, api_secret)?);
    }
    if let Some((api_key, api_secret, passphrase)) = credentials.bitget_live {
        routes.insert(
            "bitget".into(),
            bitget_live_adapter(api_key, api_secret, passphrase)?,
        );
    }
    if let Some((api_key, api_secret)) = credentials.gate_live {
        routes.insert("gate".into(), gate_live_adapter(api_key, api_secret)?);
    }
    if let Some((api_key, api_secret)) = credentials.gate_crossex_live {
        routes.insert(
            "gate_crossex".into(),
            gate_crossex_live_adapter(api_key, api_secret)?,
        );
    }
    if let Some((api_key, api_secret, passphrase)) = credentials.kucoin_live {
        routes.insert(
            "kucoin".into(),
            kucoin_live_adapter(api_key, api_secret, passphrase)?,
        );
    }
    if let Some((api_key, api_secret, passphrase)) = credentials.okx_live {
        routes.insert(
            "okx".into(),
            okx_live_adapter(api_key, api_secret, passphrase)?,
        );
    }
    if let Some(credentials) = credentials
        .kraken_live
        .filter(KrakenAdapterCredentials::is_configured)
    {
        routes.insert("kraken".into(), kraken_live_adapter(credentials)?);
    }
    if let Some(credentials) = credentials.hyperliquid_live {
        for market in HYPERLIQUID_LIVE_MARKETS {
            routes.insert(
                market.venue().into(),
                hyperliquid_live_adapter(
                    credentials.account_address.clone(),
                    credentials.private_key.clone(),
                    credentials.vault_address.clone(),
                    *market,
                )?,
            );
        }
    }
    Ok(routes)
}

pub(crate) async fn account_read_from_credentials(
    credentials: AdapterCredentials,
    failures: Arc<RouteFailureSink>,
) -> Result<VenueAccountRead, ExchangeError> {
    let Some(reader) = account_reader_from_credentials(credentials, failures)? else {
        return Ok(VenueAccountRead::default());
    };
    reader.get_account_read(None).await
}

pub(crate) fn account_reader_from_credentials(
    credentials: AdapterCredentials,
    failures: Arc<RouteFailureSink>,
) -> Result<Option<Arc<LiveVenueRouter>>, ExchangeError> {
    let routes = live_routes_from_credentials(credentials)?;
    Ok(
        (!routes.is_empty())
            .then(|| Arc::new(LiveVenueRouter::with_failure_sink(routes, failures))),
    )
}

pub(crate) async fn funding_payments_from_credentials(
    credentials: AdapterCredentials,
    failures: Arc<RouteFailureSink>,
    kucoin_symbols: &[String],
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
) -> Result<Vec<FundingPaymentData>, ExchangeError> {
    let routes = live_routes_from_credentials(credentials)?;
    if routes.is_empty() {
        return Ok(Vec::new());
    }
    LiveVenueRouter::with_failure_sink(routes, failures)
        .get_configured_funding_payments(kucoin_symbols, start_time_ms, end_time_ms)
        .await
}

pub(crate) fn capability_rows_from_credentials(
    credentials: &AdapterCredentials,
) -> Vec<TradingVenueCapability> {
    let configured = [
        ("binance", credentials.binance_live.is_some()),
        ("okx", credentials.okx_live.is_some()),
        ("bybit", credentials.bybit_live.is_some()),
        ("bitget", credentials.bitget_live.is_some()),
        ("gate", credentials.gate_live.is_some()),
        ("gate_crossex", credentials.gate_crossex_live.is_some()),
        ("kucoin", credentials.kucoin_live.is_some()),
        (
            "kraken",
            credentials
                .kraken_live
                .as_ref()
                .is_some_and(|value| value.is_configured()),
        ),
        ("hyperliquid", credentials.hyperliquid_live.is_some()),
    ];
    configured
        .into_iter()
        .filter_map(|(venue, credentials_available)| {
            let capabilities = exchange::static_exchange_capabilities(venue)?;
            let matrix = exchange::static_venue_capability_matrix(venue)?;
            Some(TradingVenueCapability {
                venue: venue.to_owned(),
                environment: ExecutionEnvironment::Live,
                credentials_available,
                capabilities: shared_capabilities(capabilities),
                matrix,
                source: "exchange.static_venue_capability_matrix".to_owned(),
                problem: None,
            })
        })
        .collect()
}

pub(super) fn shared_capabilities(
    capabilities: ExchangeCapabilities,
) -> TradingAdapterCapabilities {
    TradingAdapterCapabilities {
        spot: capabilities.supports_spot,
        perp: capabilities.supports_perp,
        limit_orders: capabilities.supports_limit_orders,
        market_orders: capabilities.supports_market_orders,
        post_only: capabilities.supports_post_only,
        reduce_only: capabilities.supports_reduce_only,
    }
}
