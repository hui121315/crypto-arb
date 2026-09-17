//! REST endpoint 官方证据注册表 DTO。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestEndpointsResponse {
    pub venues: Vec<RestEndpointVenue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestEndpointVenue {
    pub venue: String,
    pub endpoints: Vec<RestEndpointRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestEndpointRow {
    pub method: String,
    pub path: String,
    pub weight: u32,
    pub checked_at: String,
    pub doc_version: String,
    pub schema_hash: String,
    pub fixture_id: String,
    pub parser_test: String,
    pub request_builder_test: String,
    pub auth_kind: String,
    pub doc_urls: Vec<String>,
    pub use_cases: Vec<String>,
    pub data_kinds: Vec<String>,
    pub rate_scopes: Vec<String>,
}
