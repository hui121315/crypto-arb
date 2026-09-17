use serde::{Deserialize, Serialize};

pub const WEBHOOK_EVENT_VERSION: &str = "2026-07-01";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookProvider {
    #[default]
    Generic,
    Bark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventKind {
    Opportunity,
    OpportunityMonitor,
    AutomationDecision,
    ExecutionResult,
    Compensation,
    RiskAlert,
    SystemDegradation,
    OnchainSpread,
    StockSpread,
    Test,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookConfig {
    pub enabled: bool,
    #[serde(default)]
    pub provider: WebhookProvider,
    /// Redacted display value only. The delivery target is never returned by the API.
    pub url: String,
    #[serde(default)]
    pub url_configured: bool,
    pub secret_configured: bool,
    pub event_kinds: Vec<WebhookEventKind>,
    pub timeout_ms: u64,
    pub max_attempts: u8,
    pub base_backoff_ms: u64,
    pub queue_capacity: usize,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: WebhookProvider::Generic,
            url: String::new(),
            url_configured: false,
            secret_configured: false,
            event_kinds: vec![
                WebhookEventKind::Opportunity,
                WebhookEventKind::OpportunityMonitor,
                WebhookEventKind::AutomationDecision,
                WebhookEventKind::ExecutionResult,
                WebhookEventKind::Compensation,
                WebhookEventKind::RiskAlert,
                WebhookEventKind::SystemDegradation,
                WebhookEventKind::OnchainSpread,
                WebhookEventKind::StockSpread,
            ],
            timeout_ms: 15_000,
            max_attempts: 3,
            base_backoff_ms: 500,
            queue_capacity: 128,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookConfigPatch {
    pub enabled: Option<bool>,
    pub provider: Option<WebhookProvider>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub clear_secret: Option<bool>,
    pub event_kinds: Option<Vec<WebhookEventKind>>,
    pub timeout_ms: Option<u64>,
    pub max_attempts: Option<u8>,
    pub base_backoff_ms: Option<u64>,
    pub queue_capacity: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookTestRequest {
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookEvent {
    pub id: String,
    pub version: String,
    pub kind: WebhookEventKind,
    pub occurred_at_ms: i64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookDeliveryStatus {
    Queued,
    Delivered,
    Failed,
    Dropped,
    Disabled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookApplicationAck {
    Accepted,
    TransportOnly,
    Rejected,
    InvalidResponse,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDeliveryRecord {
    pub event_id: String,
    pub kind: WebhookEventKind,
    #[serde(default)]
    pub provider: WebhookProvider,
    pub status: WebhookDeliveryStatus,
    pub attempts: u8,
    pub response_status: Option<u16>,
    #[serde(default)]
    pub application_ack: WebhookApplicationAck,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_message: Option<String>,
    pub error: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookRuntimeStatus {
    pub config: WebhookConfig,
    pub queue_depth: usize,
    pub delivered_total: u64,
    pub failed_total: u64,
    pub dropped_total: u64,
    pub recent_deliveries: Vec<WebhookDeliveryRecord>,
    pub updated_at_ms: i64,
}
