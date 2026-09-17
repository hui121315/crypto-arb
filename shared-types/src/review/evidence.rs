use crate::enums::OrderSide;
use crate::execution_ledger::{
    ExecutionFillConfidence, ExecutionLedgerEvent, ExecutionLedgerEventType,
    ExecutionLedgerPayload, ExecutionLedgerQuality, FeeLedgerSnapshot,
};
use crate::hedge::HedgeLegRole;
use crate::live_trading::{OrderUpdateSource, VenueOrderIdentity};
use crate::market::MarketDataHealth;
use crate::portfolio::{CloseRunCostReconciliation, CloseRunStatus, CloseRunUnwindPlanStatus};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLedgerEventEvidence {
    pub event_id: String,
    pub event_type: ExecutionLedgerEventType,
    pub source: OrderUpdateSource,
    pub order: ReviewLedgerOrderEvidence,
    pub timing: ReviewLedgerEventTiming,
    pub payload: ReviewLedgerPayloadEvidence,
}

impl ReviewLedgerEventEvidence {
    pub fn from_execution_event(event: &ExecutionLedgerEvent) -> Option<Self> {
        Some(Self {
            event_id: event.event_id.clone(),
            event_type: event.event_type,
            source: event.source,
            order: ReviewLedgerOrderEvidence {
                run_id: event.order.run_id.clone(),
                ticket_id: event.order.ticket_id.clone(),
                leg_role: event.order.leg_role,
                reduce_only: event.order.reduce_only,
                exchange: event.order.exchange.clone(),
                symbol: event.order.symbol.clone(),
                side: event.order.side,
                identity: event.order.identity.clone(),
            },
            timing: ReviewLedgerEventTiming {
                occurred_at_ms: event.occurred_at_ms,
                captured_at_ms: event.captured_at_ms,
            },
            payload: ReviewLedgerPayloadEvidence::from_payload(&event.payload)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLedgerOrderEvidence {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLedgerEventTiming {
    pub occurred_at_ms: i64,
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ReviewLedgerPayloadEvidence {
    Fill {
        quantity: f64,
        average_price: f64,
        quote_value: f64,
        quality: ExecutionLedgerQuality,
        confidence: ExecutionFillConfidence,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fee: Option<FeeLedgerSnapshot>,
    },
    Fee {
        amount: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        currency: Option<String>,
        quality: ExecutionLedgerQuality,
    },
    Funding {
        amount: f64,
        currency: String,
        funding_time_ms: i64,
        quality: ExecutionLedgerQuality,
    },
    Slippage {
        amount_usd: f64,
        reference_price: f64,
        fill_price: f64,
        quantity: f64,
        quality: ExecutionLedgerQuality,
    },
    Orderbook {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference_price: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bid: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ask: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mid: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open_vwap_price: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open_slippage_bps: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        close_vwap_price: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        close_slippage_bps: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth_usd_5bps: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth_usd_10bps: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth_usd_20bps: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_notional_usd: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        market_timestamp_ms: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        health: Option<Box<MarketDataHealth>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        quality: ExecutionLedgerQuality,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewLedgerPayloadKind {
    Fill,
    Fee,
    Funding,
    Slippage,
    Orderbook,
}

#[derive(Deserialize)]
struct ReviewLedgerPayloadWire {
    #[serde(rename = "type")]
    kind: ReviewLedgerPayloadKind,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct ReviewFillWire {
    quantity: f64,
    average_price: f64,
    quote_value: f64,
    quality: ExecutionLedgerQuality,
    confidence: ExecutionFillConfidence,
    fee: Option<FeeLedgerSnapshot>,
}

#[derive(Deserialize)]
struct ReviewFeeWire {
    amount: f64,
    currency: Option<String>,
    quality: ExecutionLedgerQuality,
}

#[derive(Deserialize)]
struct ReviewFundingWire {
    amount: f64,
    currency: String,
    funding_time_ms: i64,
    quality: ExecutionLedgerQuality,
}

#[derive(Deserialize)]
struct ReviewSlippageWire {
    amount_usd: f64,
    reference_price: f64,
    fill_price: f64,
    quantity: f64,
    quality: ExecutionLedgerQuality,
}

#[derive(Deserialize)]
struct ReviewOrderbookWire {
    reference_price: Option<f64>,
    bid: Option<f64>,
    ask: Option<f64>,
    mid: Option<f64>,
    open_vwap_price: Option<f64>,
    open_slippage_bps: Option<f64>,
    close_vwap_price: Option<f64>,
    close_slippage_bps: Option<f64>,
    depth_usd_5bps: Option<f64>,
    depth_usd_10bps: Option<f64>,
    depth_usd_20bps: Option<f64>,
    max_notional_usd: Option<f64>,
    market_timestamp_ms: Option<i64>,
    health: Option<Box<MarketDataHealth>>,
    reason: Option<String>,
    quality: ExecutionLedgerQuality,
}

impl<'de> Deserialize<'de> for ReviewLedgerPayloadEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ReviewLedgerPayloadWire::deserialize(deserializer)?;
        match wire.kind {
            ReviewLedgerPayloadKind::Fill => {
                let data: ReviewFillWire =
                    serde_json::from_value(wire.data).map_err(D::Error::custom)?;
                Ok(Self::Fill {
                    quantity: data.quantity,
                    average_price: data.average_price,
                    quote_value: data.quote_value,
                    quality: data.quality,
                    confidence: data.confidence,
                    fee: data.fee,
                })
            }
            ReviewLedgerPayloadKind::Fee => {
                let data: ReviewFeeWire =
                    serde_json::from_value(wire.data).map_err(D::Error::custom)?;
                Ok(Self::Fee {
                    amount: data.amount,
                    currency: data.currency,
                    quality: data.quality,
                })
            }
            ReviewLedgerPayloadKind::Funding => {
                let data: ReviewFundingWire =
                    serde_json::from_value(wire.data).map_err(D::Error::custom)?;
                Ok(Self::Funding {
                    amount: data.amount,
                    currency: data.currency,
                    funding_time_ms: data.funding_time_ms,
                    quality: data.quality,
                })
            }
            ReviewLedgerPayloadKind::Slippage => {
                let data: ReviewSlippageWire =
                    serde_json::from_value(wire.data).map_err(D::Error::custom)?;
                Ok(Self::Slippage {
                    amount_usd: data.amount_usd,
                    reference_price: data.reference_price,
                    fill_price: data.fill_price,
                    quantity: data.quantity,
                    quality: data.quality,
                })
            }
            ReviewLedgerPayloadKind::Orderbook => {
                let data: ReviewOrderbookWire =
                    serde_json::from_value(wire.data).map_err(D::Error::custom)?;
                Ok(Self::Orderbook {
                    reference_price: data.reference_price,
                    bid: data.bid,
                    ask: data.ask,
                    mid: data.mid,
                    open_vwap_price: data.open_vwap_price,
                    open_slippage_bps: data.open_slippage_bps,
                    close_vwap_price: data.close_vwap_price,
                    close_slippage_bps: data.close_slippage_bps,
                    depth_usd_5bps: data.depth_usd_5bps,
                    depth_usd_10bps: data.depth_usd_10bps,
                    depth_usd_20bps: data.depth_usd_20bps,
                    max_notional_usd: data.max_notional_usd,
                    market_timestamp_ms: data.market_timestamp_ms,
                    health: data.health,
                    reason: data.reason,
                    quality: data.quality,
                })
            }
        }
    }
}

impl ReviewLedgerPayloadEvidence {
    fn from_payload(payload: &ExecutionLedgerPayload) -> Option<Self> {
        match payload {
            ExecutionLedgerPayload::FillSnapshot(snapshot) => Some(Self::Fill {
                quantity: snapshot.quantity,
                average_price: snapshot.average_price,
                quote_value: snapshot.quote_value,
                quality: snapshot.quality,
                confidence: snapshot.confidence,
                fee: snapshot.fee.clone(),
            }),
            ExecutionLedgerPayload::FeeSnapshot(snapshot) => Some(Self::Fee {
                amount: snapshot.amount,
                currency: snapshot.currency.clone(),
                quality: snapshot.quality,
            }),
            ExecutionLedgerPayload::FundingPayment(payment) => Some(Self::Funding {
                amount: payment.amount,
                currency: payment.currency.clone(),
                funding_time_ms: payment.funding_time_ms,
                quality: payment.quality,
            }),
            ExecutionLedgerPayload::Slippage(record) => Some(Self::Slippage {
                amount_usd: record.amount_usd,
                reference_price: record.reference_price,
                fill_price: record.fill_price,
                quantity: record.quantity,
                quality: record.quality,
            }),
            ExecutionLedgerPayload::OrderbookEvidence(record) => Some(Self::Orderbook {
                reference_price: record.reference_price,
                bid: record.bid,
                ask: record.ask,
                mid: record.mid,
                open_vwap_price: record.open_vwap_price,
                open_slippage_bps: record.open_slippage_bps,
                close_vwap_price: record.close_vwap_price,
                close_slippage_bps: record.close_slippage_bps,
                depth_usd_5bps: record.depth_usd_5bps,
                depth_usd_10bps: record.depth_usd_10bps,
                depth_usd_20bps: record.depth_usd_20bps,
                max_notional_usd: record.max_notional_usd,
                market_timestamp_ms: record.market_timestamp_ms,
                health: record.health.clone().map(Box::new),
                reason: record.reason.clone(),
                quality: record.quality,
            }),
            ExecutionLedgerPayload::OrderState { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPnlEvidence {
    #[serde(default)]
    pub fill_event_ids: Vec<String>,
    #[serde(default)]
    pub fee_event_ids: Vec<String>,
    #[serde(default)]
    pub funding_event_ids: Vec<String>,
    #[serde(default)]
    pub slippage_event_ids: Vec<String>,
    #[serde(default)]
    pub estimated_slippage_fill_event_ids: Vec<String>,
    #[serde(default)]
    pub orderbook_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ledger_events: Vec<ReviewLedgerEventEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub close_run_evidence: Vec<ReviewCloseRunEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_confidence: Option<ExecutionFillConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_confidence_score: Option<f64>,
}

impl ReviewPnlEvidence {
    pub fn record_fill_confidence(&mut self, confidence: ExecutionFillConfidence) {
        let replace = self
            .fill_confidence
            .is_none_or(|current| confidence.score() < current.score());
        if replace {
            self.fill_confidence = Some(confidence);
            self.fill_confidence_score = Some(confidence.score());
        }
    }

    pub fn record_ledger_event(&mut self, event: &ExecutionLedgerEvent) {
        let Some(evidence) = ReviewLedgerEventEvidence::from_execution_event(event) else {
            return;
        };
        if self
            .ledger_events
            .iter()
            .any(|current| current.event_id == evidence.event_id)
        {
            return;
        }
        self.ledger_events.push(evidence);
    }

    pub fn record_close_run_evidence(&mut self, evidence: ReviewCloseRunEvidence) {
        if self
            .close_run_evidence
            .iter()
            .any(|current| current.close_run_id == evidence.close_run_id)
        {
            return;
        }
        self.close_run_evidence.push(evidence);
    }

    pub fn has_complete_slippage_evidence(&self) -> bool {
        let required = self.event_order_ids(&self.fill_event_ids, |payload| {
            matches!(
                payload,
                ReviewLedgerPayloadEvidence::Fill { quality, .. }
                    if *quality != ExecutionLedgerQuality::Missing
            )
        });
        let mut covered = self.event_order_ids(&self.slippage_event_ids, |payload| {
            matches!(
                payload,
                ReviewLedgerPayloadEvidence::Slippage { quality, .. }
                    if *quality != ExecutionLedgerQuality::Missing
            )
        });
        covered.extend(
            self.event_order_ids(&self.estimated_slippage_fill_event_ids, |payload| {
                matches!(
                    payload,
                    ReviewLedgerPayloadEvidence::Fill { quality, .. }
                        if *quality != ExecutionLedgerQuality::Missing
                )
            }),
        );
        !required.is_empty() && required.is_subset(&covered)
    }

    fn event_order_ids<'a>(
        &'a self,
        event_ids: &[String],
        accepted: impl Fn(&ReviewLedgerPayloadEvidence) -> bool,
    ) -> BTreeSet<&'a str> {
        let event_ids = event_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        self.ledger_events
            .iter()
            .filter(|event| event_ids.contains(event.event_id.as_str()) && accepted(&event.payload))
            .map(|event| event.order.identity.internal_order_id.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCloseRunEvidence {
    pub close_run_id: String,
    pub status: CloseRunStatus,
    pub run_id: String,
    pub ticket_id: String,
    pub opportunity_id: String,
    pub matched_notional_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unwind_status: Option<CloseRunUnwindPlanStatus>,
    pub compensation_attempt_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_reconciliation: Option<CloseRunCostReconciliation>,
}
