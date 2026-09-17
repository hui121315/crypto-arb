use super::InstrumentRegistry;
use std::sync::Arc;
use tracing::{info, warn};

pub(crate) async fn refresh_spot_venue(
    venue: &str,
    aggregator: &exchange::Aggregator,
    registry: &InstrumentRegistry,
) -> Result<(), String> {
    if !registry.begin_spot_instrument_refresh(venue) {
        return Ok(());
    }
    let result = refresh_started(venue, aggregator, registry).await;
    registry.finish_spot_instrument_refresh(venue);
    result
}

pub(crate) fn request_spot_refresh(
    venue: String,
    aggregator: Arc<exchange::Aggregator>,
    registry: Arc<InstrumentRegistry>,
) -> bool {
    if !registry.begin_spot_instrument_refresh(&venue) {
        return false;
    }
    tokio::spawn(async move {
        let result = refresh_started(&venue, &aggregator, &registry).await;
        registry.finish_spot_instrument_refresh(&venue);
        if let Err(error) = result {
            warn!(venue, error = %error, "on-demand Spot instrument refresh failed");
        }
    });
    true
}

async fn refresh_started(
    venue: &str,
    aggregator: &exchange::Aggregator,
    registry: &InstrumentRegistry,
) -> Result<(), String> {
    let Some(adapter) = aggregator.get(venue) else {
        let error = format!("{venue} Spot adapter not registered");
        registry.record_spot_unsupported(venue, &error);
        return Err(error);
    };
    let instruments = match adapter.fetch_spot_instruments().await {
        Ok(instruments) => instruments,
        Err(error) => {
            let message = format!("{venue} fetch_spot_instruments failed: {error}");
            if matches!(
                &error,
                exchange::ExchangeError::UnsupportedCapability(_)
                    | exchange::ExchangeError::NotImplemented(_)
            ) {
                registry.record_spot_unsupported(venue, &message);
            } else {
                registry.record_spot_refresh_failure(venue, &message);
            }
            return Err(message);
        }
    };
    let fetched = instruments.len();
    let accepted = registry.replace_spot_venue(venue, instruments);
    if accepted == 0 {
        let error = format!("{venue} Spot refresh accepted 0 of {fetched} instruments");
        registry.record_spot_refresh_failure(venue, &error);
        return Err(error);
    }
    info!(
        venue,
        fetched, accepted, "Spot instrument registry refreshed"
    );
    persist_after_refresh(venue, registry).await;
    Ok(())
}

async fn persist_after_refresh(venue: &str, registry: &InstrumentRegistry) {
    if let Err(error) = registry.persist_checkpoint().await {
        warn!(venue, %error, "Spot instrument registry checkpoint persist failed");
    }
}
