//! Generic paged list envelope shared by backend and frontend.

use crate::problem::ApiProblem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListStatus {
    #[default]
    Fresh,
    Degraded,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPage {
    pub limit: usize,
    pub max_limit: usize,
    pub start_offset: usize,
    pub returned_count: usize,
    pub total_rows: usize,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowCapEvidence {
    pub max_rows: usize,
    pub returned_count: usize,
    pub total_rows: usize,
    #[serde(default)]
    pub total_rows_is_lower_bound: bool,
    pub truncated: bool,
    pub truncated_count: usize,
    pub source: String,
}

impl RowCapEvidence {
    pub fn exact(
        max_rows: usize,
        returned_count: usize,
        total_rows: usize,
        source: impl Into<String>,
    ) -> Self {
        let truncated_count = total_rows.saturating_sub(returned_count);
        Self {
            max_rows,
            returned_count,
            total_rows,
            total_rows_is_lower_bound: false,
            truncated: truncated_count > 0,
            truncated_count,
            source: source.into(),
        }
    }

    pub fn lower_bound(
        max_rows: usize,
        returned_count: usize,
        total_rows_lower_bound: usize,
        source: impl Into<String>,
    ) -> Self {
        let truncated_count = total_rows_lower_bound.saturating_sub(returned_count);
        Self {
            max_rows,
            returned_count,
            total_rows: total_rows_lower_bound,
            total_rows_is_lower_bound: true,
            truncated: truncated_count > 0,
            truncated_count,
            source: source.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEnvelope<T> {
    pub rows: Vec<T>,
    pub page: ListPage,
    pub status: ListStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
}

impl<T> ListEnvelope<T> {
    pub fn new(
        rows: Vec<T>,
        page: ListPage,
        status: ListStatus,
        source: impl Into<String>,
        observed_at_ms: i64,
        problems: Vec<ApiProblem>,
    ) -> Self {
        Self {
            rows,
            page,
            status,
            source: source.into(),
            observed_at_ms,
            problems,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_envelope_serializes_camel_case_page_and_snake_status() {
        let envelope = ListEnvelope::new(
            Vec::<u8>::new(),
            ListPage {
                limit: 50,
                max_limit: 100,
                start_offset: 10,
                returned_count: 0,
                total_rows: 10,
                has_more: false,
                previous_cursor: None,
                next_cursor: None,
                last_cursor: None,
                snapshot_id: None,
            },
            ListStatus::Degraded,
            "order_journal",
            1_000,
            vec![ApiProblem::new("LIST_LIMIT_CLAMPED", "limit clamped")],
        );

        let text = serde_json::to_string(&envelope).expect("serialize list envelope");

        assert!(text.contains("\"startOffset\":10"));
        assert!(text.contains("\"maxLimit\":100"));
        assert!(text.contains("\"observedAtMs\":1000"));
        assert!(text.contains("\"status\":\"degraded\""));
        assert!(text.contains("\"problems\""));
    }

    #[test]
    fn row_cap_evidence_distinguishes_exact_and_lower_bound_totals() {
        let exact = RowCapEvidence::exact(24, 24, 30, "detail-history");
        let lower = RowCapEvidence::lower_bound(24, 24, 25, "history-page");

        assert!(exact.truncated);
        assert_eq!(exact.truncated_count, 6);
        assert!(!exact.total_rows_is_lower_bound);
        assert!(lower.truncated);
        assert_eq!(lower.truncated_count, 1);
        assert!(lower.total_rows_is_lower_bound);
    }

    #[test]
    fn list_page_accepts_legacy_payload_without_navigation_cursors() {
        let page: ListPage = serde_json::from_str(
            r#"{"limit":50,"maxLimit":100,"startOffset":0,"returnedCount":0,"totalRows":0,"hasMore":false}"#,
        )
        .unwrap_or_else(|error| panic!("deserialize legacy ListPage: {error}"));

        assert!(page.previous_cursor.is_none());
        assert!(page.next_cursor.is_none());
        assert!(page.last_cursor.is_none());
        assert!(page.snapshot_id.is_none());
    }
}
