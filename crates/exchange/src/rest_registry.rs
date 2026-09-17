//! REST endpoint 官方证据注册表：把 `ENDPOINT_SPECS` 与 recorded evidence 投影成 shared DTO。

use crate::venue_spec::{endpoint_evidence_for_spec, EndpointSpec, ENDPOINT_SPECS};
use shared_types::{RestEndpointRow, RestEndpointVenue, RestEndpointsResponse};

pub fn rest_endpoint_registry() -> RestEndpointsResponse {
    let mut venues: Vec<RestEndpointVenue> = Vec::new();
    for spec in ENDPOINT_SPECS {
        // Retired HTX rows are archived evidence for old snapshots only.
        if spec.venue == shared_types::VenueId::Htx {
            continue;
        }
        let venue_name = spec.venue.as_str();
        if !venues.iter().any(|row| row.venue == venue_name) {
            venues.push(RestEndpointVenue {
                venue: venue_name.to_owned(),
                endpoints: Vec::new(),
            });
        }
        let Some(venue_row) = venues.iter_mut().find(|row| row.venue == venue_name) else {
            continue;
        };
        let row = endpoint_row(spec);
        if let Some(existing) = venue_row
            .endpoints
            .iter_mut()
            .find(|existing| same_evidence_identity(existing, &row))
        {
            merge_endpoint_rows(existing, &row);
        } else {
            venue_row.endpoints.push(row);
        }
    }
    RestEndpointsResponse { venues }
}

fn endpoint_row(spec: &EndpointSpec) -> RestEndpointRow {
    let evidence = endpoint_evidence_for_spec(spec);
    RestEndpointRow {
        method: evidence.method,
        path: evidence.path,
        weight: evidence.weight,
        checked_at: evidence.checked_at,
        doc_version: evidence.doc_version,
        schema_hash: evidence.schema_hash,
        fixture_id: evidence.fixture_id,
        parser_test: evidence.parser_test,
        request_builder_test: evidence.request_builder_test,
        auth_kind: evidence.auth_kind,
        doc_urls: evidence.doc_urls,
        use_cases: evidence.use_cases,
        data_kinds: evidence.data_kinds,
        rate_scopes: evidence.rate_scopes,
    }
}

fn same_evidence_identity(left: &RestEndpointRow, right: &RestEndpointRow) -> bool {
    left.method == right.method
        && left.path == right.path
        && left.weight == right.weight
        && left.checked_at == right.checked_at
        && left.doc_version == right.doc_version
        && left.schema_hash == right.schema_hash
        && left.fixture_id == right.fixture_id
        && left.parser_test == right.parser_test
        && left.request_builder_test == right.request_builder_test
        && left.auth_kind == right.auth_kind
        && left.doc_urls == right.doc_urls
        && left.rate_scopes == right.rate_scopes
}

fn merge_endpoint_rows(target: &mut RestEndpointRow, source: &RestEndpointRow) {
    merge_unique(&mut target.use_cases, &source.use_cases);
    merge_unique(&mut target.data_kinds, &source.data_kinds);
}

fn merge_unique(target: &mut Vec<String>, source: &[String]) {
    for value in source {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::UNRECORDED_EVIDENCE_MARKER;
    use std::collections::BTreeSet;

    const OPERATION_MATRIX_TSV: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/exchange_operation_evidence_matrix.tsv"
    ));
    const ENDPOINT_EVIDENCE_ALLOWLIST_TSV: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/exchange_evidence_debt_allowlist.tsv"
    ));
    const OPERATION_MATRIX_REST_COLUMNS: &[(&str, &str, &str)] = &[
        ("rest_trade_write_order_ack", "trade_write", "order_ack"),
        ("rest_private_order_status", "private_read", "order_status"),
        (
            "rest_private_account_balance",
            "private_read",
            "account_balance",
        ),
        (
            "rest_private_account_position",
            "private_read",
            "account_position",
        ),
    ];
    type RestBucket = (&'static str, &'static str);
    type RestMatrixExpectation = (String, Vec<RestBucket>);
    type EndpointProjection = BTreeSet<(String, String)>;

    #[test]
    fn retired_htx_is_not_registered_for_rest() {
        assert!(rest_endpoint_registry()
            .venues
            .iter()
            .all(|venue| venue.venue != "htx"));
    }

    #[test]
    fn registry_covers_every_endpoint_spec_with_exact_evidence() {
        let registry = rest_endpoint_registry();
        for spec in ENDPOINT_SPECS {
            if spec.venue == shared_types::VenueId::Htx {
                continue;
            }
            let expected = endpoint_evidence_for_spec(spec);
            let venue = registry
                .venues
                .iter()
                .find(|row| row.venue == spec.venue.as_str())
                .unwrap_or_else(|| panic!("missing venue {}", spec.venue.as_str()));
            let matched = venue
                .endpoints
                .iter()
                .filter(|row| row_covers_evidence(row, &expected))
                .count();
            assert_eq!(
                matched,
                1,
                "{} {} {} {}",
                venue.venue,
                spec.method.as_str(),
                spec.path,
                spec.doc_url
            );
        }
    }

    fn row_covers_evidence(
        row: &RestEndpointRow,
        expected: &crate::EndpointEvidenceSnapshot,
    ) -> bool {
        same_evidence_identity(
            row,
            &RestEndpointRow {
                method: expected.method.clone(),
                path: expected.path.clone(),
                weight: expected.weight,
                checked_at: expected.checked_at.clone(),
                doc_version: expected.doc_version.clone(),
                schema_hash: expected.schema_hash.clone(),
                fixture_id: expected.fixture_id.clone(),
                parser_test: expected.parser_test.clone(),
                request_builder_test: expected.request_builder_test.clone(),
                auth_kind: expected.auth_kind.clone(),
                doc_urls: expected.doc_urls.clone(),
                use_cases: Vec::new(),
                data_kinds: Vec::new(),
                rate_scopes: expected.rate_scopes.clone(),
            },
        ) && expected
            .use_cases
            .iter()
            .all(|value| row.use_cases.iter().any(|actual| actual == value))
            && expected
                .data_kinds
                .iter()
                .all(|value| row.data_kinds.iter().any(|actual| actual == value))
    }

    #[test]
    fn registry_rows_carry_recorded_official_evidence() {
        for venue in rest_endpoint_registry().venues {
            for row in venue.endpoints {
                assert_ne!(
                    row.doc_version, UNRECORDED_EVIDENCE_MARKER,
                    "{} {} {} lacks recorded doc evidence",
                    venue.venue, row.method, row.path
                );
                assert!(
                    row.doc_urls.iter().all(|url| url.starts_with("https://")),
                    "{} {} {}",
                    venue.venue,
                    row.method,
                    row.path
                );
                assert!(!row.checked_at.is_empty());
            }
        }
    }

    #[test]
    fn operation_matrix_rest_buckets_match_runtime_registry_projection() {
        let registry = rest_endpoint_registry();
        for (venue_name, buckets) in operation_matrix_rest_expectations() {
            let venue = registry
                .venues
                .iter()
                .find(|row| row.venue == venue_name)
                .unwrap_or_else(|| panic!("missing operation matrix venue {venue_name}"));
            for (use_case, data_kind) in buckets {
                let mut found = false;
                let mut found_recorded = false;
                for row in venue.endpoints.iter().filter(|row| {
                    row.use_cases.iter().any(|value| value == use_case)
                        && row.data_kinds.iter().any(|value| value == data_kind)
                }) {
                    found = true;
                    found_recorded |= operation_row_has_recorded_evidence(row);
                }
                assert!(
                    found,
                    "{venue_name} runtime REST registry missing {use_case}/{data_kind}"
                );
                assert!(
                    found_recorded,
                    "{venue_name} runtime REST registry has no recorded {use_case}/{data_kind} row"
                );
            }
        }
    }

    #[test]
    fn operation_matrix_rest_buckets_match_exact_allowlist_endpoint_projection() {
        let registry = rest_endpoint_registry();
        for (venue_name, buckets) in operation_matrix_rest_expectations() {
            let venue = registry
                .venues
                .iter()
                .find(|row| row.venue == venue_name)
                .unwrap_or_else(|| panic!("missing operation matrix venue {venue_name}"));
            for (use_case, data_kind) in buckets {
                let expected =
                    allowlist_recorded_endpoint_projection(&venue_name, use_case, data_kind);
                assert!(
                    !expected.is_empty(),
                    "{venue_name} allowlist missing recorded {use_case}/{data_kind} endpoint"
                );
                let actual =
                    runtime_recorded_endpoint_projection(&venue.endpoints, use_case, data_kind);
                assert_eq!(
                    actual, expected,
                    "{venue_name} {use_case}/{data_kind} exact allowlist endpoint projection drift"
                );
            }
        }
    }

    fn operation_row_has_recorded_evidence(row: &RestEndpointRow) -> bool {
        [
            row.schema_hash.as_str(),
            row.fixture_id.as_str(),
            row.parser_test.as_str(),
            row.request_builder_test.as_str(),
            row.auth_kind.as_str(),
        ]
        .into_iter()
        .all(|value| value != UNRECORDED_EVIDENCE_MARKER)
    }

    fn allowlist_recorded_endpoint_projection(
        venue: &str,
        use_case: &str,
        data_kind: &str,
    ) -> EndpointProjection {
        let mut lines = ENDPOINT_EVIDENCE_ALLOWLIST_TSV.lines();
        let header = lines.next().expect("endpoint evidence allowlist header");
        assert_eq!(
            header,
            "venue\tmethod\tpath\tuse_case\tdata_kind\tchecked_at\tdoc_version\tschema_hash\tfixture_id\tparser_test\trequest_builder_test\tauth_kind"
        );
        lines
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| {
                let cells: Vec<&str> = line.split('\t').collect();
                assert_eq!(
                    cells.len(),
                    12,
                    "allowlist row must have 12 columns: {line}"
                );
                (cells[0].eq_ignore_ascii_case(venue)
                    && allowlist_label_matches(cells[3], use_case)
                    && allowlist_label_matches(cells[4], data_kind)
                    && allowlist_row_has_recorded_private_or_trade_evidence(&cells))
                .then(|| {
                    (
                        allowlist_method_to_registry_method(cells[1]),
                        cells[2].to_owned(),
                    )
                })
            })
            .collect()
    }

    fn runtime_recorded_endpoint_projection(
        endpoints: &[RestEndpointRow],
        use_case: &str,
        data_kind: &str,
    ) -> EndpointProjection {
        endpoints
            .iter()
            .filter(|row| {
                row.use_cases.iter().any(|value| value == use_case)
                    && row.data_kinds.iter().any(|value| value == data_kind)
                    && operation_row_has_recorded_evidence(row)
            })
            .map(|row| (row.method.clone(), row.path.clone()))
            .collect()
    }

    fn allowlist_row_has_recorded_private_or_trade_evidence(cells: &[&str]) -> bool {
        [
            cells[5], cells[6], cells[7], cells[8], cells[9], cells[10], cells[11],
        ]
        .into_iter()
        .all(|value| value != UNRECORDED_EVIDENCE_MARKER)
            && cells[11] != "public"
    }

    fn allowlist_method_to_registry_method(value: &str) -> String {
        match value {
            "Get" => "GET",
            "Post" => "POST",
            "Delete" => "DELETE",
            other => panic!("unsupported allowlist method {other}"),
        }
        .to_owned()
    }

    fn allowlist_label_matches(value: &str, expected: &str) -> bool {
        match value {
            "TradeWrite" => expected == "trade_write",
            "PrivateRead" => expected == "private_read",
            "OrderAck" => expected == "order_ack",
            "OrderStatus" => expected == "order_status",
            "AccountBalance" => expected == "account_balance",
            "AccountPosition" => expected == "account_position",
            _ => false,
        }
    }

    fn operation_matrix_rest_expectations() -> Vec<RestMatrixExpectation> {
        let mut lines = OPERATION_MATRIX_TSV.lines();
        let header = lines.next().expect("operation matrix header");
        let headers: Vec<&str> = header.split('\t').collect();
        let venue_index = column_index(&headers, "venue");
        let rest_columns: Vec<(usize, &'static str, &'static str)> = OPERATION_MATRIX_REST_COLUMNS
            .iter()
            .map(|(column, use_case, data_kind)| {
                (column_index(&headers, column), *use_case, *data_kind)
            })
            .collect();

        lines
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let cells: Vec<&str> = line.split('\t').collect();
                let venue = cells
                    .get(venue_index)
                    .unwrap_or_else(|| panic!("operation matrix row lacks venue: {line}"))
                    .trim()
                    .to_ascii_lowercase();
                let buckets = rest_columns
                    .iter()
                    .filter_map(|(index, use_case, data_kind)| {
                        let value = cells.get(*index).copied().unwrap_or_default().trim();
                        (value == "recorded").then_some((*use_case, *data_kind))
                    })
                    .collect();
                (venue, buckets)
            })
            .collect()
    }

    fn column_index(headers: &[&str], column: &str) -> usize {
        headers
            .iter()
            .position(|header| *header == column)
            .unwrap_or_else(|| panic!("operation matrix missing column {column}"))
    }
}
