//! Shared contracts for the optional, externally routed LLM diagnostics surface.
//!
//! These types deliberately describe only data that may leave CROSSLINE. Raw
//! exchange responses, credentials, account identifiers, wallets, and free-form
//! chat transcripts are not representable by this contract.

use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_LLM_EXTERNAL_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_SUMMARY_CHARS: usize = 4_096;
const MAX_LABEL_CHARS: usize = 128;
const MAX_ERROR_CODE_CHARS: usize = 96;
const MAX_ORDER_STATES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmPromptContext {
    OpportunityExplanation,
    FailureDiagnosis,
    DailyBrief,
}

impl LlmPromptContext {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpportunityExplanation => "opportunity_explanation",
            Self::FailureDiagnosis => "failure_diagnosis",
            Self::DailyBrief => "daily_brief",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProviderId {
    Openai,
    Claude,
    Gemini,
    Deepseek,
}

impl LlmProviderId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Deepseek => "deepseek",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmOrderState {
    Pending,
    Accepted,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmSanitizedOrderState {
    pub venue: String,
    pub symbol: String,
    pub state: LlmOrderState,
}

/// An explicit allowlist of information that can be sent to a configured LLM
/// provider. The caller must supply a sanitized summary; the boundary rejects
/// sensitive markers instead of attempting to recover a raw payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmExternalPayload {
    pub context: LlmPromptContext,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_states: Vec<LlmSanitizedOrderState>,
}

impl LlmExternalPayload {
    /// Validates and serializes the only payload representation that may cross
    /// the provider boundary.
    pub fn outbound_json(&self) -> Result<String, LlmExternalPayloadError> {
        self.validate()?;
        let json = serde_json::to_string(self).map_err(|_| LlmExternalPayloadError::Encode)?;
        if json.len() > MAX_LLM_EXTERNAL_PAYLOAD_BYTES {
            return Err(LlmExternalPayloadError::TooLarge {
                bytes: json.len(),
                max_bytes: MAX_LLM_EXTERNAL_PAYLOAD_BYTES,
            });
        }
        Ok(json)
    }

    pub fn validate(&self) -> Result<(), LlmExternalPayloadError> {
        validate_summary(&self.summary)?;
        validate_optional_label(self.symbol.as_deref(), "symbol")?;
        validate_optional_label(self.venue.as_deref(), "venue")?;
        validate_error_code(self.error_code.as_deref())?;
        if self.order_states.len() > MAX_ORDER_STATES {
            return Err(LlmExternalPayloadError::TooManyOrderStates);
        }
        for state in &self.order_states {
            validate_required_label(&state.venue, "order state venue")?;
            validate_required_label(&state.symbol, "order state symbol")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmExternalRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<LlmProviderId>,
    pub payload: LlmExternalPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmExternalResponse {
    pub content: String,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmExternalPayloadError {
    EmptySummary,
    SummaryTooLong,
    LabelInvalid(&'static str),
    ErrorCodeInvalid,
    SensitiveContent,
    TooManyOrderStates,
    TooLarge { bytes: usize, max_bytes: usize },
    Encode,
}

impl LlmExternalPayloadError {
    pub const fn is_too_large(&self) -> bool {
        matches!(self, Self::TooLarge { .. })
    }
}

impl fmt::Display for LlmExternalPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySummary => f.write_str("summary must not be empty"),
            Self::SummaryTooLong => f.write_str("summary exceeds the external payload limit"),
            Self::LabelInvalid(field) => write!(f, "{field} is invalid for an external payload"),
            Self::ErrorCodeInvalid => f.write_str("error code is invalid for an external payload"),
            Self::SensitiveContent => f.write_str("external payload contains a sensitive marker"),
            Self::TooManyOrderStates => f.write_str("external payload has too many order states"),
            Self::TooLarge { bytes, max_bytes } => {
                write!(f, "external payload is {bytes} bytes; max is {max_bytes}")
            }
            Self::Encode => f.write_str("external payload encoding failed"),
        }
    }
}

impl std::error::Error for LlmExternalPayloadError {}

fn validate_summary(value: &str) -> Result<(), LlmExternalPayloadError> {
    if value.trim().is_empty() {
        return Err(LlmExternalPayloadError::EmptySummary);
    }
    if value.chars().count() > MAX_SUMMARY_CHARS {
        return Err(LlmExternalPayloadError::SummaryTooLong);
    }
    reject_sensitive_marker(value)
}

fn validate_optional_label(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), LlmExternalPayloadError> {
    if let Some(value) = value {
        validate_required_label(value, field)?;
    }
    Ok(())
}

fn validate_required_label(
    value: &str,
    field: &'static str,
) -> Result<(), LlmExternalPayloadError> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_LABEL_CHARS
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'_' | b'-' | b'/')
        })
    {
        return Err(LlmExternalPayloadError::LabelInvalid(field));
    }
    reject_sensitive_marker(value)
}

fn validate_error_code(value: Option<&str>) -> Result<(), LlmExternalPayloadError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty()
        || value.len() > MAX_ERROR_CODE_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(LlmExternalPayloadError::ErrorCodeInvalid);
    }
    Ok(())
}

fn reject_sensitive_marker(value: &str) -> Result<(), LlmExternalPayloadError> {
    const SENSITIVE_MARKERS: [&str; 18] = [
        "apikey",
        "api_key",
        "secret",
        "passphrase",
        "password",
        "token",
        "credential",
        "private_key",
        "privatekey",
        "authorization",
        "cookie",
        "mnemonic",
        "seed",
        "wallet",
        "account_id",
        "accountid",
        "address",
        "raw_order_response",
    ];
    let lowered = value.to_ascii_lowercase();
    if SENSITIVE_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return Err(LlmExternalPayloadError::SensitiveContent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_payload() -> LlmExternalPayload {
        LlmExternalPayload {
            context: LlmPromptContext::OpportunityExplanation,
            summary: "Basis narrowed after a verified orderbook refresh".to_owned(),
            symbol: Some("BTCUSDT".to_owned()),
            venue: Some("okx".to_owned()),
            error_code: None,
            order_states: vec![LlmSanitizedOrderState {
                venue: "okx".to_owned(),
                symbol: "BTCUSDT".to_owned(),
                state: LlmOrderState::Accepted,
            }],
        }
    }

    #[test]
    fn valid_payload_is_bounded_and_serializable() {
        let json = valid_payload().outbound_json().expect("valid payload");

        assert!(json.contains("opportunity_explanation"));
        assert!(json.len() <= MAX_LLM_EXTERNAL_PAYLOAD_BYTES);
        assert!(!json.contains("api_key"));
    }

    #[test]
    fn sensitive_markers_fail_closed_before_external_serialization() {
        let mut payload = valid_payload();
        payload.summary = "api_key=live-secret".to_owned();

        assert_eq!(
            payload.outbound_json(),
            Err(LlmExternalPayloadError::SensitiveContent)
        );
    }

    #[test]
    fn unknown_raw_fields_cannot_deserialize_into_allowlisted_contract() {
        let raw = r#"{
            "context":"failure_diagnosis",
            "summary":"order rejected",
            "apiKey":"not allowed"
        }"#;

        assert!(serde_json::from_str::<LlmExternalPayload>(raw).is_err());
    }

    #[test]
    fn multibyte_summary_respects_byte_cap() {
        let mut payload = valid_payload();
        payload.summary = "🚫".repeat(MAX_SUMMARY_CHARS);

        assert!(matches!(
            payload.outbound_json(),
            Err(LlmExternalPayloadError::TooLarge { .. })
        ));
    }
}
