//! 执行事实账本 DTO。

use crate::enums::OrderSide;
use crate::hedge::HedgeLegRole;
use crate::live_trading::{LiveOrderState, OrderUpdateSource, VenueOrderIdentity};
use crate::market::MarketDataHealth;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLedgerEventType {
    OrderState,
    FillSnapshot,
    FillEvent,
    FeeSnapshot,
    FundingPayment,
    Slippage,
    OrderbookEvidence,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLedgerQuality {
    Actual,
    Estimated,
    Missing,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFillConfidence {
    VenueFill,
    VenueOrderSnapshot,
    OrderQuery,
    AdapterAck,
    Manual,
    #[default]
    Unknown,
}

impl ExecutionFillConfidence {
    pub const fn score(self) -> f64 {
        match self {
            Self::VenueFill => 1.0,
            Self::VenueOrderSnapshot => 0.9,
            Self::OrderQuery => 0.85,
            Self::AdapterAck => 0.65,
            Self::Manual => 0.4,
            Self::Unknown => 0.0,
        }
    }

    pub const fn from_source(
        event_type: ExecutionLedgerEventType,
        source: OrderUpdateSource,
    ) -> Self {
        match (event_type, source) {
            (ExecutionLedgerEventType::FillEvent, OrderUpdateSource::PrivateWs) => Self::VenueFill,
            (ExecutionLedgerEventType::FillSnapshot, OrderUpdateSource::PrivateWs) => {
                Self::VenueOrderSnapshot
            }
            (_, OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile) => Self::OrderQuery,
            (_, OrderUpdateSource::AdapterAck) => Self::AdapterAck,
            (_, OrderUpdateSource::Manual) => Self::Manual,
            (_, OrderUpdateSource::FundingPoller) => Self::Unknown,
            _ => Self::Unknown,
        }
    }

    pub const fn supports_terminal_fill(self) -> bool {
        matches!(
            self,
            Self::VenueFill | Self::VenueOrderSnapshot | Self::OrderQuery
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionLedgerOrderRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leg_role: Option<HedgeLegRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<bool>,
    pub exchange: String,
    pub symbol: String,
    pub side: OrderSide,
    pub identity: VenueOrderIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillLedgerSnapshot {
    pub quantity: f64,
    pub average_price: f64,
    pub quote_value: f64,
    pub quality: ExecutionLedgerQuality,
    #[serde(default)]
    pub confidence: ExecutionFillConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee: Option<FeeLedgerSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeLedgerSnapshot {
    pub amount: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    pub quality: ExecutionLedgerQuality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingPaymentLedgerRecord {
    pub amount: f64,
    pub currency: String,
    pub funding_time_ms: i64,
    pub quality: ExecutionLedgerQuality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlippageLedgerRecord {
    pub amount_usd: f64,
    pub reference_price: f64,
    pub fill_price: f64,
    pub quantity: f64,
    pub quality: ExecutionLedgerQuality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderbookDepthLedgerRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bid: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_vwap_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_slippage_bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_vwap_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_slippage_bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_usd_5bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_usd_10bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_usd_20bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_notional_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_timestamp_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<MarketDataHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub quality: ExecutionLedgerQuality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ExecutionLedgerPayload {
    OrderState {
        state: LiveOrderState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    FillSnapshot(FillLedgerSnapshot),
    FeeSnapshot(FeeLedgerSnapshot),
    FundingPayment(FundingPaymentLedgerRecord),
    Slippage(SlippageLedgerRecord),
    OrderbookEvidence(Box<OrderbookDepthLedgerRecord>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionLedgerEvent {
    pub event_id: String,
    pub event_type: ExecutionLedgerEventType,
    pub source: OrderUpdateSource,
    pub order: ExecutionLedgerOrderRef,
    pub payload: ExecutionLedgerPayload,
    pub occurred_at_ms: i64,
    pub captured_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_confidence_source_mapping_scores_are_ordered() {
        assert_eq!(
            ExecutionFillConfidence::from_source(
                ExecutionLedgerEventType::FillEvent,
                OrderUpdateSource::PrivateWs
            ),
            ExecutionFillConfidence::VenueFill
        );
        assert_eq!(
            ExecutionFillConfidence::from_source(
                ExecutionLedgerEventType::FillSnapshot,
                OrderUpdateSource::OrderQuery
            ),
            ExecutionFillConfidence::OrderQuery
        );
        assert_eq!(
            ExecutionFillConfidence::from_source(
                ExecutionLedgerEventType::FundingPayment,
                OrderUpdateSource::FundingPoller
            ),
            ExecutionFillConfidence::Unknown
        );
        assert!(
            ExecutionFillConfidence::VenueFill.score()
                > ExecutionFillConfidence::AdapterAck.score()
        );
        assert!(
            ExecutionFillConfidence::AdapterAck.score() > ExecutionFillConfidence::Unknown.score()
        );
        assert!(ExecutionFillConfidence::VenueFill.supports_terminal_fill());
        assert!(ExecutionFillConfidence::OrderQuery.supports_terminal_fill());
        assert!(!ExecutionFillConfidence::AdapterAck.supports_terminal_fill());
        assert!(!ExecutionFillConfidence::Unknown.supports_terminal_fill());
    }

    #[test]
    fn fill_snapshot_deserializes_legacy_without_confidence() {
        let snapshot = serde_json::from_str::<FillLedgerSnapshot>(
            r#"{"quantity":1.0,"averagePrice":10.0,"quoteValue":10.0,"quality":"actual"}"#,
        )
        .unwrap_or_else(|error| panic!("decode legacy fill snapshot: {error}"));

        assert_eq!(snapshot.confidence, ExecutionFillConfidence::Unknown);
    }

    #[test]
    fn order_ref_deserializes_legacy_without_reduce_only() {
        let order = serde_json::from_str::<ExecutionLedgerOrderRef>(
            r#"{
                "runId":"run-1",
                "ticketId":"ticket-1",
                "legRole":"long",
                "exchange":"okx",
                "symbol":"BTCUSDT",
                "side":"buy",
                "identity":{
                    "internalOrderId":"ord-1",
                    "publicClientOrderId":"client-1"
                }
            }"#,
        )
        .unwrap_or_else(|error| panic!("decode legacy order ref: {error}"));

        assert_eq!(order.reduce_only, None);
    }
}
