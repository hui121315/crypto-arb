//! Venue-scoped order compiler and runtime evidence capabilities.

use crate::enums::OrderType;
use crate::fees::FeeProduct;
use crate::hedge::{
    MarginMode, OrderPayloadPricePolicy, TimeInForce, VenueMarketOrderStyle, VenueOrderKind,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOrderCapability {
    pub requested_order_type: OrderType,
    pub effective_order_type: OrderType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub time_in_force: Vec<TimeInForce>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub market_order_styles: Vec<VenueMarketOrderStyle>,
    pub venue_order_kind: VenueOrderKind,
    pub payload_price_policy: OrderPayloadPricePolicy,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueAccountCapability {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_margin_modes: Vec<MarginMode>,
    pub account_mode_scope: String,
    pub runtime_account_mode_read: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueClientOrderIdCapability {
    pub venue_field: String,
    pub policy_version: String,
    pub official_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    pub supports_query_by_client_id: bool,
    pub supports_cancel_by_client_id: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueInstrumentCapability {
    pub native_symbol_required: bool,
    pub instrument_spec_required: bool,
    pub native_sizing_required: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueFinalityCapability {
    pub ack_is_final: bool,
    pub private_order_stream: bool,
    pub private_fill_stream: bool,
    pub order_status_read: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_sources: Vec<String>,
}

impl VenueFinalityCapability {
    #[must_use]
    pub const fn has_confirmed_path(&self) -> bool {
        self.private_order_stream || self.private_fill_stream || self.order_status_read
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCapabilityMatrix {
    pub venue: String,
    #[serde(default)]
    pub product: FeeProduct,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub orders: Vec<VenueOrderCapability>,
    #[serde(default)]
    pub account: VenueAccountCapability,
    #[serde(default)]
    pub client_order_id: VenueClientOrderIdCapability,
    #[serde(default)]
    pub instrument: VenueInstrumentCapability,
    #[serde(default)]
    pub finality: VenueFinalityCapability,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub official_doc_urls: Vec<String>,
}

impl VenueCapabilityMatrix {
    #[must_use]
    pub fn order(&self, order_type: OrderType) -> Option<&VenueOrderCapability> {
        self.orders
            .iter()
            .find(|capability| capability.requested_order_type == order_type)
    }
}
