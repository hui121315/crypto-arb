//! Unified transport evidence registry DTOs.

use crate::{
    ExchangeWsEvidenceScope, ExchangeWsOperationsResponse, RestEndpointRow, RestEndpointsResponse,
    UNRECORDED_EVIDENCE_MARKER,
};
use serde::{Deserialize, Serialize};

const REST_OPERATION_MATRIX_BUCKETS: &[(&str, &str)] = &[
    ("trade_write", "order_ack"),
    ("private_read", "order_status"),
    ("private_read", "account_balance"),
    ("private_read", "account_position"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeTransportRegistryResponse {
    pub summary: ExchangeTransportRegistrySummary,
    pub rest: RestEndpointsResponse,
    pub websocket: ExchangeWsOperationsResponse,
}

impl ExchangeTransportRegistryResponse {
    #[must_use]
    pub fn new(rest: RestEndpointsResponse, websocket: ExchangeWsOperationsResponse) -> Self {
        let summary = ExchangeTransportRegistrySummary::from_parts(&rest, &websocket);
        Self {
            summary,
            rest,
            websocket,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeTransportRegistrySummary {
    pub rest_venue_count: usize,
    pub rest_endpoint_count: usize,
    pub rest_operation_matrix_venue_count: usize,
    pub rest_operation_matrix_bucket_count: usize,
    pub rest_operation_matrix_recorded_bucket_count: usize,
    pub rest_operation_matrix_missing_bucket_count: usize,
    pub ws_venue_count: usize,
    pub ws_operation_count: usize,
    pub ws_ack_only_rows: usize,
    pub ws_order_status_read_rows: usize,
    pub ws_private_stream_rows: usize,
    pub ws_unknown_scope_rows: usize,
    pub ws_close_position_rows: usize,
}

impl ExchangeTransportRegistrySummary {
    fn from_parts(rest: &RestEndpointsResponse, websocket: &ExchangeWsOperationsResponse) -> Self {
        let mut summary = Self {
            rest_venue_count: rest.venues.len(),
            rest_endpoint_count: rest.venues.iter().map(|venue| venue.endpoints.len()).sum(),
            rest_operation_matrix_venue_count: 0,
            rest_operation_matrix_bucket_count: 0,
            rest_operation_matrix_recorded_bucket_count: 0,
            rest_operation_matrix_missing_bucket_count: 0,
            ws_venue_count: websocket.venues.len(),
            ws_operation_count: 0,
            ws_ack_only_rows: 0,
            ws_order_status_read_rows: 0,
            ws_private_stream_rows: 0,
            ws_unknown_scope_rows: 0,
            ws_close_position_rows: 0,
        };

        for venue in &rest.venues {
            summary.record_rest_operation_matrix_venue(&venue.endpoints);
        }

        for venue in &websocket.venues {
            for operation in &venue.operations {
                summary.ws_operation_count += 1;
                summary.record_ws_operation(operation.label.as_str(), operation.evidence_scope);
            }
        }

        summary
    }

    fn record_rest_operation_matrix_venue(&mut self, endpoints: &[RestEndpointRow]) {
        let mut recorded_buckets = 0;
        for (use_case, data_kind) in REST_OPERATION_MATRIX_BUCKETS {
            self.rest_operation_matrix_bucket_count += 1;
            let has_recorded_bucket = endpoints
                .iter()
                .filter(|endpoint| {
                    endpoint.use_cases.iter().any(|value| value == use_case)
                        && endpoint.data_kinds.iter().any(|value| value == data_kind)
                })
                .any(rest_endpoint_has_recorded_evidence);

            if has_recorded_bucket {
                recorded_buckets += 1;
                self.rest_operation_matrix_recorded_bucket_count += 1;
            } else {
                self.rest_operation_matrix_missing_bucket_count += 1;
            }
        }

        if recorded_buckets == REST_OPERATION_MATRIX_BUCKETS.len() {
            self.rest_operation_matrix_venue_count += 1;
        }
    }

    fn record_ws_operation(&mut self, label: &str, scope: ExchangeWsEvidenceScope) {
        if label == "close_position" {
            self.ws_close_position_rows += 1;
        }

        match scope {
            ExchangeWsEvidenceScope::AckOnly => self.ws_ack_only_rows += 1,
            ExchangeWsEvidenceScope::OrderStatusRead => self.ws_order_status_read_rows += 1,
            ExchangeWsEvidenceScope::PrivateAccountStream
            | ExchangeWsEvidenceScope::PrivatePositionStream
            | ExchangeWsEvidenceScope::PrivateFillStream
            | ExchangeWsEvidenceScope::PrivateOrderStream => self.ws_private_stream_rows += 1,
            ExchangeWsEvidenceScope::Unknown => self.ws_unknown_scope_rows += 1,
        }
    }
}

fn rest_endpoint_has_recorded_evidence(endpoint: &RestEndpointRow) -> bool {
    [
        endpoint.checked_at.as_str(),
        endpoint.doc_version.as_str(),
        endpoint.schema_hash.as_str(),
        endpoint.fixture_id.as_str(),
        endpoint.parser_test.as_str(),
        endpoint.request_builder_test.as_str(),
        endpoint.auth_kind.as_str(),
    ]
    .into_iter()
    .all(|value| value != UNRECORDED_EVIDENCE_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExchangeWsOperationRegistryRow, ExchangeWsOperationVenue, ExchangeWsReleaseStatus,
        ExchangeWsSupportStatus, RestEndpointRow, RestEndpointVenue,
    };

    #[test]
    fn transport_registry_response_summarizes_evidence_boundaries() {
        let rest = RestEndpointsResponse {
            venues: vec![RestEndpointVenue {
                venue: "binance".to_owned(),
                endpoints: vec![
                    rest_operation_endpoint("/fapi/v1/order", "trade_write", "order_ack"),
                    rest_operation_endpoint("/fapi/v1/get-order", "private_read", "order_status"),
                    rest_operation_endpoint("/fapi/v3/balance", "private_read", "account_balance"),
                    rest_operation_endpoint(
                        "/fapi/v3/positionRisk",
                        "private_read",
                        "account_position",
                    ),
                    rest_endpoint("/fapi/v1/order"),
                    rest_endpoint("/fapi/v1/time"),
                ],
            }],
        };
        let websocket = ExchangeWsOperationsResponse {
            venues: vec![ExchangeWsOperationVenue {
                venue: "binance".to_owned(),
                label: "Binance".to_owned(),
                operations: vec![
                    ws_row("place_order", ExchangeWsEvidenceScope::AckOnly),
                    ws_row("order_status", ExchangeWsEvidenceScope::OrderStatusRead),
                    ws_row("order_stream", ExchangeWsEvidenceScope::PrivateOrderStream),
                    ws_row("close_position", ExchangeWsEvidenceScope::Unknown),
                ],
            }],
        };

        let response = ExchangeTransportRegistryResponse::new(rest, websocket);

        assert_eq!(1, response.summary.rest_venue_count);
        assert_eq!(6, response.summary.rest_endpoint_count);
        assert_eq!(1, response.summary.rest_operation_matrix_venue_count);
        assert_eq!(4, response.summary.rest_operation_matrix_bucket_count);
        assert_eq!(
            4,
            response.summary.rest_operation_matrix_recorded_bucket_count
        );
        assert_eq!(
            0,
            response.summary.rest_operation_matrix_missing_bucket_count
        );
        assert_eq!(1, response.summary.ws_venue_count);
        assert_eq!(4, response.summary.ws_operation_count);
        assert_eq!(1, response.summary.ws_ack_only_rows);
        assert_eq!(1, response.summary.ws_order_status_read_rows);
        assert_eq!(1, response.summary.ws_private_stream_rows);
        assert_eq!(1, response.summary.ws_unknown_scope_rows);
        assert_eq!(1, response.summary.ws_close_position_rows);
    }

    #[test]
    fn transport_registry_summary_marks_unrecorded_rest_operation_buckets_missing() {
        let rest = RestEndpointsResponse {
            venues: vec![RestEndpointVenue {
                venue: "binance".to_owned(),
                endpoints: vec![
                    rest_operation_endpoint("/fapi/v1/order", "trade_write", "order_ack"),
                    rest_endpoint_with_bucket("/fapi/v1/get-order", "private_read", "order_status"),
                ],
            }],
        };
        let websocket = ExchangeWsOperationsResponse { venues: vec![] };

        let response = ExchangeTransportRegistryResponse::new(rest, websocket);

        assert_eq!(0, response.summary.rest_operation_matrix_venue_count);
        assert_eq!(4, response.summary.rest_operation_matrix_bucket_count);
        assert_eq!(
            1,
            response.summary.rest_operation_matrix_recorded_bucket_count
        );
        assert_eq!(
            3,
            response.summary.rest_operation_matrix_missing_bucket_count
        );
    }

    #[test]
    fn transport_registry_summary_requires_full_rest_operation_evidence_metadata() {
        let mut missing_checked_at =
            rest_operation_endpoint("/fapi/v1/order", "trade_write", "order_ack");
        missing_checked_at.checked_at = UNRECORDED_EVIDENCE_MARKER.to_owned();
        let mut missing_doc_version =
            rest_operation_endpoint("/fapi/v1/get-order", "private_read", "order_status");
        missing_doc_version.doc_version = UNRECORDED_EVIDENCE_MARKER.to_owned();
        let mut missing_request_builder =
            rest_operation_endpoint("/fapi/v3/positionRisk", "private_read", "account_position");
        missing_request_builder.request_builder_test = UNRECORDED_EVIDENCE_MARKER.to_owned();
        let rest = RestEndpointsResponse {
            venues: vec![RestEndpointVenue {
                venue: "binance".to_owned(),
                endpoints: vec![
                    missing_checked_at,
                    missing_doc_version,
                    rest_operation_endpoint("/fapi/v3/balance", "private_read", "account_balance"),
                    missing_request_builder,
                ],
            }],
        };
        let response = ExchangeTransportRegistryResponse::new(
            rest,
            ExchangeWsOperationsResponse { venues: vec![] },
        );

        assert_eq!(0, response.summary.rest_operation_matrix_venue_count);
        assert_eq!(4, response.summary.rest_operation_matrix_bucket_count);
        assert_eq!(
            1,
            response.summary.rest_operation_matrix_recorded_bucket_count
        );
        assert_eq!(
            3,
            response.summary.rest_operation_matrix_missing_bucket_count
        );
    }

    fn rest_operation_endpoint(path: &str, use_case: &str, data_kind: &str) -> RestEndpointRow {
        let mut endpoint = rest_endpoint_with_bucket(path, use_case, data_kind);
        endpoint.schema_hash = "sha256:test".to_owned();
        endpoint.fixture_id = format!("{path}.json");
        endpoint.parser_test = "parser_test".to_owned();
        endpoint.request_builder_test = "request_builder_test".to_owned();
        endpoint.auth_kind = "signed".to_owned();
        endpoint
    }

    fn rest_endpoint_with_bucket(path: &str, use_case: &str, data_kind: &str) -> RestEndpointRow {
        let mut endpoint = rest_endpoint(path);
        endpoint.use_cases = vec![use_case.to_owned()];
        endpoint.data_kinds = vec![data_kind.to_owned()];
        endpoint
    }

    fn rest_endpoint(path: &str) -> RestEndpointRow {
        RestEndpointRow {
            method: "GET".to_owned(),
            path: path.to_owned(),
            weight: 1,
            checked_at: "2026-07-08".to_owned(),
            doc_version: "test".to_owned(),
            schema_hash: "not_recorded".to_owned(),
            fixture_id: "not_recorded".to_owned(),
            parser_test: "not_recorded".to_owned(),
            request_builder_test: "request_test".to_owned(),
            auth_kind: "none".to_owned(),
            doc_urls: vec!["https://example.test".to_owned()],
            use_cases: vec!["baseline".to_owned()],
            data_kinds: vec!["metadata".to_owned()],
            rate_scopes: vec!["public".to_owned()],
        }
    }

    fn ws_row(
        label: &str,
        evidence_scope: ExchangeWsEvidenceScope,
    ) -> ExchangeWsOperationRegistryRow {
        ExchangeWsOperationRegistryRow {
            label: label.to_owned(),
            evidence_scope,
            supported: true,
            status: ExchangeWsSupportStatus::Ready,
            release_status: ExchangeWsReleaseStatus::ProductionReady,
            requires_authenticated_runtime_evidence: false,
            authenticated_runtime_evidence: false,
            operation: Some(label.to_owned()),
            product: "USD-M".to_owned(),
            note: "test".to_owned(),
            checked_at: "2026-07-08".to_owned(),
            doc_version: "test".to_owned(),
            doc_url: "https://example.test".to_owned(),
            parser_test: Some("parser_test".to_owned()),
            subscription_test: Some("subscription_test".to_owned()),
            fixture_id: None,
            fixture_hash: None,
            auth_kind: "login".to_owned(),
        }
    }
}
