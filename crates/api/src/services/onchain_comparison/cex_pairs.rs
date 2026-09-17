use crate::services::instrument_registry::SpotRegistryState;
use crate::services::spot;
use crate::state::AppState;
use common::AppError;
use shared_types::instruments::InstrumentListingStatus;
use shared_types::{
    ApiProblem, MarketDataEnvelope, MarketDataQuality, OnchainCexPairCatalog, OnchainCexPairOption,
    OnchainCexPairQuery, SpotTicksPage, SpotTicksQuery, VenueInstrument, ONCHAIN_CEX_VENUES,
};
use std::collections::BTreeMap;
use std::sync::Arc;

const PAIR_CATALOG_SOURCE: &str = "instrument-registry";

pub(crate) async fn catalog(
    state: &AppState,
    query: &OnchainCexPairQuery,
) -> Result<OnchainCexPairCatalog, AppError> {
    let venue = required_venue(&query.venue)?;
    let base_token = normalized_asset(&query.base_token);
    if base_token.is_empty() {
        return Err(AppError::BadRequest("baseToken is required".to_owned()));
    }
    let spot_query = SpotTicksQuery {
        base: Some(base_token.clone()),
        venue: Some(venue.clone()),
        limit: Some(128),
        ..SpotTicksQuery::default()
    };
    let envelope = spot::filtered_ticks_cached(state, &spot_query);
    let registry = state.instrument_registry();
    let instruments = registry.venue_instruments(&venue);
    let now_ms = common::time::now_ms();
    let registry_state = registry.spot_registry_state(&venue, now_ms);
    if !matches!(&registry_state, SpotRegistryState::Ready { .. })
        && registry.spot_instrument_refresh_retry_due(&venue, common::time::now_ms(), 30_000)
    {
        crate::services::instrument_registry::request_spot_refresh(
            venue.clone(),
            Arc::clone(state.aggregator_handle()),
            Arc::clone(registry),
        );
    }
    Ok(project_catalog(
        venue,
        base_token,
        &instruments,
        envelope,
        &registry_state,
    ))
}

fn project_catalog(
    venue: String,
    base_token: String,
    instruments: &[VenueInstrument],
    envelope: MarketDataEnvelope<SpotTicksPage>,
    registry_state: &SpotRegistryState,
) -> OnchainCexPairCatalog {
    let fallback_health = envelope.health;
    let evidence = envelope
        .row_evidence
        .into_iter()
        .map(|row| ((row.venue.to_ascii_lowercase(), row.symbol.clone()), row))
        .collect::<BTreeMap<_, _>>();
    let mut market_by_symbol = BTreeMap::new();
    for tick in envelope.data.ticks {
        let Some((base, quote)) = spot::split_spot_pair(&tick.symbol) else {
            continue;
        };
        if !base.eq_ignore_ascii_case(&base_token) || !tick.venue.eq_ignore_ascii_case(&venue) {
            continue;
        }
        let row = evidence.get(&(tick.venue.to_ascii_lowercase(), tick.symbol.clone()));
        let health = row.map_or(&fallback_health, |row| &row.health);
        let cex_symbol = format!("{base}/{quote}");
        market_by_symbol.insert(
            cex_symbol,
            (
                health.quality,
                health.source,
                health.freshness_ms,
                health.observed_at_ms.max(tick.received_at_ms),
            ),
        );
    }
    let mut by_symbol = BTreeMap::<String, OnchainCexPairOption>::new();
    for instrument in instruments.iter().filter(|instrument| {
        is_official_spot_instrument(instrument)
            && instrument
                .canonical_symbol
                .eq_ignore_ascii_case(&base_token)
    }) {
        let Some(quote_token) = instrument
            .quote_asset
            .as_deref()
            .map(normalized_asset)
            .filter(|quote| !quote.is_empty())
        else {
            continue;
        };
        let cex_symbol = format!("{base_token}/{quote_token}");
        let (quality, source, freshness_ms, observed_at_ms) =
            market_by_symbol.get(&cex_symbol).copied().unwrap_or((
                MarketDataQuality::Missing,
                shared_types::MarketDataSourceKind::LocalCache,
                None,
                instrument.checked_at_ms,
            ));
        let option = OnchainCexPairOption {
            venue: venue.clone(),
            base_token: base_token.clone(),
            quote_token,
            cex_symbol: cex_symbol.clone(),
            native_symbol: instrument.native_symbol.clone(),
            quality,
            source,
            freshness_ms,
            observed_at_ms,
        };
        let replace = by_symbol
            .get(&cex_symbol)
            .is_none_or(|current| option.observed_at_ms > current.observed_at_ms);
        if replace {
            by_symbol.insert(cex_symbol, option);
        }
    }
    let mut pairs = by_symbol.into_values().collect::<Vec<_>>();
    pairs.sort_by(|left, right| {
        quality_rank(left.quality)
            .cmp(&quality_rank(right.quality))
            .then_with(|| quote_rank(&left.quote_token).cmp(&quote_rank(&right.quote_token)))
            .then_with(|| left.cex_symbol.cmp(&right.cex_symbol))
    });
    let problem = catalog_problem(&venue, &base_token, registry_state, pairs.is_empty());
    OnchainCexPairCatalog {
        venue,
        base_token,
        pairs,
        problem,
    }
}

fn catalog_problem(
    venue: &str,
    base_token: &str,
    state: &SpotRegistryState,
    empty: bool,
) -> Option<ApiProblem> {
    if matches!(state, SpotRegistryState::Ready { .. }) && !empty {
        return None;
    }
    let (code, message) = match state {
        SpotRegistryState::Ready { .. } => (
            "ONCHAIN_CEX_PAIR_MISSING",
            format!("{venue} 最新官方 Spot instrument registry 没有 {base_token} 交易对"),
        ),
        SpotRegistryState::Syncing => (
            "ONCHAIN_CEX_PAIR_REGISTRY_SYNCING",
            format!("{venue} 官方 Spot instrument registry 正在完成首次同步"),
        ),
        SpotRegistryState::Stale { .. } => (
            "ONCHAIN_CEX_PAIR_REGISTRY_STALE",
            format!("{venue} 官方 Spot instrument registry 已过期，正在刷新"),
        ),
        SpotRegistryState::Failed { problem, .. } => (
            "ONCHAIN_CEX_PAIR_REGISTRY_UNAVAILABLE",
            format!(
                "{venue} 官方 Spot instrument registry 刷新失败：{}",
                problem.message
            ),
        ),
        SpotRegistryState::Unsupported { problem, .. } => (
            "ONCHAIN_CEX_PAIR_REGISTRY_UNSUPPORTED",
            format!(
                "{venue} 尚未接入官方 Spot instrument registry：{}",
                problem.message
            ),
        ),
    };
    Some(ApiProblem::new(code, message).with_source(PAIR_CATALOG_SOURCE))
}

fn is_official_spot_instrument(instrument: &VenueInstrument) -> bool {
    instrument
        .product_type
        .as_deref()
        .is_some_and(|product| product.eq_ignore_ascii_case("spot"))
        && instrument.listing_status == InstrumentListingStatus::Trading
        && instrument.has_official_provenance()
}

fn required_venue(value: &str) -> Result<String, AppError> {
    let venue = value.trim().to_ascii_lowercase();
    ONCHAIN_CEX_VENUES
        .iter()
        .any(|option| option.id == venue)
        .then_some(venue)
        .ok_or_else(|| AppError::BadRequest("unsupported on-chain CEX venue".to_owned()))
}

fn normalized_asset(value: &str) -> String {
    value
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| char::from(byte.to_ascii_uppercase()))
        .collect()
}

const fn quality_rank(quality: MarketDataQuality) -> u8 {
    match quality {
        MarketDataQuality::Fresh => 0,
        MarketDataQuality::StaleAllowed => 1,
        MarketDataQuality::StaleBlocked => 2,
        MarketDataQuality::RateLimited => 3,
        MarketDataQuality::CircuitOpen => 4,
        MarketDataQuality::Missing => 5,
        MarketDataQuality::Unsupported => 6,
        MarketDataQuality::Unverified => 7,
    }
}

fn quote_rank(quote: &str) -> u8 {
    match quote {
        "USDC" => 0,
        "USDT" => 1,
        "USD" => 2,
        "BTC" => 3,
        "ETH" => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use shared_types::instruments::InstrumentMetadataSource;
    use shared_types::{
        InstrumentAssetClass, ListPage, MarketDataHealth, MarketDataRowEvidence,
        MarketDataSnapshotOperation, MarketDataSourceKind, SpotTick,
    };

    #[test]
    fn catalog_uses_structured_spot_rows_and_prefers_stable_quotes() {
        let envelope = MarketDataEnvelope {
            data: SpotTicksPage {
                ticks: vec![tick("WIFUSDT", 90), tick("WIFUSDC", 100)],
                page: ListPage::default(),
                ..SpotTicksPage::default()
            },
            health: health(MarketDataQuality::Fresh, 100),
            retry_after_ms: None,
            row_cap: None,
            row_evidence: vec![evidence("WIFUSDT", 90), evidence("WIFUSDC", 100)],
            fanout: Vec::new(),
        };

        let instruments = vec![
            instrument("binance", "WIFUSDT", "WIF", "USDT", 90),
            instrument("binance", "WIFUSDC", "WIF", "USDC", 100),
        ];
        let catalog = project_catalog(
            "binance".to_owned(),
            "WIF".to_owned(),
            &instruments,
            envelope,
            &SpotRegistryState::Ready { checked_at_ms: 100 },
        );

        assert_eq!(catalog.pairs.len(), 2);
        assert_eq!(catalog.pairs[0].cex_symbol, "WIF/USDC");
        assert_eq!(catalog.pairs[0].native_symbol, "WIFUSDC");
        assert_eq!(catalog.pairs[1].cex_symbol, "WIF/USDT");
        assert!(catalog.problem.is_none());
    }

    #[test]
    fn catalog_does_not_attach_unrelated_aggregate_problem_to_valid_pairs() {
        let mut envelope = MarketDataEnvelope {
            data: SpotTicksPage {
                ticks: vec![tick("ETHUSDC", 100)],
                page: ListPage::default(),
                ..SpotTicksPage::default()
            },
            health: health(MarketDataQuality::Fresh, 100),
            retry_after_ms: None,
            row_cap: None,
            row_evidence: vec![evidence("ETHUSDC", 100)],
            fanout: Vec::new(),
        };
        envelope.health.problem = Some(ApiProblem::new(
            "UNRELATED_BASELINE_FAILURE",
            "another venue timed out",
        ));

        let instruments = vec![instrument("binance", "ETHUSDC", "ETH", "USDC", 100)];
        let catalog = project_catalog(
            "binance".to_owned(),
            "ETH".to_owned(),
            &instruments,
            envelope,
            &SpotRegistryState::Ready { checked_at_ms: 100 },
        );

        assert_eq!(catalog.pairs.len(), 1);
        assert!(catalog.problem.is_none());
    }

    #[test]
    fn catalog_accepts_kraken_native_spot_symbols_from_ws_cache() {
        let envelope = MarketDataEnvelope {
            data: SpotTicksPage {
                ticks: vec![tick_for_venue("kraken", "SOL/USD", 100)],
                page: ListPage::default(),
                ..SpotTicksPage::default()
            },
            health: health(MarketDataQuality::Fresh, 100),
            retry_after_ms: None,
            row_cap: None,
            row_evidence: vec![evidence_for_venue("kraken", "SOL/USD", 100)],
            fanout: Vec::new(),
        };

        let instruments = vec![instrument("kraken", "SOL/USD", "SOL", "USD", 100)];
        let catalog = project_catalog(
            "kraken".to_owned(),
            "SOL".to_owned(),
            &instruments,
            envelope,
            &SpotRegistryState::Ready { checked_at_ms: 100 },
        );

        assert_eq!(catalog.pairs.len(), 1);
        assert_eq!(catalog.pairs[0].cex_symbol, "SOL/USD");
        assert_eq!(catalog.pairs[0].native_symbol, "SOL/USD");
        assert_eq!(catalog.pairs[0].source, MarketDataSourceKind::WsPush);
        assert!(catalog.problem.is_none());
    }

    #[test]
    fn official_pair_remains_selectable_before_its_ticker_arrives() {
        let envelope = MarketDataEnvelope {
            data: SpotTicksPage::default(),
            health: health(MarketDataQuality::Missing, 100),
            retry_after_ms: None,
            row_cap: None,
            row_evidence: Vec::new(),
            fanout: Vec::new(),
        };
        let instruments = vec![instrument("kraken", "PUPS/USD", "PUPS", "USD", 100)];

        let catalog = project_catalog(
            "kraken".to_owned(),
            "PUPS".to_owned(),
            &instruments,
            envelope,
            &SpotRegistryState::Ready { checked_at_ms: 100 },
        );

        assert_eq!(catalog.pairs.len(), 1);
        assert_eq!(catalog.pairs[0].cex_symbol, "PUPS/USD");
        assert_eq!(catalog.pairs[0].quality, MarketDataQuality::Missing);
        assert!(catalog.problem.is_none());
    }

    #[test]
    fn restored_pair_stays_selectable_but_is_marked_as_stale_evidence() {
        let instruments = vec![instrument("binance", "SOLUSDC", "SOL", "USDC", 100)];

        let catalog = project_catalog(
            "binance".to_owned(),
            "SOL".to_owned(),
            &instruments,
            empty_envelope(),
            &SpotRegistryState::Stale { checked_at_ms: 100 },
        );

        assert_eq!(catalog.pairs.len(), 1);
        assert_eq!(catalog.pairs[0].cex_symbol, "SOL/USDC");
        assert_eq!(
            catalog
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some("ONCHAIN_CEX_PAIR_REGISTRY_STALE")
        );
    }

    #[test]
    fn empty_catalog_distinguishes_first_sync_from_a_proven_missing_pair() {
        let syncing = project_catalog(
            "kraken".to_owned(),
            "PUPS".to_owned(),
            &[],
            empty_envelope(),
            &SpotRegistryState::Syncing,
        );
        assert_eq!(
            syncing
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some("ONCHAIN_CEX_PAIR_REGISTRY_SYNCING")
        );

        let missing = project_catalog(
            "kraken".to_owned(),
            "PUPS".to_owned(),
            &[],
            empty_envelope(),
            &SpotRegistryState::Ready { checked_at_ms: 100 },
        );
        assert_eq!(
            missing
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some("ONCHAIN_CEX_PAIR_MISSING")
        );
    }

    fn tick(symbol: &str, received_at_ms: i64) -> SpotTick {
        tick_for_venue("binance", symbol, received_at_ms)
    }

    fn empty_envelope() -> MarketDataEnvelope<SpotTicksPage> {
        MarketDataEnvelope {
            data: SpotTicksPage::default(),
            health: health(MarketDataQuality::Missing, 100),
            retry_after_ms: None,
            row_cap: None,
            row_evidence: Vec::new(),
            fanout: Vec::new(),
        }
    }

    fn tick_for_venue(venue: &str, symbol: &str, received_at_ms: i64) -> SpotTick {
        SpotTick {
            venue: venue.to_owned(),
            symbol: symbol.to_owned(),
            bid: Decimal::ONE,
            ask: Decimal::ONE,
            last: Decimal::ONE,
            bid_size: Some(Decimal::ONE),
            ask_size: Some(Decimal::ONE),
            volume_24h: Decimal::ONE,
            exchange_ts_ms: Some(received_at_ms),
            received_at_ms,
        }
    }

    fn evidence(symbol: &str, observed_at_ms: i64) -> MarketDataRowEvidence {
        evidence_for_venue("binance", symbol, observed_at_ms)
    }

    fn evidence_for_venue(venue: &str, symbol: &str, observed_at_ms: i64) -> MarketDataRowEvidence {
        MarketDataRowEvidence {
            venue: venue.to_owned(),
            symbol: symbol.to_owned(),
            operation: MarketDataSnapshotOperation::WsSpotTicks,
            health: health(MarketDataQuality::Fresh, observed_at_ms),
        }
    }

    fn health(quality: MarketDataQuality, observed_at_ms: i64) -> MarketDataHealth {
        MarketDataHealth {
            quality,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms,
            coverage: None,
            problem: None,
        }
    }

    fn instrument(
        venue: &str,
        native_symbol: &str,
        base: &str,
        quote: &str,
        checked_at_ms: i64,
    ) -> VenueInstrument {
        VenueInstrument {
            venue: venue.to_owned(),
            native_symbol: native_symbol.to_owned(),
            canonical_symbol: base.to_owned(),
            display_symbol: format!("{base}/{quote}"),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("spot".to_owned()),
            quote_asset: Some(quote.to_owned()),
            settle_asset: Some(quote.to_owned()),
            margin_asset: None,
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.000001),
            qty_step: Some(0.00001),
            min_qty: Some(1.0),
            min_notional: Some(0.5),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: None,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("https://example.test/instruments".to_owned()),
            checked_at_ms,
            schema_version: Some("test-v1".to_owned()),
        }
    }
}
