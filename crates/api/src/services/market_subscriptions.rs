use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use shared_types::{
    MarketSubscriptionFeedRuntime, MarketSubscriptionPatch, MarketSubscriptionRuntimeState,
    MarketSubscriptionsResponse, VenueMarketSubscription, VenueMarketSubscriptionRuntime,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use thiserror::Error;

use crate::services::market_data::{
    MarketQuality, MarketRuntimeHealth, MarketSource, MARKET_OP_FUNDING_RATES,
    MARKET_OP_PERP_TICKERS, MARKET_OP_REST_FUNDING_RATES, MARKET_OP_REST_PERP_TICKERS,
    MARKET_OP_REST_SPOT_TICKS, MARKET_OP_SPOT_TICKS, MARKET_OP_WS_FUNDING_SNAPSHOT,
    MARKET_OP_WS_SPOT_SNAPSHOT, MARKET_OP_WS_TICKER_SNAPSHOT,
};

const CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub(crate) enum MarketSubscriptionFeed {
    Spot,
    Perp,
    Funding,
}

#[derive(Debug)]
pub(crate) struct MarketSubscriptions {
    path: Option<PathBuf>,
    rows: ArcSwap<BTreeMap<String, VenueMarketSubscription>>,
    updated_at_ms: AtomicI64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Checkpoint {
    version: u32,
    updated_at_ms: i64,
    venues: Vec<VenueMarketSubscription>,
}

#[derive(Debug, Error)]
pub(crate) enum MarketSubscriptionsError {
    #[error("market subscription storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("market subscription serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("market subscription venue is empty")]
    EmptyVenue,
    #[error("market subscription venue is not supported: {0}")]
    UnsupportedVenue(String),
}

impl MarketSubscriptions {
    pub(crate) fn load(path: Option<PathBuf>) -> Self {
        let (rows, updated_at_ms) = load_checkpoint_rows(path.as_deref());
        Self {
            path,
            rows: ArcSwap::from_pointee(rows),
            updated_at_ms: AtomicI64::new(updated_at_ms),
        }
    }

    pub(crate) fn enabled(&self, venue: &str, feed: MarketSubscriptionFeed) -> bool {
        let rows = self.rows.load();
        let key = venue_key(venue);
        let row = rows
            .get(&key)
            .or_else(|| key.split_once(':').and_then(|(family, _)| rows.get(family)));
        let Some(row) = row else {
            return default_feed_enabled(&key);
        };
        match feed {
            MarketSubscriptionFeed::Spot => row.spot_enabled,
            MarketSubscriptionFeed::Perp => row.perp_enabled,
            MarketSubscriptionFeed::Funding => row.perp_enabled && row.funding_enabled,
        }
    }

    pub(crate) fn snapshot(
        &self,
        known_venues: impl IntoIterator<Item = String>,
    ) -> MarketSubscriptionsResponse {
        let configured = self.rows.load();
        let mut rows = BTreeMap::new();
        for venue in known_venues {
            let key = venue_key(&venue);
            if !key.is_empty() && !key.contains(':') && supported_subscription_venue(&key) {
                rows.entry(key.clone())
                    .or_insert_with(|| default_subscription(key));
            }
        }
        for (venue, row) in configured.iter() {
            if supported_subscription_venue(venue) {
                rows.insert(venue.clone(), row.clone());
            }
        }
        MarketSubscriptionsResponse {
            updated_at_ms: self.updated_at_ms.load(Ordering::Acquire),
            venues: rows.into_values().collect(),
            runtime: Vec::new(),
        }
    }

    pub(crate) fn snapshot_with_runtime(
        &self,
        known_venues: impl IntoIterator<Item = String>,
        health: &[MarketRuntimeHealth],
    ) -> MarketSubscriptionsResponse {
        let mut snapshot = self.snapshot(known_venues);
        snapshot.runtime = snapshot
            .venues
            .iter()
            .map(|row| VenueMarketSubscriptionRuntime {
                venue: row.venue.clone(),
                spot: feed_runtime(
                    &row.venue,
                    row.spot_enabled,
                    MarketSubscriptionFeed::Spot,
                    health,
                ),
                perp: feed_runtime(
                    &row.venue,
                    row.perp_enabled,
                    MarketSubscriptionFeed::Perp,
                    health,
                ),
                funding: feed_runtime(
                    &row.venue,
                    row.perp_enabled && row.funding_enabled,
                    MarketSubscriptionFeed::Funding,
                    health,
                ),
            })
            .collect();
        snapshot
    }

    pub(crate) fn update(
        &self,
        patch: &MarketSubscriptionPatch,
    ) -> Result<VenueMarketSubscription, MarketSubscriptionsError> {
        let key = venue_key(&patch.venue);
        if key.is_empty() {
            return Err(MarketSubscriptionsError::EmptyVenue);
        }
        if !supported_subscription_venue(&key) {
            return Err(MarketSubscriptionsError::UnsupportedVenue(key));
        }
        let mut rows = (*self.rows.load_full()).clone();
        let updated = {
            let row = rows
                .entry(key.clone())
                .or_insert_with(|| default_subscription(key));
            if let Some(enabled) = patch.spot_enabled {
                row.spot_enabled = enabled;
            }
            if let Some(enabled) = patch.perp_enabled {
                row.perp_enabled = enabled;
            }
            if let Some(enabled) = patch.funding_enabled {
                row.funding_enabled = enabled;
            }
            row.clone()
        };
        let updated_at_ms = common::time::now_ms();
        persist(self.path.as_deref(), updated_at_ms, &rows)?;
        self.rows.store(Arc::new(rows));
        self.updated_at_ms.store(updated_at_ms, Ordering::Release);
        Ok(updated)
    }
}

fn load_checkpoint_rows(path: Option<&Path>) -> (BTreeMap<String, VenueMarketSubscription>, i64) {
    match path.map_or(Ok(None), read_checkpoint) {
        Ok(Some(checkpoint)) => rows_from_checkpoint(checkpoint),
        Ok(None) => (BTreeMap::new(), 0),
        Err(error) => {
            tracing::warn!(error = %error, "market subscription checkpoint replay failed; defaults remain enabled");
            (BTreeMap::new(), 0)
        }
    }
}

fn rows_from_checkpoint(
    checkpoint: Checkpoint,
) -> (BTreeMap<String, VenueMarketSubscription>, i64) {
    if checkpoint.version != CHECKPOINT_VERSION {
        tracing::warn!(
            version = checkpoint.version,
            "market subscription checkpoint version is unsupported; defaults remain enabled"
        );
        return (BTreeMap::new(), 0);
    }
    (
        checkpoint
            .venues
            .into_iter()
            .filter(|row| supported_subscription_venue(&venue_key(&row.venue)))
            .map(|row| (venue_key(&row.venue), row))
            .collect(),
        checkpoint.updated_at_ms,
    )
}

fn venue_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn default_feed_enabled(venue: &str) -> bool {
    let family = venue.split_once(':').map_or(venue, |(family, _)| family);
    supported_subscription_venue(family) && family != "gate_crossex"
}

fn supported_subscription_venue(venue: &str) -> bool {
    matches!(
        venue.split_once(':').map_or(venue, |(family, _)| family),
        "binance"
            | "bitget"
            | "bybit"
            | "gate"
            | "gate_crossex"
            | "hyperliquid"
            | "kraken"
            | "kucoin"
            | "okx"
    )
}

fn default_subscription(venue: String) -> VenueMarketSubscription {
    let enabled = default_feed_enabled(&venue);
    VenueMarketSubscription {
        venue,
        spot_enabled: enabled,
        perp_enabled: enabled,
        funding_enabled: enabled,
    }
}

fn feed_runtime(
    venue: &str,
    enabled: bool,
    feed: MarketSubscriptionFeed,
    health: &[MarketRuntimeHealth],
) -> MarketSubscriptionFeedRuntime {
    if !enabled {
        return MarketSubscriptionFeedRuntime {
            state: MarketSubscriptionRuntimeState::Disabled,
            ..Default::default()
        };
    }
    let row = health
        .iter()
        .filter(|row| venue_matches(venue, &row.venue) && operation_matches(feed, row.operation))
        .max_by_key(|row| row.observed_at_ms);
    let Some(row) = row else {
        return MarketSubscriptionFeedRuntime::default();
    };
    let state = match (row.quality, row.source) {
        (MarketQuality::Fresh, MarketSource::WsPush) => MarketSubscriptionRuntimeState::Live,
        (MarketQuality::Warming, _) => MarketSubscriptionRuntimeState::Warming,
        _ => MarketSubscriptionRuntimeState::Degraded,
    };
    MarketSubscriptionFeedRuntime {
        state,
        rows: row.rows,
        source: Some(row.source.as_str().to_owned()),
        observed_at_ms: Some(row.observed_at_ms),
    }
}

fn venue_matches(configured: &str, observed: &str) -> bool {
    let configured = venue_key(configured);
    let observed = venue_key(observed);
    observed == configured
        || observed
            .strip_prefix(&configured)
            .is_some_and(|suffix| suffix.starts_with(':'))
}

fn operation_matches(feed: MarketSubscriptionFeed, operation: &str) -> bool {
    match feed {
        MarketSubscriptionFeed::Spot => matches!(
            operation,
            MARKET_OP_SPOT_TICKS | MARKET_OP_WS_SPOT_SNAPSHOT | MARKET_OP_REST_SPOT_TICKS
        ),
        MarketSubscriptionFeed::Perp => matches!(
            operation,
            MARKET_OP_PERP_TICKERS | MARKET_OP_WS_TICKER_SNAPSHOT | MARKET_OP_REST_PERP_TICKERS
        ),
        MarketSubscriptionFeed::Funding => matches!(
            operation,
            MARKET_OP_FUNDING_RATES | MARKET_OP_WS_FUNDING_SNAPSHOT | MARKET_OP_REST_FUNDING_RATES
        ),
    }
}

fn read_checkpoint(path: &Path) -> Result<Option<Checkpoint>, MarketSubscriptionsError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn persist(
    path: Option<&Path>,
    updated_at_ms: i64,
    rows: &BTreeMap<String, VenueMarketSubscription>,
) -> Result<(), MarketSubscriptionsError> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let checkpoint = Checkpoint {
        version: CHECKPOINT_VERSION,
        updated_at_ms,
        venues: rows.values().cloned().collect(),
    };
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(&checkpoint)?)?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_defaults_every_feed_to_enabled() {
        let config = MarketSubscriptions::load(None);
        assert!(config.enabled("kraken", MarketSubscriptionFeed::Spot));
        assert!(config.enabled("kraken", MarketSubscriptionFeed::Perp));
        assert!(config.enabled("kraken", MarketSubscriptionFeed::Funding));
    }

    #[test]
    fn gate_crossex_requires_an_explicit_subscription() {
        let config = MarketSubscriptions::load(None);
        assert!(!config.enabled("gate_crossex", MarketSubscriptionFeed::Spot));
        assert!(!config.enabled("gate_crossex:gate", MarketSubscriptionFeed::Perp));
        assert!(!config.enabled("gate_crossex:kraken", MarketSubscriptionFeed::Funding));

        let snapshot = config.snapshot(["gate_crossex".to_owned()]);
        let row = snapshot
            .venues
            .iter()
            .find(|row| row.venue == "gate_crossex")
            .expect("Gate CrossEx must stay visible as an opt-in venue");
        assert!(!row.spot_enabled);
        assert!(!row.perp_enabled);
        assert!(!row.funding_enabled);
    }

    #[test]
    fn retired_htx_cannot_reenter_the_subscription_surface() {
        let config = MarketSubscriptions::load(None);
        assert!(!config.enabled("htx", MarketSubscriptionFeed::Spot));
        assert!(config.snapshot(["htx".to_owned()]).venues.is_empty());
        assert!(matches!(
            config.update(&MarketSubscriptionPatch {
                venue: "htx".to_owned(),
                spot_enabled: Some(true),
                perp_enabled: Some(true),
                funding_enabled: Some(true),
            }),
            Err(MarketSubscriptionsError::UnsupportedVenue(venue)) if venue == "htx"
        ));
    }

    #[test]
    fn family_config_controls_scoped_routes_and_persists() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "crossline-market-subscriptions-{}-{}.json",
            std::process::id(),
            common::time::now_ms()
        ));
        let config = MarketSubscriptions::load(Some(path.clone()));
        let updated = config.update(&MarketSubscriptionPatch {
            venue: "gate_crossex".to_owned(),
            spot_enabled: Some(false),
            perp_enabled: None,
            funding_enabled: None,
        })?;
        assert!(!updated.spot_enabled);
        assert!(!config.enabled("gate_crossex:gate", MarketSubscriptionFeed::Spot));

        let restored = MarketSubscriptions::load(Some(path.clone()));
        assert!(!restored.enabled("gate_crossex:okx", MarketSubscriptionFeed::Spot));
        let _ = fs::remove_file(path);
        Ok(())
    }
}
