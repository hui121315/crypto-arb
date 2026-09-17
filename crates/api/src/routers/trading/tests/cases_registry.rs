#![allow(clippy::panic)]
use super::super::*;

const OPERATION_MATRIX_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/exchange_operation_evidence_matrix.tsv"
));
const REST_OPERATION_MATRIX_COLUMNS: &[(&str, &str, &str)] = &[
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

#[derive(Debug, Default, PartialEq, Eq)]
struct RestMatrixSummaryExpectation {
    venue_count: usize,
    bucket_count: usize,
    recorded_bucket_count: usize,
    missing_bucket_count: usize,
}

#[tokio::test]
async fn ws_operation_registry_route_projects_recorded_evidence() {
    let Json(response) = list_ws_operations().await;
    let row_count: usize = response
        .venues
        .iter()
        .map(|venue| venue.operations.len())
        .sum();

    assert_eq!(exchange::TRADING_WS_VENUE_COUNT, response.venues.len());
    assert_eq!(exchange::TRADING_WS_VENUE_COUNT * 7, row_count);
    assert!(response.venues.iter().all(|venue| {
        venue
            .operations
            .iter()
            .all(|operation| operation.doc_url.starts_with("https://"))
    }));
}

#[tokio::test]
async fn transport_registry_route_unifies_rest_and_ws_evidence() {
    let Json(response) = list_transport_registry().await;
    let rest_matrix = rest_operation_matrix_summary_expectation();
    let rest_matrix_row_count = rest_operation_matrix_expectations().len();
    let rest_row_count: usize = response
        .rest
        .venues
        .iter()
        .map(|venue| venue.endpoints.len())
        .sum();
    let ws_row_count: usize = response
        .websocket
        .venues
        .iter()
        .map(|venue| venue.operations.len())
        .sum();

    assert!(!response.rest.venues.is_empty());
    assert_eq!(
        exchange::TRADING_WS_VENUE_COUNT,
        response.websocket.venues.len()
    );
    assert_eq!(rest_matrix_row_count, response.summary.rest_venue_count);
    assert_eq!(rest_row_count, response.summary.rest_endpoint_count);
    assert_eq!(
        rest_matrix.venue_count,
        response.summary.rest_operation_matrix_venue_count
    );
    assert_eq!(
        rest_matrix.bucket_count,
        response.summary.rest_operation_matrix_bucket_count
    );
    assert_eq!(
        rest_matrix.recorded_bucket_count,
        response.summary.rest_operation_matrix_recorded_bucket_count
    );
    assert_eq!(
        rest_matrix.missing_bucket_count,
        response.summary.rest_operation_matrix_missing_bucket_count
    );
    assert_eq!(
        exchange::TRADING_WS_VENUE_COUNT,
        response.summary.ws_venue_count
    );
    assert_eq!(exchange::TRADING_WS_VENUE_COUNT * 7, ws_row_count);
    assert_eq!(
        exchange::TRADING_WS_VENUE_COUNT * 7,
        response.summary.ws_operation_count
    );
    assert_eq!(0, response.summary.ws_unknown_scope_rows);
    assert_eq!(0, response.summary.ws_close_position_rows);
}

#[tokio::test]
async fn transport_registry_route_summary_matches_operation_matrix_tsv() {
    let Json(response) = list_transport_registry().await;
    let expected = rest_operation_matrix_summary_expectation();

    assert_eq!(
        expected.venue_count,
        response.summary.rest_operation_matrix_venue_count
    );
    assert_eq!(
        expected.bucket_count,
        response.summary.rest_operation_matrix_bucket_count
    );
    assert_eq!(
        expected.recorded_bucket_count,
        response.summary.rest_operation_matrix_recorded_bucket_count
    );
    assert_eq!(
        expected.missing_bucket_count,
        response.summary.rest_operation_matrix_missing_bucket_count
    );
}

#[tokio::test]
async fn transport_registry_route_preserves_rest_operation_matrix_buckets() {
    let Json(response) = list_transport_registry().await;

    for (venue_name, buckets) in rest_operation_matrix_expectations() {
        let venue = response
            .rest
            .venues
            .iter()
            .find(|row| row.venue == venue_name)
            .unwrap_or_else(|| panic!("missing REST transport venue {venue_name}"));
        for (use_case, data_kind) in buckets {
            let mut found = false;
            let mut found_recorded = false;
            for endpoint in venue.endpoints.iter().filter(|endpoint| {
                endpoint.use_cases.iter().any(|value| value == use_case)
                    && endpoint.data_kinds.iter().any(|value| value == data_kind)
            }) {
                found = true;
                found_recorded |= rest_endpoint_has_recorded_evidence(endpoint);
            }
            assert!(
                found,
                "{venue_name} transport registry missing REST {use_case}/{data_kind}"
            );
            assert!(
                found_recorded,
                "{venue_name} transport registry has no recorded REST {use_case}/{data_kind}"
            );
        }
    }
}

fn rest_endpoint_has_recorded_evidence(endpoint: &shared_types::RestEndpointRow) -> bool {
    [
        endpoint.schema_hash.as_str(),
        endpoint.fixture_id.as_str(),
        endpoint.parser_test.as_str(),
        endpoint.request_builder_test.as_str(),
        endpoint.auth_kind.as_str(),
    ]
    .into_iter()
    .all(|value| value != shared_types::UNRECORDED_EVIDENCE_MARKER)
}

fn rest_operation_matrix_expectations() -> Vec<RestMatrixExpectation> {
    let mut lines = OPERATION_MATRIX_TSV.lines();
    let Some(header) = lines.next() else {
        panic!("operation matrix header missing");
    };
    let headers: Vec<&str> = header.split('\t').collect();
    let venue_index = column_index(&headers, "venue");
    let rest_columns: Vec<(usize, &'static str, &'static str)> = REST_OPERATION_MATRIX_COLUMNS
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

fn rest_operation_matrix_summary_expectation() -> RestMatrixSummaryExpectation {
    let mut lines = OPERATION_MATRIX_TSV.lines();
    let Some(header) = lines.next() else {
        panic!("operation matrix header missing");
    };
    let headers: Vec<&str> = header.split('\t').collect();
    let rest_indexes: Vec<usize> = REST_OPERATION_MATRIX_COLUMNS
        .iter()
        .map(|(column, _, _)| column_index(&headers, column))
        .collect();

    let mut summary = RestMatrixSummaryExpectation::default();
    for line in lines.filter(|line| !line.trim().is_empty()) {
        let cells: Vec<&str> = line.split('\t').collect();
        let mut recorded_buckets = 0;
        for index in &rest_indexes {
            summary.bucket_count += 1;
            if cells.get(*index).copied().unwrap_or_default().trim() == "recorded" {
                recorded_buckets += 1;
                summary.recorded_bucket_count += 1;
            } else {
                summary.missing_bucket_count += 1;
            }
        }
        if recorded_buckets == rest_indexes.len() {
            summary.venue_count += 1;
        }
    }
    summary
}

fn column_index(headers: &[&str], column: &str) -> usize {
    headers
        .iter()
        .position(|header| *header == column)
        .unwrap_or_else(|| panic!("operation matrix missing column {column}"))
}

#[tokio::test]
async fn transport_registry_route_keeps_ack_only_boundaries() {
    let Json(response) = list_transport_registry().await;
    let mut ack_rows = 0;
    let mut order_status_rows = 0;
    let mut fill_rows = 0;
    let mut close_position_rows = 0;

    for venue in &response.websocket.venues {
        for operation in &venue.operations {
            match operation.label.as_str() {
                "place_order" | "cancel_order" => {
                    ack_rows += 1;
                    assert_eq!(
                        shared_types::ExchangeWsEvidenceScope::AckOnly,
                        operation.evidence_scope,
                        "{}/{} write evidence must remain ACK-scoped: {:?}",
                        venue.venue,
                        operation.label,
                        operation
                    );
                }
                "order_status" => {
                    order_status_rows += 1;
                    assert_eq!(
                        shared_types::ExchangeWsEvidenceScope::OrderStatusRead,
                        operation.evidence_scope
                    );
                }
                "fill_stream" => {
                    fill_rows += 1;
                    assert_eq!(
                        shared_types::ExchangeWsEvidenceScope::PrivateFillStream,
                        operation.evidence_scope
                    );
                }
                "close_position" => close_position_rows += 1,
                _ => {}
            }
        }
    }

    assert_eq!(exchange::TRADING_WS_VENUE_COUNT * 2, ack_rows);
    assert_eq!(exchange::TRADING_WS_VENUE_COUNT, order_status_rows);
    assert_eq!(exchange::TRADING_WS_VENUE_COUNT, fill_rows);
    assert_eq!(0, close_position_rows);
    assert_eq!(ack_rows, response.summary.ws_ack_only_rows);
    assert_eq!(
        order_status_rows,
        response.summary.ws_order_status_read_rows
    );
    assert_eq!(
        exchange::TRADING_WS_VENUE_COUNT * 4,
        response.summary.ws_private_stream_rows
    );
    assert_eq!(close_position_rows, response.summary.ws_close_position_rows);
}
