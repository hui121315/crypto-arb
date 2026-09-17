use super::*;

#[derive(Debug)]
pub(crate) enum PrivateWsEvent {
    Order(PrivateOrderDelta),
    OpenOrders(PrivateOpenOrdersSnapshot),
    BinanceOrderTrade(Box<BinanceOrderTradeDelta>),
    Fill(PrivateFillDelta),
    FillWithEvidence(Box<PrivateFillWithEvidenceDelta>),
    Funding(PrivateFundingDelta),
    Liquidation(PrivateLiquidationDelta),
    NonUserCancel(PrivateNonUserCancelDelta),
    Positions(PrivatePositionsSnapshot),
    PositionPatch(PrivatePositionsPatch),
    Balances(Box<PrivateBalancesSnapshot>),
    BalancePatch(Box<PrivateBalancesPatch>),
    AssetValuations(Box<PrivateAssetValuationSnapshot>),
    AccountSummary(VenueAccountSummary),
    AccountDirty(PrivateAccountDirty),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrivateAccountScope {
    Balances,
    Positions,
    All,
}

impl PrivateAccountScope {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Balances => "balances",
            Self::Positions => "positions",
            Self::All => "all",
        }
    }

    pub(crate) const fn invalidates_balances(self) -> bool {
        matches!(self, Self::Balances | Self::All)
    }

    pub(crate) const fn invalidates_positions(self) -> bool {
        matches!(self, Self::Positions | Self::All)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivateAccountDirty {
    pub(crate) venue: String,
    pub(crate) scope: PrivateAccountScope,
    pub(crate) reason: String,
}

impl PrivateAccountDirty {
    pub(crate) fn new(
        venue: impl Into<String>,
        scope: PrivateAccountScope,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            venue: venue.into(),
            scope,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BinanceOrderTradeDelta {
    pub(crate) order: PrivateOrderDelta,
    pub(crate) fill: Option<PrivateFillDelta>,
    pub(crate) execution_type: String,
    pub(crate) order_status: String,
    pub(crate) reject_reason: Option<String>,
    pub(crate) terminal: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivateOrderDelta {
    pub(crate) client_order_id: String,
    pub(crate) order: OrderInfo,
    pub(crate) received_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivateOpenOrdersSnapshot {
    pub(crate) venue: String,
    pub(crate) rows: Vec<OrderInfo>,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivateFillDelta {
    pub(crate) venue: String,
    pub(crate) exchange_order_id: String,
    pub(crate) client_order_id: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) side: Option<OrderSide>,
    pub(crate) venue_event_id: String,
    pub(crate) quantity: f64,
    pub(crate) price: f64,
    pub(crate) fee_amount: Option<f64>,
    pub(crate) fee_currency: Option<String>,
    pub(crate) occurred_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivateFillWithEvidenceDelta {
    pub(crate) fill: PrivateFillDelta,
    pub(crate) transport_metadata: OrderTransportMetadata,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivateFundingDelta {
    pub(crate) venue: String,
    pub(crate) venue_event_id: String,
    pub(crate) coin: String,
    pub(crate) amount: f64,
    pub(crate) currency: String,
    pub(crate) occurred_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivateLiquidationDelta {
    pub(crate) venue: String,
    pub(crate) venue_event_id: String,
    pub(crate) liquidator: String,
    pub(crate) liquidated_user: String,
    pub(crate) notional_position: f64,
    pub(crate) account_value: f64,
    pub(crate) occurred_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivateNonUserCancelDelta {
    pub(crate) venue: String,
    pub(crate) venue_event_id: String,
    pub(crate) exchange_order_id: String,
    pub(crate) coin: String,
    pub(crate) occurred_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivatePositionsSnapshot {
    pub(crate) venue: String,
    pub(crate) rows: Vec<PositionInfo>,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivatePositionsPatch {
    pub(crate) venue: String,
    pub(crate) rows: Vec<PositionInfo>,
}

/// PR-DP-08 D-8：mapper 推送的 per-venue balance snapshot。
///
/// `venue` 是会被写入 [`super::venue_balance_cache::VenueBalanceCache`]
/// 的 venue key（例如 `"okx"` / `"bybit"` / `"bitget"` / `"hyperliquid"`），
/// 应当与 `shared_types::VenueBalanceInfo::venue` 字段对齐。
/// `rows` 是该 venue 全部 currency 的 balance 行（采取整表替换语义）。
#[derive(Debug)]
pub(crate) struct PrivateBalancesSnapshot {
    pub(crate) venue: String,
    pub(crate) rows: Vec<VenueBalanceInfo>,
}

#[derive(Debug)]
pub(crate) struct PrivateBalancesPatch {
    pub(crate) venue: String,
    pub(crate) rows: Vec<VenueBalanceInfo>,
}

#[derive(Debug)]
pub(crate) struct PrivateAssetValuationSnapshot {
    pub(crate) venue: String,
    pub(crate) rows: Vec<VenueAssetValuation>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PrivateWsApplyOutcome {
    pub(crate) order: Option<OrderRecord>,
    pub(crate) ledger_events: Vec<ExecutionLedgerEvent>,
    pub(crate) ledger_updated: bool,
    pub(crate) order_projection_handled_by_ledger: bool,
    pub(crate) funding_skip_reason: Option<shared_types::FundingPaymentIngestSkipReason>,
    pub(crate) balance_ledger_updated: bool,
    pub(crate) open_order_cache_updated: bool,
    pub(crate) account_cache_updated: bool,
    pub(crate) account_cache_dirty: Option<PrivateAccountDirty>,
}
