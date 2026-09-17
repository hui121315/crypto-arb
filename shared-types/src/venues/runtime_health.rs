use super::{VenueOperationHealth, VenueOperationStatus};
use crate::ApiProblem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueCapabilityStatus {
    #[default]
    Unknown,
    Supported,
    Unsupported,
}

impl From<Option<bool>> for VenueCapabilityStatus {
    fn from(supported: Option<bool>) -> Self {
        match supported {
            Some(true) => Self::Supported,
            Some(false) => Self::Unsupported,
            None => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueConfigurationStatus {
    #[default]
    Unknown,
    NotRequired,
    Configured,
    NotConfigured,
}

impl VenueConfigurationStatus {
    const fn is_ready(self) -> bool {
        matches!(self, Self::NotRequired | Self::Configured)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueRuntimeOperation {
    PublicRest,
    PublicWs,
    PrivateRest,
    PrivateWs,
    Balance,
    Positions,
    OpenOrders,
    PlaceOrder,
    CancelOrder,
    OrderStream,
    Finality,
}

impl VenueRuntimeOperation {
    pub const fn requires_configuration(self) -> bool {
        !matches!(self, Self::PublicRest | Self::PublicWs)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueRuntimeOperationHealth {
    pub operation: VenueRuntimeOperation,
    pub status: VenueOperationStatus,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p95_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default)]
    pub capability_status: VenueCapabilityStatus,
    #[serde(default)]
    pub configuration_status: VenueConfigurationStatus,
    #[serde(default)]
    pub currently_usable: bool,
    pub observed_at_ms: i64,
}

impl VenueRuntimeOperationHealth {
    pub fn from_operation_health(
        operation: VenueRuntimeOperation,
        health: &VenueOperationHealth,
    ) -> Self {
        let capability_status = health.supported.into();
        let configuration_status = configuration_status(operation, health.configured);
        let currently_usable = health.status == VenueOperationStatus::Ok
            && capability_status == VenueCapabilityStatus::Supported
            && configuration_status.is_ready();

        Self {
            operation,
            status: health.status,
            source: health.source.clone(),
            freshness_ms: health.freshness_ms,
            latency_ms: health.latency_ms,
            latency_p95_ms: health.latency_p95_ms,
            requested: health.requested,
            rows: health.rows,
            last_success_ms: (health.status == VenueOperationStatus::Ok)
                .then_some(health.observed_at_ms),
            last_error: operation_last_error(health),
            retry_after_ms: operation_retry_after_ms(health),
            request_id: operation_request_id(health),
            problem: health.problem.clone(),
            capability_status,
            configuration_status,
            currently_usable,
            observed_at_ms: health.observed_at_ms,
        }
    }
}

fn configuration_status(
    operation: VenueRuntimeOperation,
    configured: Option<bool>,
) -> VenueConfigurationStatus {
    match configured {
        Some(true) => VenueConfigurationStatus::Configured,
        Some(false) => VenueConfigurationStatus::NotConfigured,
        None if !operation.requires_configuration() => VenueConfigurationStatus::NotRequired,
        None => VenueConfigurationStatus::Unknown,
    }
}

fn operation_last_error(health: &VenueOperationHealth) -> Option<String> {
    health.error.clone().or_else(|| {
        health
            .problem
            .as_ref()
            .map(|problem| problem.message.clone())
    })
}

fn operation_retry_after_ms(health: &VenueOperationHealth) -> Option<u64> {
    health
        .retry_after_ms
        .into_iter()
        .chain(
            health
                .problem
                .as_ref()
                .and_then(|problem| problem.retry_after_ms),
        )
        .max()
}

fn operation_request_id(health: &VenueOperationHealth) -> Option<String> {
    health
        .evidence
        .as_ref()
        .and_then(|evidence| evidence.request_id.clone())
        .or_else(|| {
            health
                .problem
                .as_ref()
                .and_then(|problem| problem.request_id.clone())
        })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueRuntimeHealth {
    pub venue: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_rest: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ws: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_rest: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_ws: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positions: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_orders: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_order: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_order: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_stream: Option<VenueRuntimeOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality: Option<VenueRuntimeOperationHealth>,
    pub generated_at_ms: i64,
}

impl VenueRuntimeHealth {
    pub fn new(venue: impl Into<String>, generated_at_ms: i64) -> Self {
        Self {
            venue: venue.into(),
            generated_at_ms,
            ..Self::default()
        }
    }

    pub fn operation(
        &self,
        operation: VenueRuntimeOperation,
    ) -> Option<&VenueRuntimeOperationHealth> {
        match operation {
            VenueRuntimeOperation::PublicRest => self.public_rest.as_ref(),
            VenueRuntimeOperation::PublicWs => self.public_ws.as_ref(),
            VenueRuntimeOperation::PrivateRest => self.private_rest.as_ref(),
            VenueRuntimeOperation::PrivateWs => self.private_ws.as_ref(),
            VenueRuntimeOperation::Balance => self.balance.as_ref(),
            VenueRuntimeOperation::Positions => self.positions.as_ref(),
            VenueRuntimeOperation::OpenOrders => self.open_orders.as_ref(),
            VenueRuntimeOperation::PlaceOrder => self.place_order.as_ref(),
            VenueRuntimeOperation::CancelOrder => self.cancel_order.as_ref(),
            VenueRuntimeOperation::OrderStream => self.order_stream.as_ref(),
            VenueRuntimeOperation::Finality => self.finality.as_ref(),
        }
    }

    pub fn set_operation(&mut self, health: VenueRuntimeOperationHealth) {
        match health.operation {
            VenueRuntimeOperation::PublicRest => self.public_rest = Some(health),
            VenueRuntimeOperation::PublicWs => self.public_ws = Some(health),
            VenueRuntimeOperation::PrivateRest => self.private_rest = Some(health),
            VenueRuntimeOperation::PrivateWs => self.private_ws = Some(health),
            VenueRuntimeOperation::Balance => self.balance = Some(health),
            VenueRuntimeOperation::Positions => self.positions = Some(health),
            VenueRuntimeOperation::OpenOrders => self.open_orders = Some(health),
            VenueRuntimeOperation::PlaceOrder => self.place_order = Some(health),
            VenueRuntimeOperation::CancelOrder => self.cancel_order = Some(health),
            VenueRuntimeOperation::OrderStream => self.order_stream = Some(health),
            VenueRuntimeOperation::Finality => self.finality = Some(health),
        }
    }

    pub fn operations(&self) -> impl Iterator<Item = &VenueRuntimeOperationHealth> {
        [
            self.public_rest.as_ref(),
            self.public_ws.as_ref(),
            self.private_rest.as_ref(),
            self.private_ws.as_ref(),
            self.balance.as_ref(),
            self.positions.as_ref(),
            self.open_orders.as_ref(),
            self.place_order.as_ref(),
            self.cancel_order.as_ref(),
            self.order_stream.as_ref(),
            self.finality.as_ref(),
        ]
        .into_iter()
        .flatten()
    }
}
