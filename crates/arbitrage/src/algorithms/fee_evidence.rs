//! Verified venue fee schedule evidence used by scanners and execution preview.

use std::collections::HashSet;
use std::sync::OnceLock;

use shared_types::{
    normalized_venue_name, FeeProduct, FeeScheduleRegistryResponse, FeeScheduleRegistryRow,
    TradeFeeEvidence, TradeFeeSnapshot, TradeFeeSource,
};

pub const STANDARD_FEE_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
pub const STANDARD_FEE_VENUE_FAMILIES: [&str; 7] = [
    "binance",
    "bitget",
    "bybit",
    "gate",
    "hyperliquid",
    "kucoin",
    "okx",
];
pub const STANDARD_FEE_PRODUCTS: [FeeProduct; 2] = [FeeProduct::Spot, FeeProduct::Perp];
const STANDARD_FEE_REGISTRY_SCHEMA_VERSION: &str = "fee_schedule_registry_v2";
const STANDARD_FEE_REGISTRY_FINGERPRINT: &str = "7ea5df0ec584b800";
const STANDARD_FEE_REGISTRY_FIXTURE: &str =
    include_str!("../../fixtures/fee_schedule_registry_v2.json");
static STANDARD_FEE_REGISTRY: OnceLock<
    Result<FeeScheduleRegistryResponse, FeeScheduleRegistryError>,
> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeeScheduleRegistryError {
    Decode(String),
    InvalidFixture,
}

impl std::fmt::Display for FeeScheduleRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(message) => {
                write!(formatter, "fee schedule registry decode failed: {message}")
            }
            Self::InvalidFixture => formatter.write_str(
                "fee schedule registry schema, fingerprint, or evidence rows are invalid",
            ),
        }
    }
}

impl std::error::Error for FeeScheduleRegistryError {}

pub fn standard_fee_snapshot(
    venue: &str,
    symbol: &str,
    product: FeeProduct,
    use_maker_fee: bool,
    now_ms: i64,
) -> Option<TradeFeeSnapshot> {
    if !standard_fee_registry_is_current() {
        return None;
    }
    let row = standard_fee_registry_row(venue, product)?;
    let maker_fee_bps = row.maker_fee_bps;
    let taker_fee_bps = row.taker_fee_bps;
    let execution_fee_bps = if use_maker_fee {
        maker_fee_bps
    } else {
        taker_fee_bps
    };
    Some(TradeFeeSnapshot {
        venue: normalized_venue_name(venue),
        symbol: symbol.to_ascii_uppercase(),
        product,
        account_id: None,
        maker_fee_bps,
        taker_fee_bps,
        open_fee_bps: execution_fee_bps.max(0.0),
        close_fee_bps: execution_fee_bps.max(0.0),
        source: TradeFeeSource::OfficialSchedule,
        fetched_at_ms: now_ms,
        valid_until_ms: now_ms.saturating_add(row.snapshot_ttl_ms),
        freshness_ms: Some(0),
        evidence: Some(row.evidence),
        verification_problem: None,
        note: Some("standard venue fee schedule; account/VIP discounts not applied".into()),
    })
}

pub(crate) fn standard_taker_fee_bps(venue: &str, product: FeeProduct) -> Option<f64> {
    if !standard_fee_registry_is_current() {
        return None;
    }
    let family = fee_venue_family(venue);
    parsed_standard_fee_registry()
        .ok()?
        .venues
        .iter()
        .find(|row| row.venue == family)?
        .schedules
        .iter()
        .find(|row| row.product == product)
        .map(|row| row.taker_fee_bps)
}

pub fn standard_fee_schedule_evidence(
    venue: &str,
    product: FeeProduct,
) -> Option<TradeFeeEvidence> {
    if !standard_fee_registry_is_current() {
        return None;
    }
    standard_fee_registry_row(venue, product).map(|row| row.evidence)
}

pub fn standard_fee_schedule_registry(
) -> Result<FeeScheduleRegistryResponse, FeeScheduleRegistryError> {
    parsed_standard_fee_registry().cloned()
}

pub fn standard_fee_registry_is_valid(registry: &FeeScheduleRegistryResponse) -> bool {
    let venue_names = registry
        .venues
        .iter()
        .map(|venue| venue.venue.as_str())
        .collect::<HashSet<_>>();
    registry.schema.matches(
        STANDARD_FEE_REGISTRY_SCHEMA_VERSION,
        STANDARD_FEE_REGISTRY_FINGERPRINT,
    ) && registry.schema.fingerprint == registry_fingerprint(registry)
        && registry.venues.len() == STANDARD_FEE_VENUE_FAMILIES.len()
        && venue_names.len() == STANDARD_FEE_VENUE_FAMILIES.len()
        && registry.venues.iter().all(|venue| {
            STANDARD_FEE_VENUE_FAMILIES.contains(&venue.venue.as_str())
                && venue.schedules.len() == STANDARD_FEE_PRODUCTS.len()
                && STANDARD_FEE_PRODUCTS.iter().all(|product| {
                    venue
                        .schedules
                        .iter()
                        .filter(|row| row.product == *product)
                        .count()
                        == 1
                })
                && venue
                    .schedules
                    .iter()
                    .all(|row| registry_row_is_valid(&venue.venue, row))
        })
}

fn registry_row_is_valid(venue: &str, row: &FeeScheduleRegistryRow) -> bool {
    let Some(version) = non_empty_option(row.evidence.schedule_version.as_deref()) else {
        return false;
    };
    let Some(_tier) = non_empty_option(row.evidence.tier.as_deref()) else {
        return false;
    };
    let Some(scope) = non_empty_option(row.evidence.scope.as_deref()) else {
        return false;
    };
    row.evidence.is_valid()
        && official_source_host_matches(venue, &row.evidence.source_url)
        && row.fixture_id
            == format!(
                "standard-fee:{venue}:{}:{version}",
                product_label(row.product)
            )
        && fixture_symbol_is_scoped(&row.fixture_symbol)
        && scope_matches_product(scope, row.product)
        && row.snapshot_ttl_ms == STANDARD_FEE_TTL_MS
        && row.maker_fee_bps.is_finite()
        && row.maker_fee_bps >= 0.0
        && row.taker_fee_bps.is_finite()
        && row.taker_fee_bps >= 0.0
}

fn non_empty_option(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn product_label(product: FeeProduct) -> &'static str {
    match product {
        FeeProduct::Spot => "spot",
        FeeProduct::Perp => "perp",
        FeeProduct::Margin => "margin",
        FeeProduct::Unknown => "unknown",
    }
}

fn fixture_symbol_is_scoped(symbol: &str) -> bool {
    let symbol = symbol.trim();
    !symbol.is_empty()
        && symbol == symbol.to_ascii_uppercase()
        && symbol.contains("USDT")
        && !symbol.chars().any(char::is_whitespace)
}

fn scope_matches_product(scope: &str, product: FeeProduct) -> bool {
    let scope = scope.to_ascii_lowercase();
    match product {
        FeeProduct::Spot => scope.contains("spot"),
        FeeProduct::Perp => ["perp", "future", "swap", "contract"]
            .into_iter()
            .any(|needle| scope.contains(needle)),
        FeeProduct::Margin | FeeProduct::Unknown => false,
    }
}

fn official_source_host_matches(venue: &str, source_url: &str) -> bool {
    let allowed = match venue {
        "binance" => ["developers.binance.com", "www.binance.com"].as_slice(),
        "bitget" => ["www.bitget.com"].as_slice(),
        "bybit" => ["bybit-exchange.github.io", "www.bybit.com"].as_slice(),
        "gate" => ["www.gate.io", "www.gate.com"].as_slice(),
        "hyperliquid" => ["hyperliquid.gitbook.io"].as_slice(),
        "kucoin" => ["www.kucoin.com"].as_slice(),
        "okx" => ["www.okx.com"].as_slice(),
        _ => return false,
    };
    allowed
        .iter()
        .any(|host| source_url.starts_with(&format!("https://{host}/")))
}

fn standard_fee_registry_is_current() -> bool {
    parsed_standard_fee_registry().is_ok()
}

fn parsed_standard_fee_registry(
) -> Result<&'static FeeScheduleRegistryResponse, FeeScheduleRegistryError> {
    STANDARD_FEE_REGISTRY
        .get_or_init(|| load_standard_fee_registry(STANDARD_FEE_REGISTRY_FIXTURE))
        .as_ref()
        .map_err(Clone::clone)
}

fn load_standard_fee_registry(
    fixture: &str,
) -> Result<FeeScheduleRegistryResponse, FeeScheduleRegistryError> {
    let registry = serde_json::from_str(fixture)
        .map_err(|error| FeeScheduleRegistryError::Decode(error.to_string()))?;
    if standard_fee_registry_is_valid(&registry) {
        Ok(registry)
    } else {
        Err(FeeScheduleRegistryError::InvalidFixture)
    }
}

fn standard_fee_registry_row(venue: &str, product: FeeProduct) -> Option<FeeScheduleRegistryRow> {
    let family = fee_venue_family(venue);
    let Ok(registry) = parsed_standard_fee_registry() else {
        return None;
    };
    registry
        .venues
        .iter()
        .find(|row| row.venue == family)?
        .schedules
        .iter()
        .find(|row| row.product == product)
        .cloned()
}

pub fn matches_standard_fee_fixture(snapshot: &TradeFeeSnapshot) -> bool {
    [true, false].into_iter().any(|use_maker_fee| {
        standard_fee_snapshot(
            &snapshot.venue,
            &snapshot.symbol,
            snapshot.product,
            use_maker_fee,
            snapshot.fetched_at_ms,
        )
        .as_ref()
        .is_some_and(|fixture| standard_fee_fixture_matches(snapshot, fixture))
    })
}

fn standard_fee_fixture_matches(snapshot: &TradeFeeSnapshot, fixture: &TradeFeeSnapshot) -> bool {
    normalized_venue_name(&snapshot.venue) == fixture.venue
        && snapshot.symbol.to_ascii_uppercase() == fixture.symbol
        && snapshot.product == fixture.product
        && fee_matches(snapshot.maker_fee_bps, fixture.maker_fee_bps)
        && fee_matches(snapshot.taker_fee_bps, fixture.taker_fee_bps)
        && fee_matches(snapshot.open_fee_bps, fixture.open_fee_bps)
        && fee_matches(snapshot.close_fee_bps, fixture.close_fee_bps)
        && snapshot.evidence == fixture.evidence
}

fn registry_fingerprint(registry: &FeeScheduleRegistryResponse) -> String {
    let mut fingerprint = 0xcbf2_9ce4_8422_2325_u64;
    for venue in &registry.venues {
        for row in &venue.schedules {
            let version = row.evidence.schedule_version.as_deref().unwrap_or_default();
            let line = format!(
                "{}:{:?}:{:.4}:{:.4}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{};",
                venue.venue,
                row.product,
                row.maker_fee_bps,
                row.taker_fee_bps,
                version,
                row.evidence.evidence_id,
                row.evidence.source_name,
                row.evidence.source_url,
                row.evidence.checked_at_ms,
                row.evidence.effective_at_ms.unwrap_or_default(),
                row.evidence.tier.as_deref().unwrap_or_default(),
                row.evidence.scope.as_deref().unwrap_or_default(),
                row.evidence.problem.as_deref().unwrap_or_default(),
                row.fixture_id,
                row.fixture_symbol,
                row.snapshot_ttl_ms,
            );
            for byte in line.bytes() {
                fingerprint ^= u64::from(byte);
                fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{fingerprint:016x}")
}

fn fee_venue_family(venue: &str) -> String {
    normalized_venue_name(venue)
        .split(':')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn fee_matches(actual_bps: f64, expected_bps: f64) -> bool {
    (actual_bps - expected_bps).abs() <= 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{FeeScheduleVenue, TradeFeeSource};

    #[test]
    fn standard_fee_snapshot_uses_execution_fee_side() -> Result<(), &'static str> {
        let maker = standard_fee_snapshot("hyperliquid:xyz", "MU", FeeProduct::Perp, true, 100)
            .ok_or("standard maker fee missing")?;
        let taker = standard_fee_snapshot("hyperliquid:xyz", "MU", FeeProduct::Perp, false, 100)
            .ok_or("standard taker fee missing")?;

        assert_eq!(maker.source, TradeFeeSource::OfficialSchedule);
        assert!(maker.open_fee_bps < taker.open_fee_bps);
        assert!(maker.is_fresh_verified(101));
        assert!(maker.evidence.is_some());
        Ok(())
    }

    #[test]
    fn standard_fee_snapshot_fails_closed_without_matching_evidence() {
        assert!(standard_fee_snapshot("okx", "BTC-USDT", FeeProduct::Margin, false, 100).is_none());
        assert!(
            standard_fee_snapshot("unknown", "BTC-USDT", FeeProduct::Perp, false, 100).is_none()
        );
    }

    #[test]
    fn standard_fee_snapshot_covers_major_perp_venues() {
        for venue in ["binance", "bitget", "bybit", "gate", "okx"] {
            let snapshot = standard_fee_snapshot(venue, "BTC-USDT", FeeProduct::Perp, false, 100)
                .unwrap_or_else(|| panic!("standard perp fee missing for {venue}"));
            let evidence = snapshot
                .evidence
                .as_ref()
                .unwrap_or_else(|| panic!("evidence missing for {venue}"));
            assert!(evidence.source_url.starts_with("https://"), "{venue}");
            assert!(evidence.evidence_id.contains(venue), "{venue}");
            assert!(matches_standard_fee_fixture(&snapshot), "{venue}");
        }
    }

    #[test]
    fn standard_fee_registry_covers_every_fixture_family() -> Result<(), FeeScheduleRegistryError> {
        let registry = standard_fee_schedule_registry()?;
        assert_eq!(
            registry.schema.fingerprint, STANDARD_FEE_REGISTRY_FINGERPRINT,
            "update the pinned registry fingerprint with this reviewed schema change"
        );
        assert!(standard_fee_registry_is_valid(&registry));
        assert_eq!(
            registry.schema.version,
            STANDARD_FEE_REGISTRY_SCHEMA_VERSION
        );
        assert_eq!(
            registry.schema.fingerprint,
            STANDARD_FEE_REGISTRY_FINGERPRINT
        );
        assert_standard_fee_registry_rows(&registry);
        Ok(())
    }

    fn assert_standard_fee_registry_rows(registry: &FeeScheduleRegistryResponse) {
        assert_eq!(registry.venues.len(), STANDARD_FEE_VENUE_FAMILIES.len());

        let row_count: usize = registry
            .venues
            .iter()
            .map(|venue| venue.schedules.len())
            .sum();
        assert_eq!(row_count, 14);

        for venue in &registry.venues {
            assert!(
                STANDARD_FEE_VENUE_FAMILIES.contains(&venue.venue.as_str()),
                "{}",
                venue.venue
            );
            assert!(!venue.schedules.is_empty(), "{}", venue.venue);
            assert_eq!(venue.schedules.len(), STANDARD_FEE_PRODUCTS.len());
            for product in STANDARD_FEE_PRODUCTS {
                assert_eq!(
                    venue
                        .schedules
                        .iter()
                        .filter(|row| row.product == product)
                        .count(),
                    1,
                    "{} {product:?}",
                    venue.venue
                );
            }
            assert_standard_fee_venue_rows(venue);
        }
    }

    fn assert_standard_fee_venue_rows(venue: &FeeScheduleVenue) {
        for row in &venue.schedules {
            assert!(row.evidence.is_valid(), "{} {:?}", venue.venue, row.product);
            assert_eq!(row.snapshot_ttl_ms, STANDARD_FEE_TTL_MS);
            let fixture = standard_fee_snapshot(
                &venue.venue,
                &row.fixture_symbol,
                row.product,
                false,
                row.evidence.checked_at_ms,
            )
            .unwrap_or_else(|| panic!("missing fixture {} {:?}", venue.venue, row.product));
            assert!(matches_standard_fee_fixture(&fixture));
            assert!(fee_matches(fixture.maker_fee_bps, row.maker_fee_bps));
            assert!(fee_matches(fixture.taker_fee_bps, row.taker_fee_bps));
            assert_eq!(fixture.evidence.as_ref(), Some(&row.evidence));
        }
    }

    #[test]
    fn standard_fee_registry_covers_every_scanner_venue_product() {
        for venue in STANDARD_FEE_VENUE_FAMILIES {
            for product in STANDARD_FEE_PRODUCTS {
                let snapshot = standard_fee_snapshot(venue, "BTC-USDT", product, false, 100)
                    .unwrap_or_else(|| panic!("missing fee fixture for {venue} {product:?}"));
                assert!(snapshot.is_fresh_verified(101));
                assert!(matches_standard_fee_fixture(&snapshot));
            }
        }
    }

    #[test]
    fn standard_fee_registry_fails_closed_when_product_coverage_is_removed() {
        let mut registry = standard_fee_schedule_registry()
            .unwrap_or_else(|error| panic!("fee registry missing: {error}"));
        registry.venues[0]
            .schedules
            .retain(|row| row.product != FeeProduct::Spot);

        assert!(!standard_fee_registry_is_valid(&registry));
    }

    #[test]
    fn standard_fee_registry_fails_closed_on_tier_or_contract_scope_drift() {
        let mut registry = standard_fee_schedule_registry()
            .unwrap_or_else(|error| panic!("fee registry missing: {error}"));
        registry.venues[0].schedules[0].evidence.tier = None;
        assert!(!standard_fee_registry_is_valid(&registry));

        let mut registry = standard_fee_schedule_registry()
            .unwrap_or_else(|error| panic!("fee registry missing: {error}"));
        registry.venues[0].schedules[0].evidence.scope = Some("unknown product".into());
        assert!(!standard_fee_registry_is_valid(&registry));
    }

    #[test]
    fn standard_fee_registry_rejects_non_provider_source_hosts() {
        let mut registry = standard_fee_schedule_registry()
            .unwrap_or_else(|error| panic!("fee registry missing: {error}"));
        registry.venues[0].schedules[0].evidence.source_url = "https://example.com/fees".into();

        assert!(!standard_fee_registry_is_valid(&registry));
    }

    #[test]
    fn standard_fee_registry_fails_closed_on_schema_drift() {
        let mut registry = standard_fee_schedule_registry()
            .unwrap_or_else(|error| panic!("fee registry missing: {error}"));
        registry.venues[0].schedules[0].taker_fee_bps += 0.1;

        assert!(!standard_fee_registry_is_valid(&registry));
    }

    #[test]
    fn standard_fee_registry_fails_closed_on_fixture_metadata_drift() {
        let mut registry = standard_fee_schedule_registry()
            .unwrap_or_else(|error| panic!("fee registry missing: {error}"));
        registry.venues[0].schedules[0]
            .fixture_id
            .push_str(":drift");

        assert!(!standard_fee_registry_is_valid(&registry));
    }

    #[test]
    fn standard_fee_registry_fails_closed_on_evidence_metadata_drift() {
        let mut registry = standard_fee_schedule_registry()
            .unwrap_or_else(|error| panic!("fee registry missing: {error}"));
        registry.venues[0].schedules[0]
            .evidence
            .source_name
            .push_str(" drift");

        assert!(!standard_fee_registry_is_valid(&registry));
    }

    #[test]
    fn malformed_registry_is_reported_instead_of_becoming_an_empty_registry() {
        let result = load_standard_fee_registry("{not-json");

        assert!(matches!(result, Err(FeeScheduleRegistryError::Decode(_))));
    }

    #[test]
    fn standard_fee_snapshot_keeps_builder_venue_evidence_on_parent() -> Result<(), &'static str> {
        let snapshot = standard_fee_snapshot("hyperliquid:xyz", "MU", FeeProduct::Perp, false, 100)
            .ok_or("builder standard fee missing")?;

        assert_eq!(snapshot.venue, "hyperliquid:xyz");
        assert!(snapshot
            .evidence
            .as_ref()
            .is_some_and(|evidence| evidence.evidence_id.contains("hyperliquid")));
        Ok(())
    }
}
