//! V2 实盘交易共享 DTO。

use crate::enums::{OrderSide, OrderType};
use crate::fees::FeeProduct;
use crate::hedge::{ExecutionRun, MarginMode, TimeInForce};
use crate::order_identity::ClientOrderIdPolicy;
use crate::portfolio::{AutoProfitCloseConfig, AutoProfitCloseConfigPatch};
use crate::strategy::StrategyKind;
use crate::venue_capabilities::VenueCapabilityMatrix;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    DryRun,
    Testnet,
    Live,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    #[default]
    Paper,
    Live,
}

impl ExecutionMode {
    /// Projects legacy transport modes onto the two product-facing environments.
    #[must_use]
    pub const fn environment(self) -> ExecutionEnvironment {
        match self {
            Self::DryRun | Self::Testnet => ExecutionEnvironment::Paper,
            Self::Live => ExecutionEnvironment::Live,
        }
    }
}

impl From<ExecutionMode> for ExecutionEnvironment {
    fn from(mode: ExecutionMode) -> Self {
        mode.environment()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSource {
    Manual,
    Strategy,
    ArbitragePreview,
    CloseRunCompensation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveOrderState {
    Created,
    RiskChecked,
    Submitted,
    Accepted,
    PartiallyFilled,
    Filled,
    CancelRequested,
    Cancelled,
    Rejected,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderUpdateSource {
    #[default]
    Unknown,
    Internal,
    AdapterAck,
    OrderQuery,
    PrivateWs,
    FundingPoller,
    Reconcile,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueAccountModeInfo {
    pub venue: String,
    pub mode: String,
    pub source: String,
    pub checked_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<u64>,
    /// Adapter read scope for the account-mode probe, e.g. `classic_futures`
    /// for KuCoin's Classic Futures contract API vs a web unified (UTA)
    /// account. `None` when the venue has a single account surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderIntent {
    pub id: String,
    pub source: OrderSource,
    #[serde(default)]
    pub strategy: Option<StrategyKind>,
    pub mode: ExecutionMode,
    pub exchange: String,
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage_tolerance_bps: Option<f64>,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub time_in_force: TimeInForce,
    #[serde(default)]
    pub post_only: bool,
    #[serde(default)]
    pub margin_mode: MarginMode,
    #[serde(default = "default_leverage")]
    pub leverage: f64,
    pub client_order_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_order_id_policy: Option<ClientOrderIdPolicy>,
    pub created_at_ms: i64,
}

fn default_leverage() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitOrderRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub client_order_id: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: ExecutionMode,
    #[serde(default = "default_source")]
    pub source: OrderSource,
    #[serde(default)]
    pub strategy: Option<StrategyKind>,
    #[serde(default = "default_exchange")]
    pub exchange: String,
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default, flatten)]
    pub options: SubmitOrderOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitOrderOptions {
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub time_in_force: TimeInForce,
    #[serde(default)]
    pub post_only: bool,
    #[serde(default)]
    pub margin_mode: MarginMode,
    #[serde(default = "default_leverage")]
    pub leverage: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage_tolerance_bps: Option<f64>,
}

impl Default for SubmitOrderOptions {
    fn default() -> Self {
        Self {
            reduce_only: false,
            time_in_force: TimeInForce::default(),
            post_only: false,
            margin_mode: MarginMode::default(),
            leverage: default_leverage(),
            slippage_tolerance_bps: None,
        }
    }
}

fn default_mode() -> ExecutionMode {
    ExecutionMode::DryRun
}

fn default_source() -> OrderSource {
    OrderSource::Manual
}

fn default_exchange() -> String {
    "mock".into()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOrderIdentity {
    pub internal_order_id: String,
    pub public_client_order_id: String,
    /// Product route captured at submission time. This is persisted with the
    /// order identity so cancel/query recovery never has to infer spot vs perp
    /// from an ambiguous native symbol such as `BTCUSDT`.
    #[serde(default)]
    pub product: FeeProduct,
    /// Opaque submission-account scope; absent on legacy records, never inferred from venue alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue_client_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_order_id_policy: Option<ClientOrderIdPolicy>,
    #[serde(default, skip_serializing_if = "OrderTransportMetadata::is_empty")]
    pub transport_metadata: OrderTransportMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOrderIdentityUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_client_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue_client_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "OrderTransportMetadata::is_empty")]
    pub transport_metadata: OrderTransportMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderTransportMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue_fill_evidence: Option<VenueFillTransportEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueFillTransportEvidence {
    pub venue_closed_pnl: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidation: Option<VenueLiquidationTransportEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueLiquidationTransportEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidated_user: Option<String>,
    pub mark_price: String,
    pub method: VenueLiquidationMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueLiquidationMethod {
    Market,
    Backstop,
}

impl OrderTransportMetadata {
    pub fn is_empty(&self) -> bool {
        self.native_transport.is_none()
            && self.native_request_id.is_none()
            && self.native_response_id.is_none()
            && self.venue_fill_evidence.is_none()
    }

    pub fn with_native_transport(mut self, transport: impl Into<String>) -> Self {
        let transport = transport.into();
        if !transport.trim().is_empty() {
            self.native_transport = Some(transport);
        }
        self
    }

    pub fn with_native_request_id(mut self, request_id: impl Into<String>) -> Self {
        let request_id = request_id.into();
        if !request_id.trim().is_empty() {
            self.native_request_id = Some(request_id);
        }
        self
    }

    pub fn with_native_response_id(mut self, response_id: impl Into<String>) -> Self {
        let response_id = response_id.into();
        if !response_id.trim().is_empty() {
            self.native_response_id = Some(response_id);
        }
        self
    }

    pub fn with_venue_fill_evidence(mut self, evidence: VenueFillTransportEvidence) -> Self {
        self.venue_fill_evidence = Some(evidence);
        self
    }

    pub fn merge_from(&mut self, update: &Self) {
        self.record_update(update);
    }

    fn record_update(&mut self, update: &Self) {
        if update.is_empty() {
            return;
        }
        self.native_transport = update
            .native_transport
            .clone()
            .or_else(|| self.native_transport.clone());
        self.native_request_id = update
            .native_request_id
            .clone()
            .or_else(|| self.native_request_id.clone());
        self.native_response_id = update
            .native_response_id
            .clone()
            .or_else(|| self.native_response_id.clone());
        self.venue_fill_evidence = update
            .venue_fill_evidence
            .clone()
            .or_else(|| self.venue_fill_evidence.clone());
    }
}

impl VenueOrderIdentityUpdate {
    pub fn is_empty(&self) -> bool {
        self.public_client_order_id.is_none()
            && self.venue_client_order_id.is_none()
            && self.exchange_order_id.is_none()
            && self.transport_metadata.is_empty()
    }

    pub fn from_ids(
        public_client_order_id: impl Into<String>,
        venue_client_order_id: impl Into<String>,
        exchange_order_id: Option<String>,
    ) -> Self {
        let public_client_order_id = public_client_order_id.into();
        let venue_client_order_id = venue_client_order_id.into();
        Self {
            public_client_order_id: clean_order_id(&public_client_order_id),
            venue_client_order_id: clean_order_id(&venue_client_order_id),
            exchange_order_id: exchange_order_id.and_then(|id| clean_order_id(&id)),
            transport_metadata: OrderTransportMetadata::default(),
        }
    }

    pub fn with_transport_metadata(mut self, metadata: OrderTransportMetadata) -> Self {
        self.transport_metadata = metadata;
        self
    }
}

impl VenueOrderIdentity {
    pub fn from_intent(intent: &OrderIntent) -> Self {
        Self::from_intent_with_product(intent, FeeProduct::Unknown)
    }

    pub fn from_intent_with_product(intent: &OrderIntent, product: FeeProduct) -> Self {
        Self {
            internal_order_id: intent.id.clone(),
            public_client_order_id: intent.client_order_id.clone(),
            product,
            account_scope: None,
            venue_client_order_id: None,
            exchange_order_id: None,
            client_order_id_policy: intent.client_order_id_policy.clone(),
            transport_metadata: OrderTransportMetadata::default(),
        }
    }

    pub fn record_ack(&mut self, ack: &OrderAck) {
        if ack.identity_update.is_empty() {
            self.record_venue_client_order_id(&ack.client_order_id);
        } else {
            self.record_identity_update(&ack.identity_update);
        }
        if let Some(exchange_order_id) = ack.exchange_order_id.as_deref() {
            self.record_exchange_order_id(exchange_order_id);
        }
    }

    pub fn record_identity_update(&mut self, update: &VenueOrderIdentityUpdate) {
        if self.public_client_order_id.is_empty() {
            if let Some(public_client_order_id) = update.public_client_order_id.as_deref() {
                self.public_client_order_id = public_client_order_id.to_owned();
            }
        }
        if let Some(venue_client_order_id) = update.venue_client_order_id.as_deref() {
            self.record_venue_client_order_id(venue_client_order_id);
        }
        if let Some(exchange_order_id) = update.exchange_order_id.as_deref() {
            self.record_exchange_order_id(exchange_order_id);
        }
        self.transport_metadata
            .record_update(&update.transport_metadata);
    }

    pub fn record_venue_client_order_id(&mut self, client_order_id: &str) {
        let client_order_id = client_order_id.trim();
        if client_order_id.is_empty() || client_order_id == self.public_client_order_id {
            return;
        }
        self.venue_client_order_id = Some(client_order_id.to_owned());
    }

    pub fn record_exchange_order_id(&mut self, exchange_order_id: &str) {
        let exchange_order_id = exchange_order_id.trim();
        if !exchange_order_id.is_empty() {
            self.exchange_order_id = Some(exchange_order_id.to_owned());
        }
    }

    pub fn set_exchange_order_id(&mut self, exchange_order_id: Option<&str>) {
        self.exchange_order_id = exchange_order_id.and_then(clean_order_id);
    }

    pub fn matches_order_id(&self, order_id: &str) -> bool {
        let order_id = order_id.trim();
        !order_id.is_empty()
            && (order_id == self.internal_order_id
                || order_id == self.public_client_order_id
                || self
                    .venue_client_order_id
                    .as_deref()
                    .is_some_and(|id| id == order_id)
                || self
                    .exchange_order_id
                    .as_deref()
                    .is_some_and(|id| id == order_id))
    }
}

fn clean_order_id(order_id: &str) -> Option<String> {
    let order_id = order_id.trim();
    (!order_id.is_empty()).then(|| order_id.to_owned())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedPositionFingerprint {
    pub venue: String,
    pub canonical_symbol: String,
    pub native_symbol: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_mode: Option<String>,
    pub opening_identity: String,
    pub source: String,
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskBlockReason {
    ProtectedPosition,
    KillSwitchActive,
    LiveTradingDisabled,
    ExchangeNotAllowed,
    SymbolNotAllowed,
    UnsupportedOrderType,
    MarketOrderNotReduceOnly,
    NonPositiveQuantity,
    MissingLimitPrice,
    NonPositivePrice,
    MaxOrderNotionalExceeded,
    MaxOpenOrdersExceeded,
    HedgeImbalanceExceeded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskBlockEvidence {
    pub code: RiskBlockReason,
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    pub source: String,
    pub checked_at_ms: i64,
}

impl RiskBlockEvidence {
    pub fn compact_summary(&self) -> String {
        let mut parts = vec![format!("{:?} {}", self.code, self.field)];
        if let Some(actual) = &self.actual {
            parts.push(format!("actual={}", display_json_value(actual)));
        }
        if let Some(limit) = &self.limit {
            parts.push(format!("limit={}", display_json_value(limit)));
        }
        if let Some(venue) = &self.venue {
            parts.push(format!("venue={venue}"));
        }
        if let Some(symbol) = &self.symbol {
            parts.push(format!("symbol={symbol}"));
        }
        parts.push(format!("source={}", self.source));
        parts.push(format!("checkedAtMs={}", self.checked_at_ms));
        parts.join(" ")
    }
}

fn display_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskDecision {
    pub allowed: bool,
    pub reasons: Vec<RiskBlockReason>,
    pub computed_notional: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<RiskBlockEvidence>,
}

impl RiskDecision {
    pub fn allow(computed_notional: f64) -> Self {
        Self {
            allowed: true,
            reasons: Vec::new(),
            computed_notional,
            evidence: Vec::new(),
        }
    }

    pub fn block(reasons: Vec<RiskBlockReason>, computed_notional: f64) -> Self {
        Self {
            allowed: false,
            reasons,
            computed_notional,
            evidence: Vec::new(),
        }
    }

    pub fn block_with_evidence(
        reasons: Vec<RiskBlockReason>,
        computed_notional: f64,
        evidence: Vec<RiskBlockEvidence>,
    ) -> Self {
        Self {
            allowed: false,
            reasons,
            computed_notional,
            evidence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderAck {
    pub internal_order_id: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: String,
    #[serde(default, skip_serializing_if = "VenueOrderIdentityUpdate::is_empty")]
    pub identity_update: VenueOrderIdentityUpdate,
    pub state: LiveOrderState,
    pub accepted_at_ms: i64,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_fee: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderRecord {
    pub intent: OrderIntent,
    pub state: LiveOrderState,
    pub risk: Option<RiskDecision>,
    #[serde(default)]
    pub identity: VenueOrderIdentity,
    #[serde(default)]
    pub last_update_source: OrderUpdateSource,
    pub exchange_order_id: Option<String>,
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_fee: Option<f64>,
    pub updated_at_ms: i64,
}

impl OrderRecord {
    pub fn identity_snapshot(&self) -> VenueOrderIdentity {
        let mut identity = if self.identity.internal_order_id.is_empty() {
            VenueOrderIdentity::from_intent(&self.intent)
        } else {
            self.identity.clone()
        };
        if identity.public_client_order_id.is_empty() {
            identity.public_client_order_id = self.intent.client_order_id.clone();
        }
        if identity.internal_order_id.is_empty() {
            identity.internal_order_id = self.intent.id.clone();
        }
        if identity.client_order_id_policy.is_none() {
            identity.client_order_id_policy = self.intent.client_order_id_policy.clone();
        }
        if let Some(exchange_order_id) = self.exchange_order_id.as_deref() {
            identity.record_exchange_order_id(exchange_order_id);
        }
        identity
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelOrderRequest {
    pub exchange: String,
    pub symbol: String,
    pub internal_order_id: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderLifecycleEvent {
    Created,
    RiskApproved,
    RiskRejected,
    Submitted,
    AdapterAccepted,
    AdapterRejected,
    AdapterFailed,
    PartialFill,
    FullFill,
    CancelRequest,
    CancelAck,
    Timeout,
    ExchangeUnknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderEventRecord {
    pub internal_order_id: String,
    pub client_order_id: String,
    pub exchange_order_id: Option<String>,
    #[serde(default)]
    pub source: OrderUpdateSource,
    #[serde(default)]
    pub identity: VenueOrderIdentity,
    pub previous_state: Option<LiveOrderState>,
    pub state: LiveOrderState,
    pub event: OrderLifecycleEvent,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskConfigPatch {
    #[serde(default)]
    pub max_order_notional: Option<f64>,
    #[serde(default)]
    pub max_open_orders: Option<usize>,
    #[serde(default)]
    pub max_hedge_imbalance_pct: Option<f64>,
    #[serde(default)]
    pub allowed_exchanges: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_symbols: Option<Vec<String>>,
    #[serde(default)]
    pub protected_positions: Option<Vec<ProtectedPositionFingerprint>>,
    #[serde(default)]
    pub auto_profit_close: Option<AutoProfitCloseConfigPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KillSwitchRequest {
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_open_order_count: Option<usize>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KillSwitchSummary {
    pub previous_active: bool,
    pub active: bool,
    pub open_order_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_open_order_count: Option<usize>,
    pub reason: String,
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KillSwitchResponse {
    pub status: TradingStatusResponse,
    pub summary: KillSwitchSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingStatusResponse {
    pub adapter: String,
    #[serde(default)]
    pub environment: ExecutionEnvironment,
    pub open_order_count: usize,
    pub risk: TradingRiskStatus,
    pub ws_channels: TradingWsChannels,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation: Option<crate::actions::ActionMutationDiff>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingRiskStatus {
    pub live_trading_enabled: bool,
    pub kill_switch_active: bool,
    pub max_order_notional: f64,
    pub max_open_orders: usize,
    pub max_hedge_imbalance_pct: f64,
    pub liquidation_warn_pct: f64,
    pub liquidation_danger_pct: f64,
    pub allowed_exchanges: Vec<String>,
    pub allowed_symbols: Vec<String>,
    #[serde(default)]
    pub protected_positions: Vec<ProtectedPositionFingerprint>,
    #[serde(default)]
    pub auto_profit_close: AutoProfitCloseConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskAlertEvent {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<TradingRiskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_run: Option<ExecutionRun>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRunEvent {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_run: Option<ExecutionRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_run: Option<crate::CloseRun>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingWsChannels {
    pub orders: String,
    pub execution: String,
    pub risk_alerts: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingAdaptersResponse {
    pub current: String,
    #[serde(default)]
    pub current_environment: ExecutionEnvironment,
    pub options: Vec<TradingAdapterOption>,
    #[serde(default)]
    pub venues: Vec<TradingVenueCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectTradingAdapterRequest {
    pub adapter_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingAdapterCapabilities {
    pub spot: bool,
    pub perp: bool,
    pub limit_orders: bool,
    pub market_orders: bool,
    pub post_only: bool,
    pub reduce_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingAdapterOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub environment: ExecutionEnvironment,
    pub enabled: bool,
    pub credentials_available: bool,
    pub capabilities: TradingAdapterCapabilities,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingVenueCapability {
    pub venue: String,
    #[serde(default)]
    pub environment: ExecutionEnvironment,
    pub credentials_available: bool,
    pub capabilities: TradingAdapterCapabilities,
    #[serde(default)]
    pub matrix: VenueCapabilityMatrix,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvTemplateLine {
    pub venue: String,
    pub field_label: String,
    pub key: String,
    pub configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvTemplateResponse {
    pub lines: Vec<EnvTemplateLine>,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order_identity::{ClientOrderIdDerivation, ClientOrderIdPolicy};

    #[test]
    fn funding_poller_source_uses_stable_snake_case_serde() {
        let encoded = serde_json::to_string(&OrderUpdateSource::FundingPoller)
            .expect("funding poller source encodes");
        let decoded: OrderUpdateSource =
            serde_json::from_str(&encoded).expect("funding poller source decodes");

        assert_eq!(encoded, r#""funding_poller""#);
        assert_eq!(decoded, OrderUpdateSource::FundingPoller);
        assert_eq!(
            serde_json::from_str::<OrderUpdateSource>(r#""private_ws""#)
                .expect("legacy private WS source decodes"),
            OrderUpdateSource::PrivateWs
        );
    }

    #[test]
    fn execution_modes_project_to_only_two_product_environments() {
        assert_eq!(
            ExecutionMode::DryRun.environment(),
            ExecutionEnvironment::Paper
        );
        assert_eq!(
            ExecutionMode::Testnet.environment(),
            ExecutionEnvironment::Paper
        );
        assert_eq!(
            ExecutionMode::Live.environment(),
            ExecutionEnvironment::Live
        );
    }

    #[test]
    fn submit_order_request_keeps_flat_http_shape() {
        let payload = serde_json::json!({
            "exchange": "binance",
            "symbol": "BTCUSDT",
            "side": "buy",
            "orderType": "limit",
            "quantity": 1.25,
            "price": 42000.0,
            "reduceOnly": true,
            "timeInForce": "gtc",
            "postOnly": true,
            "marginMode": "isolated",
            "leverage": 3.0,
            "slippageToleranceBps": 5.0
        });

        let request: SubmitOrderRequest =
            serde_json::from_value(payload.clone()).expect("flat request decodes");

        assert_eq!(request.exchange, "binance");
        assert_eq!(request.options.time_in_force, TimeInForce::Gtc);
        assert_eq!(request.options.margin_mode, MarginMode::Isolated);
        assert!(request.options.post_only);
        assert!(request.options.reduce_only);
        assert_eq!(request.options.leverage, 3.0);
        assert_eq!(request.options.slippage_tolerance_bps, Some(5.0));

        let encoded = serde_json::to_value(&request).expect("request encodes");

        assert_eq!(encoded["timeInForce"], payload["timeInForce"]);
        assert_eq!(encoded["postOnly"], payload["postOnly"]);
        assert_eq!(encoded["marginMode"], payload["marginMode"]);
        assert_eq!(encoded["leverage"], payload["leverage"]);
        assert_eq!(
            encoded["slippageToleranceBps"],
            payload["slippageToleranceBps"]
        );
        assert!(encoded.get("options").is_none());
    }

    #[test]
    fn submit_order_request_defaults_to_paper_safe_values() {
        let payload = serde_json::json!({
            "symbol": "ETHUSDT",
            "side": "sell",
            "orderType": "market",
            "quantity": 2.0
        });

        let request: SubmitOrderRequest =
            serde_json::from_value(payload).expect("minimal request decodes");

        assert_eq!(request.mode, ExecutionMode::DryRun);
        assert_eq!(request.source, OrderSource::Manual);
        assert_eq!(request.exchange, "mock");
        assert_eq!(request.options.leverage, 1.0);
        assert_eq!(request.options.time_in_force, TimeInForce::Ioc);
    }

    #[test]
    fn order_intent_keeps_client_order_id_policy_backward_compatible() {
        let intent: OrderIntent = serde_json::from_value(serde_json::json!({
            "id": "ord-1",
            "source": "manual",
            "mode": "dry_run",
            "exchange": "binance",
            "symbol": "BTCUSDT",
            "side": "buy",
            "orderType": "limit",
            "quantity": 1.0,
            "price": 42000.0,
            "clientOrderId": "client-1",
            "createdAtMs": 10
        }))
        .expect("legacy intent decodes");

        assert!(intent.client_order_id_policy.is_none());

        let encoded = serde_json::to_value(&intent).expect("intent encodes");

        assert!(encoded.get("clientOrderIdPolicy").is_none());
    }

    #[test]
    fn risk_decision_evidence_is_backward_compatible() {
        let decision: RiskDecision = serde_json::from_value(serde_json::json!({
            "allowed": false,
            "reasons": ["max_order_notional_exceeded"],
            "computedNotional": 500.0
        }))
        .expect("legacy risk decision decodes");

        assert!(decision.evidence.is_empty());

        let encoded = serde_json::to_value(&decision).expect("decision encodes");

        assert!(encoded.get("evidence").is_none());
    }

    #[test]
    fn protected_position_fingerprint_uses_explicit_opening_identity() {
        let fingerprint = ProtectedPositionFingerprint {
            venue: "binance".to_owned(),
            canonical_symbol: "BTC".to_owned(),
            native_symbol: "BTCUSDT".to_owned(),
            side: "long".to_owned(),
            quantity: 0.232,
            entry_price: 64_456.2,
            position_mode: Some("both".to_owned()),
            opening_identity: "preexisting-binance-btc-long".to_owned(),
            source: "account_position_runtime".to_owned(),
            captured_at_ms: 42,
        };

        let encoded = serde_json::to_value(&fingerprint).expect("fingerprint encodes");

        assert_eq!(encoded["nativeSymbol"], "BTCUSDT");
        assert_eq!(encoded["openingIdentity"], "preexisting-binance-btc-long");
        assert_eq!(encoded["capturedAtMs"], 42);
    }

    #[test]
    fn risk_block_evidence_serializes_actual_limit_source() {
        let evidence = RiskBlockEvidence {
            code: RiskBlockReason::MaxOrderNotionalExceeded,
            field: "computed_notional".to_owned(),
            actual: Some(serde_json::json!(500.0)),
            limit: Some(serde_json::json!(100.0)),
            venue: Some("binance".to_owned()),
            symbol: Some("BTCUSDT".to_owned()),
            source: "trading.risk_engine".to_owned(),
            checked_at_ms: 42,
        };
        let decision = RiskDecision::block_with_evidence(
            vec![RiskBlockReason::MaxOrderNotionalExceeded],
            500.0,
            vec![evidence],
        );
        let encoded = serde_json::to_value(&decision).expect("decision encodes");

        assert_eq!(
            encoded["evidence"][0]["code"],
            "max_order_notional_exceeded"
        );
        assert_eq!(encoded["evidence"][0]["field"], "computed_notional");
        assert_eq!(encoded["evidence"][0]["actual"], 500.0);
        assert_eq!(encoded["evidence"][0]["limit"], 100.0);
        assert_eq!(encoded["evidence"][0]["source"], "trading.risk_engine");
        assert_eq!(encoded["evidence"][0]["checkedAtMs"], 42);
    }

    #[test]
    fn execution_run_event_uses_camel_case_shape() {
        let encoded = serde_json::to_value(ExecutionRunEvent {
            event: "execution_run_updated".to_owned(),
            execution_run: None,
            close_run: None,
            timestamp_ms: 42,
        })
        .expect("execution run event encodes");

        assert_eq!(encoded["event"], "execution_run_updated");
        assert!(encoded.get("executionRun").is_none());
        assert_eq!(encoded["timestampMs"], 42);

        let decoded: ExecutionRunEvent = serde_json::from_value(serde_json::json!({
            "event": "execution_run_updated",
            "executionRun": null,
            "timestampMs": 43
        }))
        .expect("execution run event decodes");

        assert_eq!(decoded.event, "execution_run_updated");
        assert!(decoded.execution_run.is_none());
        assert_eq!(decoded.timestamp_ms, 43);
    }

    #[test]
    fn venue_identity_carries_policy_from_intent_and_snapshot() {
        let policy = client_policy("okx", "client-1");
        let intent = OrderIntent {
            id: "ord-1".to_owned(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "okx".to_owned(),
            symbol: "BTC-USDT-SWAP".to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(42_000.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".to_owned(),
            client_order_id_policy: Some(policy.clone()),
            created_at_ms: 10,
        };

        let identity = VenueOrderIdentity::from_intent(&intent);

        assert_eq!(identity.client_order_id_policy, Some(policy.clone()));

        let record = OrderRecord {
            intent,
            state: LiveOrderState::Created,
            risk: None,
            identity: VenueOrderIdentity {
                account_scope: None,
                internal_order_id: "ord-1".to_owned(),
                public_client_order_id: "client-1".to_owned(),
                product: FeeProduct::Perp,
                venue_client_order_id: None,
                exchange_order_id: None,
                client_order_id_policy: None,
                transport_metadata: OrderTransportMetadata::default(),
            },
            last_update_source: OrderUpdateSource::Internal,
            exchange_order_id: Some("ex-1".to_owned()),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
            updated_at_ms: 11,
        };
        let snapshot = record.identity_snapshot();

        assert_eq!(snapshot.client_order_id_policy, Some(policy));
        assert_eq!(snapshot.exchange_order_id.as_deref(), Some("ex-1"));
    }

    #[test]
    fn order_ack_identity_update_is_backward_compatible() {
        let ack: OrderAck = serde_json::from_value(serde_json::json!({
            "internalOrderId": "ord-1",
            "exchangeOrderId": "ex-1",
            "clientOrderId": "client-1",
            "state": "accepted",
            "acceptedAtMs": 10
        }))
        .expect("legacy ack decodes");

        assert!(ack.identity_update.is_empty());
        assert_eq!(ack.client_order_id, "client-1");
    }

    #[test]
    fn venue_identity_ack_merge_separates_transport_request_id() {
        let mut identity = VenueOrderIdentity {
            account_scope: None,
            internal_order_id: "ord-1".to_owned(),
            public_client_order_id: "public-cid".to_owned(),
            product: FeeProduct::Perp,
            venue_client_order_id: None,
            exchange_order_id: None,
            client_order_id_policy: None,
            transport_metadata: OrderTransportMetadata::default(),
        };
        let ack = OrderAck {
            internal_order_id: "ord-1".to_owned(),
            exchange_order_id: None,
            client_order_id: "public-cid".to_owned(),
            identity_update: VenueOrderIdentityUpdate::from_ids(
                "public-cid",
                "venue-cid",
                Some("ex-1".to_owned()),
            )
            .with_transport_metadata(
                OrderTransportMetadata::default()
                    .with_native_transport("hyperliquid_ws_post")
                    .with_native_request_id("ws-req-42")
                    .with_native_response_id("ws-resp-42"),
            ),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 10,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        };

        identity.record_ack(&ack);

        assert!(identity.matches_order_id("public-cid"));
        assert!(identity.matches_order_id("venue-cid"));
        assert!(identity.matches_order_id("ex-1"));
        assert!(!identity.matches_order_id("ws-req-42"));
        assert_eq!(identity.exchange_order_id.as_deref(), Some("ex-1"));
        assert_eq!(
            identity.transport_metadata.native_transport.as_deref(),
            Some("hyperliquid_ws_post")
        );
        assert_eq!(
            identity.transport_metadata.native_request_id.as_deref(),
            Some("ws-req-42")
        );
        assert_eq!(
            identity.transport_metadata.native_response_id.as_deref(),
            Some("ws-resp-42")
        );
        let encoded = serde_json::to_value(&identity).expect("identity encodes");
        assert_eq!(encoded["transportMetadata"]["nativeRequestId"], "ws-req-42");
    }

    #[test]
    fn kill_switch_request_uses_structured_confirmation_shape() {
        let request: KillSwitchRequest = serde_json::from_value(serde_json::json!({
            "active": true,
            "expectedActive": false,
            "expectedOpenOrderCount": 3,
            "reason": "positions.kill_switch.enable"
        }))
        .expect("kill switch request decodes");
        let encoded = serde_json::to_value(&request).expect("kill switch request encodes");

        assert!(request.active);
        assert_eq!(request.expected_active, Some(false));
        assert_eq!(request.expected_open_order_count, Some(3));
        assert_eq!(encoded["expectedOpenOrderCount"], 3);
        assert!(encoded.get("confirmationPhrase").is_none());
    }

    #[test]
    fn trading_status_action_receipt_is_backward_compatible() {
        let status: TradingStatusResponse = serde_json::from_value(serde_json::json!({
            "adapter": "mock",
            "environment": "paper",
            "openOrderCount": 0,
            "risk": {
                "liveTradingEnabled": false,
                "killSwitchActive": false,
                "maxOrderNotional": 1000.0,
                "maxOpenOrders": 4,
                "maxHedgeImbalancePct": 0.05,
                "liquidationWarnPct": 0.2,
                "liquidationDangerPct": 0.1,
                "allowedExchanges": [],
                "allowedSymbols": []
            },
            "wsChannels": {
                "orders": "orders",
                "execution": "execution",
                "riskAlerts": "risk_alerts"
            }
        }))
        .expect("legacy status decodes");

        assert!(status.action_run_id.is_none());
        assert!(status.request_id.is_none());
        assert!(status.idempotency_key.is_none());
        assert!(status.mutation.is_none());
    }

    #[test]
    fn adapter_selection_request_uses_camel_case_shape() {
        let request = SelectTradingAdapterRequest {
            adapter_id: "live".to_owned(),
        };
        let encoded = serde_json::to_value(&request).expect("adapter request encodes");

        assert_eq!(encoded["adapterId"], "live");
        assert!(encoded.get("adapter_id").is_none());
    }

    fn client_policy(venue: &str, client_order_id: &str) -> ClientOrderIdPolicy {
        ClientOrderIdPolicy {
            venue: venue.to_owned(),
            venue_family: venue.to_owned(),
            venue_field: "clOrdId".to_owned(),
            public_client_order_id: client_order_id.to_owned(),
            venue_client_order_id: Some(client_order_id.to_owned()),
            derivation: ClientOrderIdDerivation::Identity,
            policy_version: "test".to_owned(),
            official_format: "test fixture".to_owned(),
            max_length: Some(32),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            constraints: Vec::new(),
            blockers: Vec::new(),
            official_doc_urls: Vec::new(),
        }
    }
}
