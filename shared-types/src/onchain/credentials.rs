use serde::{Deserialize, Serialize};

use crate::{SecretStorageStatus, VenueCredentialField, VenueCredentialValue};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainProviderCredentialStatus {
    pub provider: String,
    pub label: String,
    pub official_docs_url: String,
    pub fields: Vec<VenueCredentialField>,
    pub missing_fields: Vec<String>,
    pub configured_count: usize,
    pub field_count: usize,
    pub ready: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainProviderCredentialsResponse {
    pub providers: Vec<OnchainProviderCredentialStatus>,
    #[serde(default)]
    pub secret_storage: SecretStorageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainProviderCredentialUpdateRequest {
    pub provider: String,
    pub fields: Vec<VenueCredentialValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainProviderCredentialClearRequest {
    pub provider: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainProviderCredentialMutationResponse {
    pub provider: String,
    pub label: String,
    pub configured_count: usize,
    pub field_count: usize,
    pub affected_fields: Vec<String>,
    pub missing_fields: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub secret_storage: SecretStorageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_status_serialization_never_contains_secret_values() {
        let response = OnchainProviderCredentialsResponse {
            providers: vec![OnchainProviderCredentialStatus {
                provider: "zeroex_swap_v2".to_owned(),
                label: "0x Swap API V2".to_owned(),
                official_docs_url: "https://docs.0x.org/".to_owned(),
                fields: vec![VenueCredentialField {
                    key: "api_key".to_owned(),
                    label: "API Key".to_owned(),
                    env_key: "ZEROX_API_KEY".to_owned(),
                    configured: true,
                    secret: true,
                    required: true,
                    source: crate::VenueCredentialFieldSource::Keychain,
                }],
                missing_fields: Vec::new(),
                configured_count: 1,
                field_count: 1,
                ready: true,
                note: "status only".to_owned(),
            }],
            secret_storage: SecretStorageStatus::runtime_only(),
        };

        let encoded = serde_json::to_string(&response).expect("provider status serializes");

        assert!(encoded.contains("ZEROX_API_KEY"));
        assert!(!encoded.contains("super-secret-value"));
        assert!(!encoded.contains("\"value\""));
    }
}
