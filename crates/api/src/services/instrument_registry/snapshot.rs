use super::InstrumentRegistry;
use shared_types::instrument_registry::VenueInstrument;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

#[derive(Debug, Default)]
pub(super) struct InstrumentLookupSnapshot {
    by_venue_symbol: HashMap<(String, String), Vec<VenueInstrument>>,
    health_by_scope: HashMap<String, InstrumentHealthSummary>,
}

#[derive(Debug, Default)]
pub(super) struct InstrumentHealthSummary {
    rows: usize,
    constructible_checked_at_ms: Vec<i64>,
    schema_versions: Vec<String>,
    source_urls: Vec<String>,
}

impl InstrumentLookupSnapshot {
    pub(super) fn rows(&self, venue: &str, symbol: &str) -> &[VenueInstrument] {
        self.by_venue_symbol
            .get(&(venue.trim().to_ascii_lowercase(), canonical_symbol(symbol)))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn health(&self, venue: &str) -> Option<&InstrumentHealthSummary> {
        self.health_by_scope.get(&venue.trim().to_ascii_lowercase())
    }
}

impl InstrumentHealthSummary {
    pub(super) const fn rows(&self) -> usize {
        self.rows
    }

    pub(super) fn execution_ready_rows_at(&self, now_ms: i64, max_age_ms: i64) -> usize {
        self.constructible_checked_at_ms
            .iter()
            .filter(|checked_at_ms| {
                max_age_ms > 0
                    && now_ms >= **checked_at_ms
                    && now_ms.saturating_sub(**checked_at_ms) < max_age_ms
            })
            .count()
    }

    pub(super) fn schema_versions(&self) -> Vec<String> {
        self.schema_versions.clone()
    }

    pub(super) fn source_urls(&self) -> Vec<String> {
        self.source_urls.clone()
    }
}

impl InstrumentRegistry {
    pub(super) fn instrument_lookup_snapshot(&self) -> Arc<InstrumentLookupSnapshot> {
        self.instrument_lookup.load_full()
    }

    pub(super) fn rebuild_instrument_lookup(&self) {
        let mut by_venue_symbol = HashMap::with_capacity(self.by_key.len());
        let mut health_by_scope = HashMap::<String, InstrumentHealthBuilder>::new();
        for entry in &self.by_key {
            let instrument = entry.value();
            let venue = instrument.venue.trim().to_ascii_lowercase();
            by_venue_symbol
                .entry((
                    venue.clone(),
                    canonical_symbol(&instrument.canonical_symbol),
                ))
                .or_insert_with(Vec::new)
                .push(instrument.clone());
            add_health_row(&mut health_by_scope, &venue, instrument);
            let family = shared_types::venue_family(&venue).to_ascii_lowercase();
            if family != venue {
                add_health_row(&mut health_by_scope, &family, instrument);
            }
        }
        self.instrument_lookup
            .store(Arc::new(InstrumentLookupSnapshot {
                by_venue_symbol,
                health_by_scope: health_by_scope
                    .into_iter()
                    .map(|(venue, summary)| (venue, summary.finish()))
                    .collect(),
            }));
    }
}

#[derive(Default)]
struct InstrumentHealthBuilder {
    rows: usize,
    constructible_checked_at_ms: Vec<i64>,
    schema_versions: BTreeSet<String>,
    source_urls: BTreeSet<String>,
}

impl InstrumentHealthBuilder {
    fn finish(self) -> InstrumentHealthSummary {
        InstrumentHealthSummary {
            rows: self.rows,
            constructible_checked_at_ms: self.constructible_checked_at_ms,
            schema_versions: self.schema_versions.into_iter().collect(),
            source_urls: self.source_urls.into_iter().collect(),
        }
    }
}

fn add_health_row(
    summaries: &mut HashMap<String, InstrumentHealthBuilder>,
    venue: &str,
    instrument: &VenueInstrument,
) {
    let summary = summaries.entry(venue.to_owned()).or_default();
    summary.rows += 1;
    if instrument.is_hedge_constructible() {
        summary
            .constructible_checked_at_ms
            .push(instrument.checked_at_ms);
    }
    if let Some(schema_version) = instrument.schema_version.as_ref() {
        summary.schema_versions.insert(schema_version.clone());
    }
    if let Some(source_url) = instrument.source_url.as_ref() {
        summary.source_urls.insert(source_url.clone());
    }
}

fn canonical_symbol(value: &str) -> String {
    exchange::strip_common_suffixes(value.trim()).to_ascii_uppercase()
}
