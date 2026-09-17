//! 订单 / 仓位 / 余额数据模型。

use crate::enums::{OrderSide, OrderStatus, OrderType};
use crate::list::ListStatus;
use crate::live_trading::{LiveOrderState, OrderRecord};
use crate::problem::ApiProblem;
use crate::venues::VenueOperationHealth;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderInfo {
    pub order_id: String,
    pub symbol: String,
    pub exchange: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub status: OrderStatus,
    pub quantity: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub filled_quantity: f64,
    #[serde(default)]
    pub filled_price: f64,
    #[serde(default)]
    pub fees: f64,
    pub created_at: DateTime<Utc>,
    /// Venue-native execution style retained for legacy ticket compatibility.
    /// so UI/replay/risk can tell apart BBO/optimal/true-market fills
    /// from a plain limit. `None` when no extra style is reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_style: Option<String>,
    /// Venue-native time-in-force or combined order-condition token. Some
    /// venues report this as a dedicated TIF while others encode it in their
    /// native order kind. The raw token is retained instead of guessed into a
    /// cross-venue enum; `None` means the read payload did not report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue_time_in_force: Option<String>,
    /// Venue-native client order id so
    /// reconciliation/replay can match a local intent to the venue order.
    /// `None` when the venue reports no client id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
    /// Whether the venue marks the order reduce-only / close-only, so risk and
    /// finality can tell a closing leg from an opening one. `None` when the
    /// venue does not report it on this read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderReconcileDiffKind {
    LocalMissing,
    RemoteMissing,
    StateMismatch,
    QuantityMismatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderReconcileDiff {
    pub kind: OrderReconcileDiffKind,
    pub exchange_order_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_state: Option<LiveOrderState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_state: Option<LiveOrderState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_quantity: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderStreamRecordEvent {
    pub event: String,
    pub record: OrderRecord,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderStreamReconcileEvent {
    pub event: String,
    #[serde(default)]
    pub diffs: Vec<OrderReconcileDiff>,
    pub diff_count: usize,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OrderStreamPayload {
    Record(Box<OrderStreamRecordEvent>),
    Reconcile(OrderStreamReconcileEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountFieldQualityStatus {
    Actual,
    Estimated,
    Unknown,
    Invalid,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountFieldSubjectKind {
    Account,
    Balance,
    Position,
    OpenOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountBindingStatus {
    Verified,
    Unverified,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountBindingEvidence {
    pub venue: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_scope: Option<String>,
    pub status: AccountBindingStatus,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountFieldSubject {
    pub kind: AccountFieldSubjectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
}

impl AccountFieldSubject {
    pub fn account(venue: impl Into<String>) -> Self {
        Self {
            kind: AccountFieldSubjectKind::Account,
            venue: Some(venue.into()),
            account_scope: None,
            currency: None,
            symbol: None,
            side: None,
            order_id: None,
        }
    }

    pub fn balance(venue: impl Into<String>, currency: impl Into<String>) -> Self {
        Self {
            kind: AccountFieldSubjectKind::Balance,
            venue: Some(venue.into()),
            account_scope: None,
            currency: Some(currency.into()),
            symbol: None,
            side: None,
            order_id: None,
        }
    }

    pub fn position(
        venue: impl Into<String>,
        symbol: impl Into<String>,
        side: impl Into<String>,
    ) -> Self {
        Self {
            kind: AccountFieldSubjectKind::Position,
            venue: Some(venue.into()),
            account_scope: None,
            currency: None,
            symbol: Some(symbol.into()),
            side: Some(side.into()),
            order_id: None,
        }
    }

    pub fn open_order(
        venue: impl Into<String>,
        order_id: impl Into<String>,
        symbol: impl Into<String>,
        side: impl Into<String>,
    ) -> Self {
        Self {
            kind: AccountFieldSubjectKind::OpenOrder,
            venue: Some(venue.into()),
            account_scope: None,
            currency: None,
            symbol: Some(symbol.into()),
            side: Some(side.into()),
            order_id: Some(order_id.into()),
        }
    }

    pub fn with_account_scope(mut self, account_scope: impl Into<String>) -> Self {
        self.account_scope = Some(account_scope.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountFieldQuality {
    pub subject: AccountFieldSubject,
    pub field: String,
    pub status: AccountFieldQualityStatus,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl AccountFieldQuality {
    pub fn new(
        subject: AccountFieldSubject,
        field: impl Into<String>,
        status: AccountFieldQualityStatus,
        source: impl Into<String>,
        observed_at_ms: Option<i64>,
    ) -> Self {
        Self {
            subject,
            field: field.into(),
            status,
            source: source.into(),
            observed_at_ms,
            problem: None,
        }
    }

    pub fn with_problem(mut self, problem: ApiProblem) -> Self {
        self.problem = Some(problem);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDataHealth {
    pub subject: AccountFieldSubject,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl AccountDataHealth {
    pub fn new(
        subject: AccountFieldSubject,
        source: impl Into<String>,
        observed_at_ms: i64,
    ) -> Self {
        Self {
            subject,
            source: source.into(),
            observed_at_ms,
            freshness_ms: None,
            last_success_ms: None,
            last_error: None,
            retry_after_ms: None,
            request_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueAccountSummary {
    pub venue: String,
    pub account_type: String,
    #[serde(default)]
    pub equity_scope: AccountEquityScope,
    pub total_equity_usd: f64,
    pub total_available_balance_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawable_balance_usd: Option<f64>,
    pub total_initial_margin_usd: f64,
    pub total_maintenance_margin_usd: f64,
    pub account_im_rate: f64,
    pub account_mm_rate: f64,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountEquityScope {
    Unified,
    Perpetuals,
    Spot,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionInfo {
    pub symbol: String,
    pub exchange: String,
    /// 'long' or 'short'.
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub mark_price: f64,
    pub unrealized_pnl: f64,
    #[serde(default = "default_leverage")]
    pub leverage: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidation_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidation_distance_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_funding_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_with: Option<String>,
    #[serde(default)]
    pub margin: f64,
    #[serde(default)]
    pub maintenance_margin_ratio: f64,
    /// Venue-native position mode (e.g. `single`, `dual_long`, `single_side`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_mode: Option<String>,
    /// Venue-native margin mode (`cross` / `isolated`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_mode: Option<String>,
    /// Venue-native account/position risk rate when the venue reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_rate: Option<f64>,
    /// Contracts available to close (venue-native `available`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_position: Option<f64>,
    /// Contracts frozen by in-flight close orders (venue-native `frozen`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_position: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenuePositionEnvelope {
    pub rows: Vec<PositionInfo>,
    pub row_count: usize,
    pub status: ListStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub problems: Vec<ApiProblem>,
    #[serde(default)]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default)]
    pub field_quality: Vec<AccountFieldQuality>,
    #[serde(default)]
    pub row_health: Vec<AccountDataHealth>,
    #[serde(default)]
    pub account_bindings: Vec<AccountBindingEvidence>,
}

impl VenuePositionEnvelope {
    pub fn new(
        rows: Vec<PositionInfo>,
        status: ListStatus,
        source: impl Into<String>,
        observed_at_ms: i64,
        problems: Vec<ApiProblem>,
        operation_health: Vec<VenueOperationHealth>,
    ) -> Self {
        let row_count = rows.len();
        Self {
            rows,
            row_count,
            status,
            source: source.into(),
            observed_at_ms,
            problems,
            operation_health,
            field_quality: Vec::new(),
            row_health: Vec::new(),
            account_bindings: Vec::new(),
        }
    }

    pub fn with_field_quality(mut self, field_quality: Vec<AccountFieldQuality>) -> Self {
        self.field_quality = field_quality;
        self
    }

    pub fn with_row_health(mut self, row_health: Vec<AccountDataHealth>) -> Self {
        self.row_health = row_health;
        self
    }

    pub fn with_account_bindings(mut self, account_bindings: Vec<AccountBindingEvidence>) -> Self {
        self.account_bindings = account_bindings;
        self
    }
}

fn default_leverage() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceInfo {
    pub currency: String,
    pub total: f64,
    pub available: f64,
    #[serde(default)]
    pub frozen: f64,
    #[serde(default)]
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueBalanceInfo {
    pub venue: String,
    pub currency: String,
    pub total: f64,
    pub available: f64,
    #[serde(default)]
    pub frozen: f64,
    #[serde(default)]
    pub unrealized_pnl: f64,
}

/// Venue-reported fiat valuation for one balance row.
///
/// Kept separate from [`VenueBalanceInfo`] because quantity and USD value have
/// different units and not every venue supplies a trustworthy valuation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueAssetValuation {
    pub venue: String,
    pub currency: String,
    pub usd_value: f64,
    pub source: String,
    pub observed_at_ms: i64,
}

impl VenueBalanceInfo {
    pub fn from_balance(venue: impl Into<String>, balance: BalanceInfo) -> Self {
        Self {
            venue: venue.into(),
            currency: balance.currency,
            total: balance.total,
            available: balance.available,
            frozen: balance.frozen,
            unrealized_pnl: balance.unrealized_pnl,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueBalanceEnvelope {
    pub rows: Vec<VenueBalanceInfo>,
    pub row_count: usize,
    pub status: ListStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub problems: Vec<ApiProblem>,
    #[serde(default)]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default)]
    pub field_quality: Vec<AccountFieldQuality>,
    #[serde(default)]
    pub row_health: Vec<AccountDataHealth>,
    #[serde(default)]
    pub account_bindings: Vec<AccountBindingEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_summaries: Vec<VenueAccountSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asset_valuations: Vec<VenueAssetValuation>,
}

impl VenueBalanceEnvelope {
    pub fn new(
        rows: Vec<VenueBalanceInfo>,
        status: ListStatus,
        source: impl Into<String>,
        observed_at_ms: i64,
        problems: Vec<ApiProblem>,
        operation_health: Vec<VenueOperationHealth>,
    ) -> Self {
        let row_count = rows.len();
        Self {
            rows,
            row_count,
            status,
            source: source.into(),
            observed_at_ms,
            problems,
            operation_health,
            field_quality: Vec::new(),
            row_health: Vec::new(),
            account_bindings: Vec::new(),
            account_summaries: Vec::new(),
            asset_valuations: Vec::new(),
        }
    }

    pub fn with_field_quality(mut self, field_quality: Vec<AccountFieldQuality>) -> Self {
        self.field_quality = field_quality;
        self
    }

    pub fn with_row_health(mut self, row_health: Vec<AccountDataHealth>) -> Self {
        self.row_health = row_health;
        self
    }

    pub fn with_account_bindings(mut self, account_bindings: Vec<AccountBindingEvidence>) -> Self {
        self.account_bindings = account_bindings;
        self
    }

    pub fn with_account_summaries(mut self, account_summaries: Vec<VenueAccountSummary>) -> Self {
        self.account_summaries = account_summaries;
        self
    }

    pub fn with_asset_valuations(mut self, asset_valuations: Vec<VenueAssetValuation>) -> Self {
        self.asset_valuations = asset_valuations;
        self
    }
}

pub type VenueBalanceSnapshot = VenueBalanceEnvelope;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOpenOrdersEnvelope {
    pub rows: Vec<OrderInfo>,
    pub row_count: usize,
    pub status: ListStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
    #[serde(default)]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_quality: Vec<AccountFieldQuality>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_health: Vec<AccountDataHealth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_bindings: Vec<AccountBindingEvidence>,
}

impl VenueOpenOrdersEnvelope {
    pub fn new(
        rows: Vec<OrderInfo>,
        status: ListStatus,
        source: impl Into<String>,
        observed_at_ms: i64,
        problems: Vec<ApiProblem>,
        operation_health: Vec<VenueOperationHealth>,
    ) -> Self {
        let row_count = rows.len();
        Self {
            rows,
            row_count,
            status,
            source: source.into(),
            observed_at_ms,
            problems,
            operation_health,
            field_quality: Vec::new(),
            row_health: Vec::new(),
            account_bindings: Vec::new(),
        }
    }

    pub fn with_field_quality(mut self, field_quality: Vec<AccountFieldQuality>) -> Self {
        self.field_quality = field_quality;
        self
    }

    pub fn with_row_health(mut self, row_health: Vec<AccountDataHealth>) -> Self {
        self.row_health = row_health;
        self
    }

    pub fn with_account_bindings(mut self, account_bindings: Vec<AccountBindingEvidence>) -> Self {
        self.account_bindings = account_bindings;
        self
    }
}

impl Default for VenueOpenOrdersEnvelope {
    fn default() -> Self {
        Self::new(
            Vec::new(),
            ListStatus::Fresh,
            "account_open_orders_default",
            0,
            Vec::new(),
            Vec::new(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStateSnapshot {
    pub balances: VenueBalanceEnvelope,
    pub positions: VenuePositionEnvelope,
    #[serde(default)]
    pub open_orders: VenueOpenOrdersEnvelope,
    pub status: ListStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
    #[serde(default)]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_quality: Vec<AccountFieldQuality>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_bindings: Vec<AccountBindingEvidence>,
}

impl Default for AccountStateSnapshot {
    fn default() -> Self {
        Self {
            balances: VenueBalanceEnvelope::new(
                Vec::new(),
                ListStatus::Degraded,
                "account_state_default",
                0,
                Vec::new(),
                Vec::new(),
            ),
            positions: VenuePositionEnvelope::new(
                Vec::new(),
                ListStatus::Degraded,
                "account_state_default",
                0,
                Vec::new(),
                Vec::new(),
            ),
            open_orders: VenueOpenOrdersEnvelope::default(),
            status: ListStatus::Degraded,
            source: "account_state_default".to_owned(),
            observed_at_ms: 0,
            problems: Vec::new(),
            operation_health: Vec::new(),
            field_quality: Vec::new(),
            account_bindings: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_balance_envelope_serializes_evidence_fields() {
        let envelope = VenueBalanceEnvelope::new(
            vec![VenueBalanceInfo {
                venue: "mock".to_owned(),
                currency: "USDC".to_owned(),
                total: 1.0,
                available: 1.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            }],
            ListStatus::Degraded,
            "account_balance_runtime",
            42,
            vec![ApiProblem::new(
                "BALANCE_EVIDENCE_MISSING",
                "missing evidence",
            )],
            Vec::new(),
        )
        .with_asset_valuations(vec![VenueAssetValuation {
            venue: "mock".to_owned(),
            currency: "USDC".to_owned(),
            usd_value: 1.0,
            source: "mock.account".to_owned(),
            observed_at_ms: 42,
        }])
        .with_field_quality(vec![AccountFieldQuality::new(
            AccountFieldSubject::balance("mock", "USDC"),
            "available",
            AccountFieldQualityStatus::Unknown,
            "account_balance_runtime",
            Some(42),
        )
        .with_problem(ApiProblem::new(
            "BALANCE_FIELD_UNKNOWN",
            "available balance field is unknown",
        ))])
        .with_row_health(vec![AccountDataHealth {
            subject: AccountFieldSubject::balance("mock", "USDC"),
            source: "account_balance_runtime".to_owned(),
            observed_at_ms: 42,
            freshness_ms: Some(10),
            last_success_ms: Some(41),
            last_error: Some(ApiProblem::new("BALANCE_READ_DEGRADED", "rate limited")),
            retry_after_ms: Some(2_000),
            request_id: Some("req-1".to_owned()),
        }])
        .with_account_bindings(vec![AccountBindingEvidence {
            venue: "mock".to_owned(),
            account_scope: Some("cross_margin".to_owned()),
            status: AccountBindingStatus::Verified,
            source: "account_mode_probe".to_owned(),
            checked_at_ms: Some(40),
            freshness_ms: Some(2),
            credential_fingerprint: Some("hmac-sha256:0123456789abcdef01234567".to_owned()),
            problem: None,
        }]);

        let text = serde_json::to_string(&envelope).expect("serialize balance envelope");

        assert!(text.contains("\"rowCount\":1"));
        assert!(text.contains("\"assetValuations\""));
        assert!(text.contains("\"usdValue\":1.0"));
        assert!(text.contains("\"observedAtMs\":42"));
        assert!(text.contains("\"status\":\"degraded\""));
        assert!(text.contains("\"operationHealth\""));
        assert!(text.contains("\"BALANCE_EVIDENCE_MISSING\""));
        assert!(text.contains("\"fieldQuality\""));
        assert!(text.contains("\"field\":\"available\""));
        assert!(text.contains("\"status\":\"unknown\""));
        assert!(text.contains("\"rowHealth\""));
        assert!(text.contains("\"lastSuccessMs\":41"));
        assert!(text.contains("\"retryAfterMs\":2000"));
        assert!(text.contains("\"requestId\":\"req-1\""));
        assert_balance_binding_json(&text);
    }

    #[test]
    fn venue_position_envelope_serializes_evidence_fields() {
        let envelope = VenuePositionEnvelope::new(
            vec![PositionInfo {
                symbol: "BTCUSDT".to_owned(),
                exchange: "mock".to_owned(),
                side: "long".to_owned(),
                quantity: 1.0,
                entry_price: 10.0,
                mark_price: 11.0,
                unrealized_pnl: 1.0,
                leverage: 1.0,
                liquidation_price: None,
                liquidation_distance_pct: None,
                next_funding_ms: None,
                paired_with: None,
                margin: 10.0,
                maintenance_margin_ratio: 0.0,
                position_mode: None,
                margin_mode: None,
                risk_rate: None,
                available_position: None,
                frozen_position: None,
            }],
            ListStatus::Degraded,
            "account_position_runtime",
            42,
            vec![ApiProblem::new(
                "POSITION_EVIDENCE_MISSING",
                "missing evidence",
            )],
            Vec::new(),
        )
        .with_field_quality(vec![AccountFieldQuality::new(
            AccountFieldSubject::position("mock", "BTCUSDT", "long"),
            "markPrice",
            AccountFieldQualityStatus::Missing,
            "account_position_runtime",
            Some(42),
        )
        .with_problem(ApiProblem::new(
            "POSITION_MARK_PRICE_MISSING",
            "mark price is missing",
        ))])
        .with_row_health(vec![AccountDataHealth {
            subject: AccountFieldSubject::position("mock", "BTCUSDT", "long"),
            source: "account_position_runtime".to_owned(),
            observed_at_ms: 42,
            freshness_ms: Some(10),
            last_success_ms: Some(32),
            last_error: Some(ApiProblem::new("POSITION_READ_DEGRADED", "rate limited")),
            retry_after_ms: Some(2_000),
            request_id: Some("req-2".to_owned()),
        }])
        .with_account_bindings(vec![AccountBindingEvidence {
            venue: "mock".to_owned(),
            account_scope: None,
            status: AccountBindingStatus::Unverified,
            source: "credential_inventory".to_owned(),
            checked_at_ms: None,
            freshness_ms: None,
            credential_fingerprint: None,
            problem: Some(ApiProblem::new(
                "ACCOUNT_SCOPE_UNVERIFIED",
                "account scope is unverified",
            )),
        }]);

        let text = serde_json::to_string(&envelope).expect("serialize position envelope");

        assert!(text.contains("\"rowCount\":1"));
        assert!(text.contains("\"observedAtMs\":42"));
        assert!(text.contains("\"status\":\"degraded\""));
        assert!(text.contains("\"operationHealth\""));
        assert!(text.contains("\"POSITION_EVIDENCE_MISSING\""));
        assert!(text.contains("\"fieldQuality\""));
        assert!(text.contains("\"field\":\"markPrice\""));
        assert!(text.contains("\"status\":\"missing\""));
        assert!(text.contains("\"rowHealth\""));
        assert!(text.contains("\"lastSuccessMs\":32"));
        assert!(text.contains("\"requestId\":\"req-2\""));
        assert_position_binding_json(&text);
    }

    fn assert_balance_binding_json(text: &str) {
        assert!(text.contains("\"accountBindings\""));
        assert!(text.contains("\"accountScope\":\"cross_margin\""));
        assert!(text.contains("\"freshnessMs\":2"));
        assert!(text.contains("\"credentialFingerprint\":\"hmac-sha256:"));
    }

    fn assert_position_binding_json(text: &str) {
        assert!(text.contains("\"status\":\"unverified\""));
        assert!(text.contains("\"ACCOUNT_SCOPE_UNVERIFIED\""));
    }

    #[test]
    fn venue_open_orders_envelope_serializes_row_health_contract() {
        let envelope = VenueOpenOrdersEnvelope::new(
            vec![OrderInfo {
                order_id: "order-1".to_owned(),
                symbol: "BTCUSDT".to_owned(),
                exchange: "mock".to_owned(),
                side: OrderSide::Buy,
                order_type: OrderType::Limit,
                status: OrderStatus::Open,
                quantity: 1.0,
                price: 10.0,
                filled_quantity: 0.0,
                filled_price: 0.0,
                fees: 0.0,
                created_at: Utc::now(),
                execution_style: None,
                venue_time_in_force: None,
                client_order_id: None,
                reduce_only: None,
            }],
            ListStatus::Degraded,
            "account_open_orders_runtime",
            42,
            vec![ApiProblem::new(
                "OPEN_ORDER_EVIDENCE_MISSING",
                "missing evidence",
            )],
            Vec::new(),
        )
        .with_row_health(vec![AccountDataHealth {
            subject: AccountFieldSubject::open_order("mock", "order-1", "BTCUSDT", "buy"),
            source: "account_open_orders_runtime".to_owned(),
            observed_at_ms: 42,
            freshness_ms: Some(10),
            last_success_ms: Some(32),
            last_error: Some(ApiProblem::new("OPEN_ORDER_READ_DEGRADED", "rate limited")),
            retry_after_ms: Some(2_000),
            request_id: Some("req-3".to_owned()),
        }]);

        let text = serde_json::to_string(&envelope).expect("serialize open orders envelope");

        assert!(text.contains("\"rowCount\":1"));
        assert!(text.contains("\"operationHealth\""));
        assert!(text.contains("\"rowHealth\""));
        assert!(text.contains("\"kind\":\"open_order\""));
        assert!(text.contains("\"orderId\":\"order-1\""));
        assert!(text.contains("\"lastSuccessMs\":32"));
        assert!(text.contains("\"requestId\":\"req-3\""));
    }

    #[test]
    fn account_state_snapshot_serializes_field_quality_contract() {
        let balances = VenueBalanceEnvelope::new(
            Vec::new(),
            ListStatus::Degraded,
            "account_balance_runtime",
            42,
            Vec::new(),
            Vec::new(),
        );
        let positions = VenuePositionEnvelope::new(
            Vec::new(),
            ListStatus::Degraded,
            "account_position_runtime",
            42,
            Vec::new(),
            Vec::new(),
        );
        let snapshot = AccountStateSnapshot {
            balances,
            positions,
            open_orders: VenueOpenOrdersEnvelope::default(),
            status: ListStatus::Degraded,
            source: "account_state_runtime".to_owned(),
            observed_at_ms: 42,
            problems: Vec::new(),
            operation_health: Vec::new(),
            field_quality: vec![AccountFieldQuality::new(
                AccountFieldSubject::account("mock"),
                "equity",
                AccountFieldQualityStatus::Estimated,
                "account_state_runtime",
                Some(42),
            )],
            account_bindings: Vec::new(),
        };

        let text = serde_json::to_string(&snapshot).expect("serialize account state snapshot");

        assert!(text.contains("\"fieldQuality\""));
        assert!(text.contains("\"kind\":\"account\""));
        assert!(text.contains("\"field\":\"equity\""));
    }

    #[test]
    fn order_stream_payload_decodes_reconcile_without_record() {
        let payload = serde_json::json!({
            "event": "order_reconcile_diff",
            "diffs": [{
                "kind": "remote_missing",
                "exchangeOrderId": "ex-1",
                "internalOrderId": "order-1",
                "localState": "accepted",
                "localQuantity": 1.0
            }],
            "diffCount": 1,
            "timestampMs": 42
        });

        let decoded = serde_json::from_value::<OrderStreamPayload>(payload)
            .expect("reconcile payload decodes without record");

        let OrderStreamPayload::Reconcile(event) = decoded else {
            panic!("expected reconcile payload");
        };
        assert_eq!(event.event, "order_reconcile_diff");
        assert_eq!(event.diff_count, 1);
        assert_eq!(event.diffs[0].kind, OrderReconcileDiffKind::RemoteMissing);
    }
}
