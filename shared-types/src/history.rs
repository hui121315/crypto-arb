//! Historical data response contracts.

use crate::{
    arbitrage::ArbitrageOpportunityDto,
    index_composition::{IndexCompositionQuality, IndexCompositionSnapshot},
    list::RowCapEvidence,
    ApiProblem, VenueOperationHealth,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingRow {
    pub occurred_at_ms: i64,
    pub exchange: String,
    pub symbol: String,
    pub rate: f64,
    pub interval_hours: u32,
    pub next_funding_ms: i64,
    pub volume_24h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingDiffRow {
    pub occurred_at_ms: i64,
    pub symbol: String,
    pub long_exchange: String,
    pub short_exchange: String,
    pub long_rate_8h: f64,
    pub short_rate_8h: f64,
    pub gross_diff_bps: f64,
    pub long_next_funding_ms: i64,
    pub short_next_funding_ms: i64,
    pub window_alignment_minutes: i32,
    pub long_interval_hours: u32,
    pub short_interval_hours: u32,
    pub min_volume_24h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityHistoryRow {
    pub occurred_at_ms: i64,
    pub id: String,
    pub symbol: String,
    pub long_exchange: String,
    pub short_exchange: String,
    pub spread_8h: f64,
    pub net_yield: f64,
    pub volume_24h_min: f64,
    pub payload: ArbitrageOpportunityDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexCompositionHistoryRow {
    pub occurred_at_ms: i64,
    pub venue: String,
    pub symbol: String,
    pub index_id: String,
    pub quality: IndexCompositionQuality,
    pub component_count: usize,
    pub source: String,
    pub payload: IndexCompositionSnapshot,
}

/// Per venue/endpoint API health sample (status / error / latency / retry-after
/// / circuit) so Prometheus and the settings page can localise which venue and
/// endpoint is slow, erroring, or rate-limited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiHealthSampleRow {
    pub occurred_at_ms: i64,
    pub exchange: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

/// Unified audit/event ledger row threaded with the full correlation chain
/// (`request_id` / `run_id` / `ticket_id` / `client_order_id` /
/// `exchange_order_id`) so every high-risk operation leaves a durable,
/// replayable, joinable record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEventRow {
    pub occurred_at_ms: i64,
    pub event_id: String,
    pub category: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioNavHistoryRow {
    pub occurred_at_ms: i64,
    pub nav_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub limit: usize,
    pub max_limit: usize,
    pub returned_count: usize,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl HistoryPage {
    pub fn row_cap(&self, source: impl Into<String>) -> RowCapEvidence {
        let total_rows = self
            .returned_count
            .saturating_add(usize::from(self.has_more));
        if self.has_more {
            RowCapEvidence::lower_bound(self.limit, self.returned_count, total_rows, source)
        } else {
            RowCapEvidence::exact(self.limit, self.returned_count, total_rows, source)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryTimescaleStatus {
    NotApplicable,
    Enabled,
    PlainPostgres,
    Partial,
}

pub type HistoryMigrationStatus = crate::StorageMigrationAuthority;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryBackendStatus {
    pub backend: String,
    #[serde(default)]
    pub storage_contract: crate::StorageRuntimeContract,
    pub enabled: bool,
    pub durable: bool,
    pub fallback: bool,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_status: Option<HistoryMigrationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timescale_status: Option<HistoryTimescaleStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timescale_problem: Option<String>,
    pub append_success_total: u64,
    pub append_error_total: u64,
    pub query_success_total: u64,
    pub query_error_total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_append_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_query_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
    pub observed_at_ms: i64,
}

impl Default for HistoryBackendStatus {
    fn default() -> Self {
        Self {
            backend: "unknown".to_owned(),
            storage_contract: crate::StorageRuntimeContract::default(),
            enabled: false,
            durable: false,
            fallback: false,
            ephemeral: false,
            schema_version: None,
            migration_checksum: None,
            migration_status: None,
            startup_problem: None,
            timescale_status: None,
            timescale_problem: None,
            append_success_total: 0,
            append_error_total: 0,
            query_success_total: 0,
            query_error_total: 0,
            last_success_at_ms: None,
            last_append_at_ms: None,
            last_query_at_ms: None,
            last_error_at_ms: None,
            last_error: None,
            last_error_code: None,
            observed_at_ms: 0,
        }
    }
}

impl HistoryBackendStatus {
    pub fn success_total(&self) -> u64 {
        self.append_success_total
            .saturating_add(self.query_success_total)
    }

    pub fn error_total(&self) -> u64 {
        self.append_error_total
            .saturating_add(self.query_error_total)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResponse<T> {
    pub count: usize,
    pub rows: Vec<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<HistoryPage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_cap: Option<RowCapEvidence>,
    #[serde(default)]
    pub backend_status: HistoryBackendStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_health: Option<VenueOperationHealth>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_response_serializes_health_fields() {
        let response = serialized_health_response();

        let text = serde_json::to_string(&response).expect("serialize history response");

        assert_history_envelope_text(&text);
        assert_history_backend_text(&text);
        assert_history_storage_text(&text);
    }

    fn assert_history_envelope_text(text: &str) {
        assert!(text.contains("\"source\":\"memory\""));
        assert!(text.contains("\"occurredAtMs\":1000"));
        assert!(text.contains("\"navUsd\":101.5"));
        assert!(text.contains("\"observedAtMs\":1100"));
        assert!(text.contains("\"freshnessMs\":100"));
        assert!(text.contains("\"retryAfterMs\":2000"));
        assert!(text.contains("\"hasMore\":true"));
        assert!(text.contains("\"nextCursor\":\"v1:900\""));
        assert!(text.contains("\"rowCap\""));
        assert!(text.contains("\"totalRowsIsLowerBound\":true"));
        assert!(text.contains("\"problems\""));
    }

    #[test]
    fn history_page_row_cap_uses_lower_bound_when_has_more() {
        let page = HistoryPage {
            limit: 24,
            max_limit: 1_000,
            returned_count: 24,
            has_more: true,
            next_cursor: Some("v1:900".into()),
        };

        let cap = page.row_cap("history");

        assert_eq!(cap.max_rows, 24);
        assert_eq!(cap.returned_count, 24);
        assert_eq!(cap.total_rows, 25);
        assert!(cap.total_rows_is_lower_bound);
        assert!(cap.truncated);
        assert_eq!(cap.truncated_count, 1);
    }

    fn assert_history_backend_text(text: &str) {
        assert!(text.contains("\"backendStatus\""));
        assert!(text.contains("\"backend\":\"memory\""));
        assert!(text.contains("\"storageContract\""));
        assert!(text.contains("\"backendKind\":\"memory\""));
        assert!(text.contains("\"degradedReasons\":[\"ephemeral\"]"));
        assert!(text.contains("\"migrationAuthority\""));
        assert!(text.contains("\"fallback\":true"));
        assert!(text.contains("\"ephemeral\":true"));
        assert!(text.contains("\"schemaVersion\":1"));
        assert!(text.contains("\"migrationChecksum\":\"history-v1-test\""));
        assert_history_backend_migration_text(text);
        assert_history_backend_runtime_text(text);
    }

    fn assert_history_backend_migration_text(text: &str) {
        assert!(text.contains("\"migrationStatus\""));
        assert!(text.contains("\"migrationId\":\"20260701_history\""));
        assert!(text.contains("\"applied\":true"));
        assert!(text.contains("\"appliedAtMs\":875"));
    }

    fn assert_history_backend_runtime_text(text: &str) {
        assert!(text.contains("\"timescaleStatus\":\"plain_postgres\""));
        assert!(text.contains("\"timescaleProblem\":\"timescaledb unavailable\""));
        assert!(text.contains("\"lastAppendAtMs\":950"));
        assert!(text.contains("\"lastQueryAtMs\":1000"));
        assert!(text.contains("\"lastErrorCode\":\"HISTORY_STORE_UNAVAILABLE\""));
    }

    fn assert_history_storage_text(text: &str) {
        assert!(text.contains("\"storageHealth\""));
        assert!(text.contains("\"operation\":\"storage:history\""));
    }

    fn serialized_health_response() -> HistoryResponse<PortfolioNavHistoryRow> {
        HistoryResponse {
            count: 1,
            rows: vec![PortfolioNavHistoryRow {
                occurred_at_ms: 1_000,
                nav_usd: 101.5,
            }],
            page: Some(serialized_health_page()),
            row_cap: Some(serialized_health_page().row_cap("memory")),
            backend_status: serialized_backend_status(),
            storage_health: Some(serialized_storage_health()),
            source: "memory".into(),
            observed_at_ms: 1_100,
            latest_at_ms: Some(1_000),
            freshness_ms: Some(100),
            problem: Some(ApiProblem::new("HISTORY_STORE_UNAVAILABLE", "disabled")),
            retry_after_ms: Some(2_000),
            problems: vec![ApiProblem::new("HISTORY_STORE_UNAVAILABLE", "disabled")],
        }
    }

    fn serialized_health_page() -> HistoryPage {
        HistoryPage {
            limit: 50,
            max_limit: 1_000,
            returned_count: 0,
            has_more: true,
            next_cursor: Some("v1:900".into()),
        }
    }

    fn serialized_backend_status() -> HistoryBackendStatus {
        let migration = HistoryMigrationStatus {
            migration_id: "20260701_history".into(),
            schema_name: "realtime_history".into(),
            migration_path: "crates/realtime/migrations/20260701_history.sql".into(),
            schema_version: Some(1),
            migration_checksum: Some("history-v1-test".into()),
            applied: true,
            applied_at_ms: Some(875),
        };
        HistoryBackendStatus {
            backend: "memory".into(),
            storage_contract: crate::StorageRuntimeContract {
                backend_kind: crate::StorageBackendKind::Memory,
                degraded_reasons: vec![crate::StorageDegradedReason::Ephemeral],
                migration_authority: Some(migration.clone()),
            },
            enabled: true,
            durable: false,
            fallback: true,
            ephemeral: true,
            schema_version: Some(1),
            migration_checksum: Some("history-v1-test".into()),
            migration_status: Some(migration),
            startup_problem: Some("postgres connect failed".into()),
            timescale_status: Some(HistoryTimescaleStatus::PlainPostgres),
            timescale_problem: Some("timescaledb unavailable".into()),
            append_success_total: 2,
            append_error_total: 1,
            query_success_total: 3,
            query_error_total: 0,
            last_success_at_ms: Some(1_000),
            last_append_at_ms: Some(950),
            last_query_at_ms: Some(1_000),
            last_error_at_ms: Some(900),
            last_error: Some("temporary outage".into()),
            last_error_code: Some("HISTORY_STORE_UNAVAILABLE".into()),
            observed_at_ms: 1_100,
        }
    }

    fn serialized_storage_health() -> VenueOperationHealth {
        VenueOperationHealth {
            venue: "system".into(),
            operation: "storage:history".into(),
            status: crate::VenueOperationStatus::Warn,
            source: "history_store".into(),
            message: "history store is ephemeral".into(),
            supported: Some(true),
            configured: Some(true),
            requested: Some(5),
            rows: Some(5),
            freshness_ms: Some(100),
            retry_after_ms: None,
            latency_ms: None,
            latency_p95_ms: None,
            error: Some("history store is ephemeral".into()),
            evidence: None,
            problem: None,
            observed_at_ms: 1_000,
        }
    }

    #[test]
    fn history_response_deserializes_without_backend_status() {
        let response: HistoryResponse<u8> =
            serde_json::from_str(r#"{"count":0,"rows":[]}"#).expect("deserialize history response");

        assert_eq!(response.backend_status.backend, "unknown");
        assert!(!response.backend_status.enabled);
    }

    #[test]
    fn api_health_sample_round_trips_camel_case() {
        let row = ApiHealthSampleRow {
            occurred_at_ms: 1_700_000_000_000,
            exchange: "binance".into(),
            endpoint: "/fapi/v1/order".into(),
            method: Some("POST".into()),
            outcome: "rate_limited".into(),
            status_code: Some(429),
            latency_ms: Some(42.5),
            retry_after_ms: Some(2_000),
            circuit_state: Some("open".into()),
            error_code: Some("EXCHANGE_RATE_LIMITED".into()),
            payload: serde_json::json!({ "weight": 1200 }),
        };

        let text = serde_json::to_string(&row).expect("serialize api health sample");
        assert!(text.contains("\"occurredAtMs\":1700000000000"));
        assert!(text.contains("\"endpoint\":\"/fapi/v1/order\""));
        assert!(text.contains("\"statusCode\":429"));
        assert!(text.contains("\"retryAfterMs\":2000"));
        assert!(text.contains("\"circuitState\":\"open\""));

        let parsed: ApiHealthSampleRow = serde_json::from_str(&text).expect("deserialize sample");
        assert_eq!(parsed, row);
    }

    #[test]
    fn api_health_sample_omits_empty_optionals() {
        let row = ApiHealthSampleRow {
            occurred_at_ms: 1,
            exchange: "okx".into(),
            endpoint: "/api/v5/account/balance".into(),
            method: None,
            outcome: "ok".into(),
            status_code: Some(200),
            latency_ms: Some(8.0),
            retry_after_ms: None,
            circuit_state: None,
            error_code: None,
            payload: serde_json::Value::Null,
        };

        let text = serde_json::to_string(&row).expect("serialize api health sample");
        assert!(!text.contains("retryAfterMs"));
        assert!(!text.contains("circuitState"));
        assert!(!text.contains("payload"));
    }

    #[test]
    fn ledger_event_round_trips_correlation_ids() {
        let row = LedgerEventRow {
            occurred_at_ms: 1_700_000_000_001,
            event_id: "evt-1".into(),
            category: "trade".into(),
            action: "close_position".into(),
            actor: Some("operator".into()),
            resource: Some("ticket:abc".into()),
            outcome: "submitted".into(),
            severity: Some("high".into()),
            request_id: Some("req-1".into()),
            run_id: Some("run-1".into()),
            ticket_id: Some("ticket-1".into()),
            client_order_id: Some("cl-1".into()),
            exchange_order_id: Some("ex-1".into()),
            payload: serde_json::json!({ "qty": "0.5" }),
        };

        let text = serde_json::to_string(&row).expect("serialize ledger event");
        assert!(text.contains("\"eventId\":\"evt-1\""));
        assert!(text.contains("\"requestId\":\"req-1\""));
        assert!(text.contains("\"runId\":\"run-1\""));
        assert!(text.contains("\"ticketId\":\"ticket-1\""));
        assert!(text.contains("\"clientOrderId\":\"cl-1\""));
        assert!(text.contains("\"exchangeOrderId\":\"ex-1\""));

        let parsed: LedgerEventRow = serde_json::from_str(&text).expect("deserialize event");
        assert_eq!(parsed, row);
    }
}
