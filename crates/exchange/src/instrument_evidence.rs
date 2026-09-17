//! Public evidence predicates shared with orchestration crates.

use shared_types::instrument_registry::VenueInstrument;

pub fn spot_instrument_evidence_matches(instrument: &VenueInstrument) -> bool {
    crate::adapters::spot_instruments::evidence_matches(instrument)
}

pub fn official_instrument_evidence_matches(instrument: &VenueInstrument) -> bool {
    spot_instrument_evidence_matches(instrument) || matches_new_venue_evidence(instrument)
}

fn matches_new_venue_evidence(instrument: &VenueInstrument) -> bool {
    use shared_types::instruments::InstrumentMetadataSource;

    if instrument.source != InstrumentMetadataSource::OfficialEndpoint {
        return false;
    }
    let source = instrument.source_url.as_deref();
    let schema = instrument.schema_version.as_deref();
    let family = shared_types::venue_family(&instrument.venue).to_ascii_lowercase();
    match (family.as_str(), instrument.product_type.as_deref()) {
        ("kraken", Some("spot")) => {
            evidence_matches(
                source,
                schema,
                "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument",
                "kraken-spot-ws-v2-instrument-2026-08-06",
            ) || evidence_matches(
                source,
                schema,
                "https://api.kraken.com/0/public/AssetPairs?assetVersion=1",
                "kraken-spot-rest-asset-pairs-v1-2026-08-11",
            ) || evidence_matches(
                source,
                schema,
                "https://api.kraken.com/0/public/AssetPairs?assetVersion=1&aclass_base=tokenized_asset",
                "kraken-spot-rest-asset-pairs-v1-2026-08-11",
            )
        }
        ("kraken", Some("perp" | "inverse_perp")) => evidence_matches(
            source,
            schema,
            "https://futures.kraken.com/derivatives/api/v3/instruments",
            "kraken-futures-instruments-v3-2026-08-06",
        ),
        ("gate_crossex", Some("spot" | "perp")) => evidence_matches(
            source,
            schema,
            "https://api.gateio.ws/api/v4/crossex/rule/symbols",
            "crossex-rest-v1.0.2",
        ),
        _ => false,
    }
}

fn evidence_matches(
    source: Option<&str>,
    schema: Option<&str>,
    expected_source: &str,
    expected_schema: &str,
) -> bool {
    source == Some(expected_source) && schema == Some(expected_schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::instrument_registry::InstrumentAssetClass;
    use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};

    fn kraken_spot(source_url: &str, schema_version: &str) -> VenueInstrument {
        VenueInstrument {
            venue: "kraken".to_owned(),
            native_symbol: "PUPS/USD".to_owned(),
            canonical_symbol: "PUPS".to_owned(),
            display_symbol: "PUPS/USD".to_owned(),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("spot".to_owned()),
            quote_asset: Some("USD".to_owned()),
            settle_asset: Some("USD".to_owned()),
            margin_asset: None,
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.000_001),
            qty_step: Some(1.0),
            min_qty: Some(3_000.0),
            min_notional: Some(0.5),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: None,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some(source_url.to_owned()),
            checked_at_ms: 1,
            schema_version: Some(schema_version.to_owned()),
        }
    }

    #[test]
    fn accepts_both_official_kraken_spot_instrument_sources() {
        let ws = kraken_spot(
            "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument",
            "kraken-spot-ws-v2-instrument-2026-08-06",
        );
        let rest = kraken_spot(
            "https://api.kraken.com/0/public/AssetPairs?assetVersion=1",
            "kraken-spot-rest-asset-pairs-v1-2026-08-11",
        );

        assert!(matches_new_venue_evidence(&ws));
        assert!(matches_new_venue_evidence(&rest));
        let stocks=kraken_spot("https://api.kraken.com/0/public/AssetPairs?assetVersion=1&aclass_base=tokenized_asset","kraken-spot-rest-asset-pairs-v1-2026-08-11");
        assert!(matches_new_venue_evidence(&stocks));
    }
}
