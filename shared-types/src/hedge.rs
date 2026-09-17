//! Hedge ticket and execution-run contracts.

use crate::arbitrage::{ExecutionCostProfile, OpportunityLegMarketEvidence, SpotLegMode};
use crate::enums::{OrderSide, OrderType};
use crate::execution_run::ExecutionRunEvidence;
use crate::execution_sizing::{
    validate_order_sizing_contract, OrderSizingContractError, OrderSizingPlan,
};
use crate::fees::{FeeProduct, TradeFeeSnapshot};
use crate::instrument_registry::InstrumentSpec;
use crate::live_trading::{
    LiveOrderState, OrderUpdateSource, VenueAccountModeInfo, VenueOrderIdentity,
};
use crate::market::MarketDataHealth;
use crate::order_identity::{ClientOrderIdPolicy, OrderIdentityPlan};
use crate::orders::{AccountDataHealth, AccountFieldQuality, VenueBalanceInfo};
use crate::problem::ApiProblem;
use crate::strategy::StrategyKind;
use crate::venue_capabilities::VenueCapabilityMatrix;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeLegRole {
    #[default]
    Long,
    Short,
}

/// Resolves the product carried by one leg of a P0 hedge strategy.
///
/// Spot/perp orientation is explicit because inventory-sale and borrow-sale
/// opportunities place the spot product on the short leg.
#[must_use]
pub fn p0_hedge_leg_product(
    strategy: Option<StrategyKind>,
    spot_leg_mode: Option<SpotLegMode>,
    role: HedgeLegRole,
) -> Option<FeeProduct> {
    match strategy? {
        StrategyKind::PerpCross | StrategyKind::PerpPriceSpread => Some(FeeProduct::Perp),
        StrategyKind::SpotCross => Some(FeeProduct::Spot),
        StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp => match (spot_leg_mode, role) {
            (Some(SpotLegMode::BuySpot), HedgeLegRole::Long)
            | (
                Some(SpotLegMode::SellInventory | SpotLegMode::BorrowAndSell),
                HedgeLegRole::Short,
            ) => Some(FeeProduct::Spot),
            (Some(_), _) => Some(FeeProduct::Perp),
            (None, _) => None,
        },
        StrategyKind::Triangular
        | StrategyKind::FundingCarry
        | StrategyKind::CashAndCarry
        | StrategyKind::OptionsPerpBasis
        | StrategyKind::QuarterlyPerp
        | StrategyKind::OnchainDepeg => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarginMode {
    #[default]
    Cross,
    Isolated,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    #[default]
    Ioc,
    Fok,
    Gtc,
    Gtx,
}

/// Legacy venue market-like order style retained for serialized ticket compatibility.
/// Active venue capability registries do not advertise these styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueMarketOrderStyle {
    Opponent,
    Optimal5,
    Optimal10,
    Optimal20,
}

impl VenueMarketOrderStyle {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Opponent => "opponent",
            Self::Optimal5 => "optimal_5",
            Self::Optimal10 => "optimal_10",
            Self::Optimal20 => "optimal_20",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Opponent => "Opponent",
            Self::Optimal5 => "Optimal 5",
            Self::Optimal10 => "Optimal 10",
            Self::Optimal20 => "Optimal 20",
        }
    }
}

impl std::str::FromStr for VenueMarketOrderStyle {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "opponent" => Ok(Self::Opponent),
            "optimal_5" => Ok(Self::Optimal5),
            "optimal_10" => Ok(Self::Optimal10),
            "optimal_20" => Ok(Self::Optimal20),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeExecutionParams {
    pub capital_usd: f64,
    pub leverage: f64,
    #[serde(default = "default_order_type")]
    pub order_type: OrderType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_order_style: Option<VenueMarketOrderStyle>,
    pub margin_mode: MarginMode,
    pub time_in_force: TimeInForce,
    pub post_only: bool,
    pub limit_offset_bps: f64,
}

impl Default for HedgeExecutionParams {
    fn default() -> Self {
        Self {
            capital_usd: 750.0,
            leverage: 1.0,
            order_type: OrderType::Limit,
            market_order_style: None,
            margin_mode: MarginMode::Cross,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            limit_offset_bps: 0.0,
        }
    }
}

fn default_order_type() -> OrderType {
    OrderType::Limit
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionGuard {
    pub key: String,
    pub label: String,
    pub passed: bool,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_outcome: Option<MarginPreflightOutcome>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgePreflightOperation {
    #[default]
    MarginBalance,
    PrivateRead,
    Positions,
    OpenOrders,
    Capability,
    AccountMode,
    OrderWrite,
    PrivateWs,
    OrderFinality,
    Orderbook,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgePreflightStatus {
    Passed,
    Blocked,
    Failed,
    #[default]
    Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgePreflightScope {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub venues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<HedgePreflightOperation>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarginPreflightOutcome {
    pub status: HedgePreflightStatus,
    pub checked_at_ms: i64,
    pub scope: HedgePreflightScope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_venues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub balance_rows: Vec<VenueBalanceInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_quality: Vec<AccountFieldQuality>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_health: Vec<AccountDataHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueOrderKind {
    #[default]
    Limit,
    PostOnly,
    NativeMarket,
    ProtectedIoc,
    PriceZeroIoc,
    MarketLike,
    MarketLikeRequired,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderPayloadPricePolicy {
    #[default]
    LimitPrice,
    Omit,
    ProtectionPrice,
    ZeroPrice,
    MarketLikeNoPrice,
}

/// Route-scoped capability evidence for one venue, symbol, and product.
///
/// `OrderCompilePlan` retains its legacy flattened availability fields for
/// older clients, while this contract is the authoritative capability payload
/// consumed by current preview surfaces and ticket-bound execution.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueSymbolCapability {
    pub venue: String,
    pub symbol: String,
    #[serde(default)]
    pub product: FeeProduct,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_order_types: Vec<OrderType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_time_in_force: Vec<TimeInForce>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_margin_modes: Vec<MarginMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub market_order_styles: Vec<VenueMarketOrderStyle>,
    #[serde(default)]
    pub supports_reduce_only: bool,
    #[serde(default)]
    pub matrix: VenueCapabilityMatrix,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_mode: Option<VenueAccountModeInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_mode_error: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// Write-only context derived from immutable ticket-bound compile evidence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderSubmissionContext {
    /// Product selected by the immutable compile plan. Adapters must use this
    /// value instead of inferring spot/perp routing from a symbol spelling.
    #[serde(default)]
    pub product: FeeProduct,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_order_style: Option<VenueMarketOrderStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument_spec: Option<InstrumentSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizing_plan: Option<OrderSizingPlan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderCompilePlan {
    pub role: HedgeLegRole,
    pub exchange: String,
    pub symbol: String,
    #[serde(default)]
    pub client_order_id_policy: ClientOrderIdPolicy,
    #[serde(default)]
    pub product: FeeProduct,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument_spec: Option<InstrumentSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizing_plan: Option<OrderSizingPlan>,
    pub requested_order_type: OrderType,
    pub effective_order_type: OrderType,
    pub requested_time_in_force: TimeInForce,
    pub effective_time_in_force: TimeInForce,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_order_types: Vec<OrderType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_time_in_force: Vec<TimeInForce>,
    /// Margin modes the venue order write path can honor for this order.
    /// Empty means per-order margin mode is not part of this venue's write payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_margin_modes: Vec<MarginMode>,
    #[serde(default)]
    pub venue_capability: VenueSymbolCapability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_order_style: Option<VenueMarketOrderStyle>,
    pub venue_order_kind: VenueOrderKind,
    pub payload_price_policy: OrderPayloadPricePolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protection_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage_tolerance_bps: Option<f64>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
}

impl OrderCompilePlan {
    pub fn identity_plan(&self) -> OrderIdentityPlan {
        OrderIdentityPlan::from_compile_contract(
            &self.exchange,
            &self.symbol,
            self.product,
            &self.client_order_id_policy,
        )
    }

    pub fn submission_context(&self) -> OrderSubmissionContext {
        OrderSubmissionContext {
            product: self.product,
            market_order_style: self.market_order_style,
            instrument_spec: self.instrument_spec.clone(),
            sizing_plan: self.sizing_plan,
        }
    }

    pub fn validate_sizing_contract(&self) -> Result<(), OrderSizingContractError> {
        let instrument = self
            .instrument_spec
            .as_ref()
            .ok_or(OrderSizingContractError::InstrumentMissing)?;
        let sizing = self
            .sizing_plan
            .ok_or(OrderSizingContractError::SizingMissing)?;
        validate_order_sizing_contract(&self.exchange, &self.symbol, instrument, sizing)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeTicketOrderPlanEvidence {
    pub compile_plan: OrderCompilePlan,
    pub identity_plan: OrderIdentityPlan,
}

impl HedgeTicketOrderPlanEvidence {
    fn from_compile_plan(compile_plan: OrderCompilePlan) -> Self {
        let identity_plan = compile_plan.identity_plan();
        Self {
            compile_plan,
            identity_plan,
        }
    }

    fn validate(&self, expected_role: HedgeLegRole) -> Result<(), HedgeTicketOrderPlansError> {
        if self.compile_plan.role != expected_role {
            return Err(HedgeTicketOrderPlansError::RoleMismatch(expected_role));
        }
        if self.identity_plan != self.compile_plan.identity_plan() {
            return Err(HedgeTicketOrderPlansError::IdentityMismatch(expected_role));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HedgeTicketOrderPlansError {
    EmptyTicketId,
    TicketMismatch,
    RoleMismatch(HedgeLegRole),
    IdentityMismatch(HedgeLegRole),
}

impl std::fmt::Display for HedgeTicketOrderPlansError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTicketId => f.write_str("ticket id is required"),
            Self::TicketMismatch => f.write_str("ticket id does not match ticket plan evidence"),
            Self::RoleMismatch(role) => {
                write!(
                    f,
                    "{} ticket plan has the wrong compile role",
                    role_label(*role)
                )
            }
            Self::IdentityMismatch(role) => {
                write!(
                    f,
                    "{} ticket plan identity evidence does not match",
                    role_label(*role)
                )
            }
        }
    }
}

fn role_label(role: HedgeLegRole) -> &'static str {
    match role {
        HedgeLegRole::Long => "long",
        HedgeLegRole::Short => "short",
    }
}

/// Immutable order-plan evidence bound to a hedge ticket.
///
/// The role keys make the long/short mapping explicit, while the stored identity
/// plan freezes the evidence after preview guard composition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeTicketOrderPlans {
    pub ticket_id: String,
    pub long: HedgeTicketOrderPlanEvidence,
    pub short: HedgeTicketOrderPlanEvidence,
}

impl HedgeTicketOrderPlans {
    pub fn from_compile_plans(
        ticket_id: impl Into<String>,
        long: OrderCompilePlan,
        short: OrderCompilePlan,
    ) -> Result<Self, HedgeTicketOrderPlansError> {
        let plans = Self {
            ticket_id: ticket_id.into(),
            long: HedgeTicketOrderPlanEvidence::from_compile_plan(long),
            short: HedgeTicketOrderPlanEvidence::from_compile_plan(short),
        };
        plans.plans_for_ticket(&plans.ticket_id)?;
        Ok(plans)
    }

    pub fn plans_for_ticket(
        &self,
        ticket_id: &str,
    ) -> Result<[&HedgeTicketOrderPlanEvidence; 2], HedgeTicketOrderPlansError> {
        if self.ticket_id.trim().is_empty() || ticket_id.trim().is_empty() {
            return Err(HedgeTicketOrderPlansError::EmptyTicketId);
        }
        if self.ticket_id != ticket_id {
            return Err(HedgeTicketOrderPlansError::TicketMismatch);
        }
        self.long.validate(HedgeLegRole::Long)?;
        self.short.validate(HedgeLegRole::Short)?;
        Ok([&self.long, &self.short])
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeDepthStatus {
    Available,
    Insufficient,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeExecutableNotional {
    pub status: HedgeDepthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_leg_depth_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_leg_depth_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeSizing {
    pub requested_capital_usd: f64,
    pub leverage: f64,
    pub target_notional_usd: f64,
    /// 用户为多腿授权的美元名义上限；旧票据缺失时回退到 `target_notional_usd`。
    #[serde(default)]
    pub long_notional_cap_usd: f64,
    /// 用户为空腿授权的美元名义上限；旧票据缺失时回退到 `target_notional_usd`。
    #[serde(default)]
    pub short_notional_cap_usd: f64,
    /// 双腿共同使用的基础资产数量；存在时优先于“同美元名义”推导。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_base_quantity: Option<f64>,
    #[serde(default)]
    pub max_executable_notional: HedgeExecutableNotional,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeLegQuote {
    pub role: HedgeLegRole,
    pub exchange: String,
    pub symbol: String,
    pub side: OrderSide,
    pub reference_price: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub mid: Option<f64>,
    pub open_vwap_price: Option<f64>,
    pub open_slippage_bps: Option<f64>,
    pub close_vwap_price: Option<f64>,
    pub close_slippage_bps: Option<f64>,
    pub depth_usd_5bps: Option<f64>,
    pub depth_usd_10bps: Option<f64>,
    pub depth_usd_20bps: Option<f64>,
    pub max_notional_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_evidence: Option<OpportunityLegMarketEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_health: Option<MarketDataHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_reason: Option<String>,
    pub funding_bps: Option<f64>,
    pub next_funding_time: i64,
    #[serde(default)]
    pub funding_interval_hours: u32,
    pub market_timestamp_ms: Option<i64>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeTicket {
    pub ticket_id: String,
    pub opportunity_id: String,
    pub strategy: Option<StrategyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_leg_mode: Option<SpotLegMode>,
    pub symbol: String,
    pub created_at_ms: i64,
    /// Timestamp of the market/risk facts currently bound to this ticket.
    /// Legacy tickets fall back to `created_at_ms` when this field is zero.
    #[serde(default)]
    pub market_checked_at_ms: i64,
    pub expires_at_ms: i64,
    pub long_leg: HedgeLegQuote,
    pub short_leg: HedgeLegQuote,
    pub cost: Option<ExecutionCostProfile>,
    #[serde(default)]
    pub fee_snapshots: Vec<TradeFeeSnapshot>,
    pub sizing: HedgeSizing,
    pub guards: Vec<ExecutionGuard>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRunState {
    #[default]
    Previewed,
    RiskChecked,
    SubmittingFirstLeg,
    FirstLegPartial,
    SubmittingSecondLeg,
    SecondLegSubmitted,
    Hedged,
    UnwindRequired,
    Unwinding,
    FailedSafe,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    CancelOpenOrders,
    UnwindLongLeg,
    UnwindShortLeg,
    ManualReview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRunLeg {
    pub role: HedgeLegRole,
    pub exchange: String,
    pub symbol: String,
    pub order_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<VenueOrderIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_source: Option<OrderUpdateSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_filled_at_ms: Option<i64>,
    pub state: LiveOrderState,
    pub target_quantity: f64,
    pub filled_quantity: Option<f64>,
    pub target_notional_usd: f64,
    pub filled_notional_usd: Option<f64>,
    pub filled_fee: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCostReconciliation {
    pub estimated_open_cost_usd: f64,
    pub estimated_close_cost_usd: f64,
    pub estimated_slippage_usd: f64,
    pub estimated_total_cost_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_fee_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_slippage_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_open_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_funding_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_unwind_fee_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_unwind_slippage_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_unwind_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unwind_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_delta_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRun {
    pub run_id: String,
    pub ticket_id: String,
    pub opportunity_id: String,
    pub state: ExecutionRunState,
    pub long_leg: ExecutionRunLeg,
    pub short_leg: ExecutionRunLeg,
    pub net_exposure_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_reconciliation: Option<ExecutionCostReconciliation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valuation_problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unwind_problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_checked_at_ms: Option<i64>,
    #[serde(default)]
    pub evidence: ExecutionRunEvidence,
    pub recovery_action: Option<RecoveryAction>,
    pub status_reason: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{MarketDataQuality, MarketDataSourceKind};
    use crate::OrderSide;

    #[test]
    fn p0_leg_product_resolves_both_spot_perp_orientations() {
        assert_eq!(
            p0_hedge_leg_product(
                Some(StrategyKind::SpotPerp),
                Some(SpotLegMode::BuySpot),
                HedgeLegRole::Long,
            ),
            Some(FeeProduct::Spot)
        );
        assert_eq!(
            p0_hedge_leg_product(
                Some(StrategyKind::SpotPerp),
                Some(SpotLegMode::SellInventory),
                HedgeLegRole::Long,
            ),
            Some(FeeProduct::Perp)
        );
        assert_eq!(
            p0_hedge_leg_product(
                Some(StrategyKind::SpotPerp),
                Some(SpotLegMode::BorrowAndSell),
                HedgeLegRole::Short,
            ),
            Some(FeeProduct::Spot)
        );
        assert_eq!(
            p0_hedge_leg_product(Some(StrategyKind::CrossSpotPerp), None, HedgeLegRole::Long,),
            None
        );
    }

    #[test]
    fn hedge_leg_quote_market_evidence_is_camel_case_and_legacy_safe() {
        let legacy = r#"{
            "role":"long",
            "exchange":"okx",
            "symbol":"BTCUSDT",
            "side":"buy",
            "referencePrice":100.0,
            "bid":99.9,
            "ask":100.1,
            "mid":100.0,
            "openVwapPrice":100.1,
            "openSlippageBps":0.0,
            "closeVwapPrice":99.9,
            "closeSlippageBps":0.0,
            "depthUsd5bps":1000.0,
            "depthUsd10bps":1500.0,
            "depthUsd20bps":2000.0,
            "maxNotionalUsd":1000.0,
            "fundingBps":0.0,
            "nextFundingTime":0,
            "marketTimestampMs":1,
            "blockers":[]
        }"#;
        let mut quote: HedgeLegQuote = serde_json::from_str(legacy).expect("legacy quote");
        assert_eq!(quote.market_evidence, None);

        quote.market_evidence = Some(OpportunityLegMarketEvidence {
            venue: "okx".into(),
            symbol: "BTCUSDT".into(),
            price: Some(100.0),
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::WsPush,
                freshness_ms: Some(12),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 1,
                coverage: None,
                problem: None,
            },
        });
        let json = serde_json::to_string(&quote).expect("quote json");

        assert!(json.contains("\"marketEvidence\""));
        assert!(json.contains("\"observedAtMs\""));
    }

    #[test]
    fn hedge_leg_quote_omits_empty_market_evidence() {
        let quote = HedgeLegQuote {
            role: HedgeLegRole::Long,
            exchange: "okx".into(),
            symbol: "BTCUSDT".into(),
            side: OrderSide::Buy,
            reference_price: None,
            bid: None,
            ask: None,
            mid: None,
            open_vwap_price: None,
            open_slippage_bps: None,
            close_vwap_price: None,
            close_slippage_bps: None,
            depth_usd_5bps: None,
            depth_usd_10bps: None,
            depth_usd_20bps: None,
            max_notional_usd: None,
            market_evidence: None,
            depth_health: None,
            depth_reason: None,
            funding_bps: None,
            next_funding_time: 0,
            funding_interval_hours: 0,
            market_timestamp_ms: None,
            blockers: Vec::new(),
        };
        let json = serde_json::to_string(&quote).expect("quote json");

        assert!(!json.contains("marketEvidence"));
    }

    #[test]
    fn margin_preflight_outcome_row_health_is_legacy_safe() {
        let json = serde_json::json!({
            "status": "passed",
            "checkedAtMs": 1,
            "scope": {},
            "source": "test"
        });
        let mut outcome: MarginPreflightOutcome =
            serde_json::from_value(json).expect("legacy preflight outcome");

        assert!(outcome.row_health.is_empty());

        outcome.row_health.push(AccountDataHealth::new(
            crate::orders::AccountFieldSubject::balance("binance", "USDT"),
            "account_cache",
            2,
        ));
        let text = serde_json::to_string(&outcome).expect("outcome json");

        assert!(text.contains("\"rowHealth\""));
        assert!(text.contains("\"account_cache\""));
    }

    #[test]
    fn execution_run_deserializes_without_finality_problem() {
        let json = serde_json::json!({
            "runId": "run-1",
            "ticketId": "ticket-1",
            "opportunityId": "opp-1",
            "state": "second_leg_submitted",
            "longLeg": execution_leg_json("long"),
            "shortLeg": execution_leg_json("short"),
            "netExposureUsd": 0.0,
            "recoveryAction": null,
            "statusReason": "legacy",
            "createdAtMs": 1,
            "updatedAtMs": 2
        });

        let run: ExecutionRun = serde_json::from_value(json).expect("legacy execution run");

        assert_eq!(run.finality_problem, None);
        assert_eq!(run.finality_checked_at_ms, None);
    }

    #[test]
    fn execution_cost_reconciliation_deserializes_legacy_without_unwind_fields() {
        let cost: ExecutionCostReconciliation = serde_json::from_value(serde_json::json!({
            "estimatedOpenCostUsd": 0.5,
            "estimatedCloseCostUsd": 0.5,
            "estimatedSlippageUsd": 0.0,
            "estimatedTotalCostUsd": 1.0,
            "filledFeeUsd": 0.2,
            "actualOpenCostUsd": 0.2
        }))
        .expect("legacy cost reconciliation");

        assert_eq!(cost.actual_unwind_fee_usd, None);
        assert_eq!(cost.actual_unwind_slippage_usd, None);
        assert!(cost.unwind_event_ids.is_empty());
        assert!(cost.missing_fields.is_empty());
    }

    #[test]
    fn ticket_order_plans_keep_hyperliquid_builder_compile_and_identity_evidence() {
        let plans = match HedgeTicketOrderPlans::from_compile_plans(
            "ticket-hyperliquid",
            protected_hyperliquid_plan(HedgeLegRole::Long),
            protected_hyperliquid_plan(HedgeLegRole::Short),
        ) {
            Ok(plans) => plans,
            Err(error) => panic!("ticket plans must be valid: {error}"),
        };

        let [long, short] = match plans.plans_for_ticket("ticket-hyperliquid") {
            Ok(plans) => plans,
            Err(error) => panic!("ticket plans must remain valid: {error}"),
        };

        assert_eq!(long.compile_plan.role, HedgeLegRole::Long);
        assert_eq!(short.compile_plan.role, HedgeLegRole::Short);
        assert_eq!(long.compile_plan.exchange, "hyperliquid:xyz");
        assert_eq!(
            long.compile_plan.venue_order_kind,
            VenueOrderKind::ProtectedIoc
        );
        assert_eq!(
            long.compile_plan.payload_price_policy,
            OrderPayloadPricePolicy::ProtectionPrice
        );
        assert_eq!(long.compile_plan.effective_time_in_force, TimeInForce::Ioc);
        assert_eq!(
            long.identity_plan
                .client_order_id_policy
                .venue_client_order_id
                .as_deref(),
            Some("0x00000000000000000000000000000001")
        );
        assert!(long.identity_plan.is_execution_ready());
    }

    #[test]
    fn ticket_order_plans_reject_swapped_compile_roles() {
        let result = HedgeTicketOrderPlans::from_compile_plans(
            "ticket-hyperliquid",
            protected_hyperliquid_plan(HedgeLegRole::Short),
            protected_hyperliquid_plan(HedgeLegRole::Long),
        );

        assert!(matches!(
            result,
            Err(HedgeTicketOrderPlansError::RoleMismatch(HedgeLegRole::Long))
        ));
    }

    fn protected_hyperliquid_plan(role: HedgeLegRole) -> OrderCompilePlan {
        let venue_client_order_id = match role {
            HedgeLegRole::Long => "0x00000000000000000000000000000001",
            HedgeLegRole::Short => "0x00000000000000000000000000000002",
        };
        OrderCompilePlan {
            role,
            exchange: "hyperliquid:xyz".into(),
            symbol: "XYZ".into(),
            client_order_id_policy: ClientOrderIdPolicy {
                venue: "hyperliquid:xyz".into(),
                venue_family: "hyperliquid".into(),
                venue_field: "c/cloid".into(),
                public_client_order_id: format!("public-{}", role_label(role)),
                venue_client_order_id: Some(venue_client_order_id.into()),
                derivation: crate::order_identity::ClientOrderIdDerivation::StableHash,
                policy_version: "hyperliquid-cloid-v1".into(),
                official_format: "0x + 32 lowercase hex".into(),
                max_length: Some(34),
                supports_query_by_client_id: true,
                supports_cancel_by_client_id: true,
                constraints: Vec::new(),
                blockers: Vec::new(),
                official_doc_urls: Vec::new(),
            },
            product: FeeProduct::Perp,
            instrument_spec: None,
            sizing_plan: None,
            requested_order_type: OrderType::Market,
            effective_order_type: OrderType::Limit,
            requested_time_in_force: TimeInForce::Ioc,
            effective_time_in_force: TimeInForce::Ioc,
            available_order_types: vec![OrderType::Limit, OrderType::Market],
            available_time_in_force: vec![TimeInForce::Ioc],
            available_margin_modes: vec![MarginMode::Cross],
            venue_capability: VenueSymbolCapability::default(),
            market_order_style: None,
            venue_order_kind: VenueOrderKind::ProtectedIoc,
            payload_price_policy: OrderPayloadPricePolicy::ProtectionPrice,
            reference_price: Some(100.0),
            protection_price: Some(100.05),
            payload_price: Some(100.05),
            slippage_tolerance_bps: Some(5.0),
            summary: "Hyperliquid protected IOC".into(),
            blockers: Vec::new(),
        }
    }

    fn execution_leg_json(role: &str) -> serde_json::Value {
        serde_json::json!({
            "role": role,
            "exchange": "okx",
            "symbol": "BTCUSDT",
            "orderIds": [],
            "state": "submitted",
            "targetQuantity": 1.0,
            "filledQuantity": null,
            "targetNotionalUsd": 100.0,
            "filledNotionalUsd": null,
            "filledFee": null
        })
    }
}
