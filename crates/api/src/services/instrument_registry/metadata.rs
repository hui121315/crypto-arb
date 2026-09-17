use shared_types::instrument_registry::VenueInstrument;
use shared_types::RestEndpointRow;
use std::sync::OnceLock;

pub(super) fn normalized_product_key(product_type: Option<&str>) -> String {
    let product = product_type.unwrap_or_default().trim().to_ascii_lowercase();
    match product.as_str() {
        "perp" | "perpetual" | "swap" => "perp".to_owned(),
        "spot" => "spot".to_owned(),
        "" => "unknown".to_owned(),
        _ => product,
    }
}

pub(super) fn normalized_native_symbol(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

pub(super) const fn fee_product_key(product: shared_types::FeeProduct) -> Option<&'static str> {
    match product {
        shared_types::FeeProduct::Perp => Some("perp"),
        shared_types::FeeProduct::Spot => Some("spot"),
        shared_types::FeeProduct::Margin | shared_types::FeeProduct::Unknown => None,
    }
}

pub(super) fn venue_in_scope(candidate: &str, scope: &str) -> bool {
    let candidate = candidate.trim();
    let scope = scope.trim();
    if scope.contains(':') {
        candidate.eq_ignore_ascii_case(scope)
    } else {
        shared_types::venue_family(candidate).eq_ignore_ascii_case(scope)
    }
}

pub(crate) fn instrument_metadata_evidence(venue: &str) -> Option<&'static RestEndpointRow> {
    let family = venue
        .trim()
        .split_once(':')
        .map_or_else(|| venue.trim(), |(family, _)| family);
    instrument_metadata_evidence_rows()
        .iter()
        .find_map(|(venue, row)| venue.eq_ignore_ascii_case(family).then_some(row))
}

fn instrument_metadata_evidence_rows() -> &'static [(String, RestEndpointRow)] {
    static ROWS: OnceLock<Vec<(String, RestEndpointRow)>> = OnceLock::new();
    ROWS.get_or_init(|| {
        exchange::rest_endpoint_registry()
            .venues
            .into_iter()
            .filter_map(|venue| {
                let endpoint = venue.endpoints.into_iter().find(|row| {
                    row.use_cases.iter().any(|value| value == "metadata")
                        && row
                            .data_kinds
                            .iter()
                            .any(|value| value == "instrument_metadata")
                })?;
                Some((venue.venue.to_ascii_lowercase(), endpoint))
            })
            .collect()
    })
}

pub(super) fn metadata_evidence_matches(
    instrument: &VenueInstrument,
    evidence: &RestEndpointRow,
) -> bool {
    let source_matches = instrument.source_url.as_deref().is_some_and(|source| {
        source_matches_endpoint(source, &evidence.path)
            || evidence.doc_urls.iter().any(|url| url == source)
    });
    instrument.has_official_provenance()
        && source_matches
        && instrument.schema_version.as_deref() == Some(evidence.doc_version.as_str())
}

fn source_matches_endpoint(source: &str, endpoint_path: &str) -> bool {
    source == endpoint_path
        || source.strip_prefix(endpoint_path).is_some_and(|suffix| {
            suffix.starts_with('?') || suffix.starts_with(char::is_whitespace)
        })
}
