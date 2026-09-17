use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueOperationKind {
    PrivateRead,
    OrderWrite,
    OrderFinality,
    Balance,
    Positions,
    OrderReconciliation,
    CredentialProbeBalanceRead,
    CredentialProbePositionsRead,
    CredentialProbeOpenOrdersRead,
    CredentialProbeAccountModeRead,
    CredentialProbeOrderPermission,
    PrivateWsSession,
    PrivateWsSubscribe,
    PrivateWsOrderStream,
    PrivateWsAccountStream,
    AppWsBroadcast,
    HttpRest,
    HostGate,
    RateLimiter,
    RestOrderbooks,
    RestFundingRates,
    RestIndexCompositions,
    RestInstrumentSpecs,
    RestMetadata,
    RestPerpTickers,
    RestSpotTicks,
    RestFundingFallback,
    RestTickerFallback,
    WsFunding,
    WsFundingSubscribe,
    WsFundingSnapshot,
    WsTicker,
    WsTickerSubscribe,
    WsTickerSnapshot,
    WsSpotSnapshot,
    OpportunitySnapshot,
    WatchlistPrewarm,
    BackgroundTasks,
    BackgroundTask,
    StorageAuditLog,
    StorageHistory,
    StoragePortfolioNav,
    StorageExecutionLedger,
    StorageOrderSnapshot,
    StorageTradingSqlMigrations,
    StorageTradingSqlLedger,
    StorageWatchlistAlerts,
    Unknown,
}

impl VenueOperationKind {
    pub fn parse(operation: &str) -> Self {
        let operation = operation.trim();
        if let Some(kind) = operation.strip_prefix(OP_CREDENTIAL_PROBE_PREFIX) {
            return Self::from_credential_probe_kind(kind);
        }
        if operation.starts_with(OP_HTTP_REST_PREFIX) {
            return Self::HttpRest;
        }
        if operation.starts_with(OP_HOST_GATE_PREFIX) {
            return Self::HostGate;
        }
        if operation.starts_with(OP_RATE_LIMITER_PREFIX) {
            return Self::RateLimiter;
        }
        if operation.starts_with(OP_BACKGROUND_TASK_PREFIX) {
            return Self::BackgroundTask;
        }
        if operation.starts_with(OP_APP_WS_BROADCAST_PREFIX) {
            return Self::AppWsBroadcast;
        }

        match operation {
            OP_PRIVATE_READ => Self::PrivateRead,
            OP_ORDER_WRITE => Self::OrderWrite,
            OP_ORDER_FINALITY => Self::OrderFinality,
            OP_BALANCE => Self::Balance,
            OP_POSITIONS => Self::Positions,
            OP_ORDER_RECONCILIATION => Self::OrderReconciliation,
            OP_PRIVATE_WS_SESSION => Self::PrivateWsSession,
            OP_PRIVATE_WS_SUBSCRIBE => Self::PrivateWsSubscribe,
            OP_PRIVATE_WS_ORDER_STREAM => Self::PrivateWsOrderStream,
            OP_PRIVATE_WS_ACCOUNT_STREAM => Self::PrivateWsAccountStream,
            OP_REST_ORDERBOOKS => Self::RestOrderbooks,
            OP_REST_FUNDING_RATES => Self::RestFundingRates,
            OP_REST_INDEX_COMPOSITIONS => Self::RestIndexCompositions,
            OP_REST_INSTRUMENT_SPECS => Self::RestInstrumentSpecs,
            OP_REST_METADATA => Self::RestMetadata,
            OP_REST_PERP_TICKERS => Self::RestPerpTickers,
            OP_REST_SPOT_TICKS => Self::RestSpotTicks,
            OP_REST_FUNDING_FALLBACK => Self::RestFundingFallback,
            OP_REST_TICKER_FALLBACK => Self::RestTickerFallback,
            OP_WS_FUNDING => Self::WsFunding,
            OP_WS_FUNDING_SUBSCRIBE => Self::WsFundingSubscribe,
            OP_WS_FUNDING_SNAPSHOT => Self::WsFundingSnapshot,
            OP_WS_TICKER => Self::WsTicker,
            OP_WS_TICKER_SUBSCRIBE => Self::WsTickerSubscribe,
            OP_WS_TICKER_SNAPSHOT => Self::WsTickerSnapshot,
            OP_WS_SPOT_SNAPSHOT => Self::WsSpotSnapshot,
            OP_OPPORTUNITY_SNAPSHOT => Self::OpportunitySnapshot,
            OP_WATCHLIST_PREWARM => Self::WatchlistPrewarm,
            OP_BACKGROUND_TASKS => Self::BackgroundTasks,
            OP_STORAGE_AUDIT_LOG => Self::StorageAuditLog,
            OP_STORAGE_HISTORY => Self::StorageHistory,
            OP_STORAGE_PORTFOLIO_NAV => Self::StoragePortfolioNav,
            OP_STORAGE_EXECUTION_LEDGER => Self::StorageExecutionLedger,
            OP_STORAGE_ORDER_SNAPSHOT => Self::StorageOrderSnapshot,
            OP_STORAGE_TRADING_SQL_MIGRATIONS => Self::StorageTradingSqlMigrations,
            OP_STORAGE_TRADING_SQL_LEDGER => Self::StorageTradingSqlLedger,
            OP_STORAGE_WATCHLIST_ALERTS => Self::StorageWatchlistAlerts,
            _ => Self::Unknown,
        }
    }

    pub fn from_credential_probe_kind(kind: &str) -> Self {
        match kind.trim() {
            "balance_read" => Self::CredentialProbeBalanceRead,
            "positions_read" => Self::CredentialProbePositionsRead,
            "open_orders_read" => Self::CredentialProbeOpenOrdersRead,
            "account_mode_read" => Self::CredentialProbeAccountModeRead,
            "order_permission" => Self::CredentialProbeOrderPermission,
            _ => Self::Unknown,
        }
    }

    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::PrivateRead => Some(OP_PRIVATE_READ),
            Self::OrderWrite => Some(OP_ORDER_WRITE),
            Self::OrderFinality => Some(OP_ORDER_FINALITY),
            Self::Balance => Some(OP_BALANCE),
            Self::Positions => Some(OP_POSITIONS),
            Self::OrderReconciliation => Some(OP_ORDER_RECONCILIATION),
            Self::PrivateWsSession => Some(OP_PRIVATE_WS_SESSION),
            Self::PrivateWsSubscribe => Some(OP_PRIVATE_WS_SUBSCRIBE),
            Self::PrivateWsOrderStream => Some(OP_PRIVATE_WS_ORDER_STREAM),
            Self::PrivateWsAccountStream => Some(OP_PRIVATE_WS_ACCOUNT_STREAM),
            Self::RestOrderbooks => Some(OP_REST_ORDERBOOKS),
            Self::RestFundingRates => Some(OP_REST_FUNDING_RATES),
            Self::RestIndexCompositions => Some(OP_REST_INDEX_COMPOSITIONS),
            Self::RestInstrumentSpecs => Some(OP_REST_INSTRUMENT_SPECS),
            Self::RestMetadata => Some(OP_REST_METADATA),
            Self::RestPerpTickers => Some(OP_REST_PERP_TICKERS),
            Self::RestSpotTicks => Some(OP_REST_SPOT_TICKS),
            Self::RestFundingFallback => Some(OP_REST_FUNDING_FALLBACK),
            Self::RestTickerFallback => Some(OP_REST_TICKER_FALLBACK),
            Self::WsFunding => Some(OP_WS_FUNDING),
            Self::WsFundingSubscribe => Some(OP_WS_FUNDING_SUBSCRIBE),
            Self::WsFundingSnapshot => Some(OP_WS_FUNDING_SNAPSHOT),
            Self::WsTicker => Some(OP_WS_TICKER),
            Self::WsTickerSubscribe => Some(OP_WS_TICKER_SUBSCRIBE),
            Self::WsTickerSnapshot => Some(OP_WS_TICKER_SNAPSHOT),
            Self::WsSpotSnapshot => Some(OP_WS_SPOT_SNAPSHOT),
            Self::OpportunitySnapshot => Some(OP_OPPORTUNITY_SNAPSHOT),
            Self::WatchlistPrewarm => Some(OP_WATCHLIST_PREWARM),
            Self::BackgroundTasks => Some(OP_BACKGROUND_TASKS),
            Self::StorageAuditLog => Some(OP_STORAGE_AUDIT_LOG),
            Self::StorageHistory => Some(OP_STORAGE_HISTORY),
            Self::StoragePortfolioNav => Some(OP_STORAGE_PORTFOLIO_NAV),
            Self::StorageExecutionLedger => Some(OP_STORAGE_EXECUTION_LEDGER),
            Self::StorageOrderSnapshot => Some(OP_STORAGE_ORDER_SNAPSHOT),
            Self::StorageTradingSqlMigrations => Some(OP_STORAGE_TRADING_SQL_MIGRATIONS),
            Self::StorageTradingSqlLedger => Some(OP_STORAGE_TRADING_SQL_LEDGER),
            Self::StorageWatchlistAlerts => Some(OP_STORAGE_WATCHLIST_ALERTS),
            Self::CredentialProbeBalanceRead
            | Self::CredentialProbePositionsRead
            | Self::CredentialProbeOpenOrdersRead
            | Self::CredentialProbeAccountModeRead
            | Self::CredentialProbeOrderPermission
            | Self::HttpRest
            | Self::HostGate
            | Self::RateLimiter
            | Self::BackgroundTask
            | Self::AppWsBroadcast
            | Self::Unknown => None,
        }
    }

    pub const fn class(self) -> VenueOperationClass {
        match self {
            Self::PrivateRead
            | Self::OrderWrite
            | Self::OrderFinality
            | Self::Balance
            | Self::Positions
            | Self::OrderReconciliation
            | Self::CredentialProbeBalanceRead
            | Self::CredentialProbePositionsRead
            | Self::CredentialProbeOpenOrdersRead
            | Self::CredentialProbeAccountModeRead
            | Self::CredentialProbeOrderPermission
            | Self::HttpRest
            | Self::HostGate
            | Self::RateLimiter => VenueOperationClass::Api,
            Self::PrivateWsSession
            | Self::PrivateWsSubscribe
            | Self::PrivateWsOrderStream
            | Self::PrivateWsAccountStream => VenueOperationClass::PrivateWs,
            Self::AppWsBroadcast => VenueOperationClass::AppWs,
            Self::RestOrderbooks
            | Self::RestFundingRates
            | Self::RestIndexCompositions
            | Self::RestInstrumentSpecs
            | Self::RestMetadata
            | Self::RestPerpTickers
            | Self::RestSpotTicks
            | Self::RestFundingFallback
            | Self::RestTickerFallback
            | Self::WsFunding
            | Self::WsFundingSubscribe
            | Self::WsFundingSnapshot
            | Self::WsTicker
            | Self::WsTickerSubscribe
            | Self::WsTickerSnapshot
            | Self::WsSpotSnapshot
            | Self::WatchlistPrewarm => VenueOperationClass::MarketData,
            Self::OpportunitySnapshot | Self::BackgroundTasks | Self::BackgroundTask => {
                VenueOperationClass::BackgroundTask
            }
            Self::StorageAuditLog
            | Self::StorageHistory
            | Self::StoragePortfolioNav
            | Self::StorageExecutionLedger
            | Self::StorageOrderSnapshot
            | Self::StorageTradingSqlMigrations
            | Self::StorageTradingSqlLedger
            | Self::StorageWatchlistAlerts => VenueOperationClass::Storage,
            Self::Unknown => VenueOperationClass::Unknown,
        }
    }

    pub const fn is_api_status_row(self) -> bool {
        matches!(
            self,
            Self::PrivateRead
                | Self::OrderWrite
                | Self::OrderFinality
                | Self::Balance
                | Self::Positions
                | Self::OrderReconciliation
                | Self::CredentialProbeBalanceRead
                | Self::CredentialProbePositionsRead
                | Self::CredentialProbeOpenOrdersRead
                | Self::CredentialProbeAccountModeRead
                | Self::CredentialProbeOrderPermission
                | Self::HttpRest
                | Self::HostGate
                | Self::RateLimiter
        )
    }

    pub const fn is_trading_api_status_row(self) -> bool {
        matches!(
            self,
            Self::PrivateRead
                | Self::OrderWrite
                | Self::OrderFinality
                | Self::Balance
                | Self::Positions
                | Self::OrderReconciliation
                | Self::CredentialProbeBalanceRead
                | Self::CredentialProbePositionsRead
                | Self::CredentialProbeOpenOrdersRead
                | Self::CredentialProbeAccountModeRead
                | Self::CredentialProbeOrderPermission
        )
    }

    pub const fn is_api_transport_status_row(self) -> bool {
        matches!(self, Self::HttpRest | Self::HostGate | Self::RateLimiter)
    }

    pub const fn is_private_ws_status_row(self) -> bool {
        matches!(
            self,
            Self::PrivateWsSession
                | Self::PrivateWsSubscribe
                | Self::PrivateWsOrderStream
                | Self::PrivateWsAccountStream
        )
    }

    pub const fn is_market_data_execution_core_status_row(self) -> bool {
        matches!(
            self,
            Self::RestInstrumentSpecs
                | Self::WsFunding
                | Self::WsFundingSubscribe
                | Self::WsFundingSnapshot
                | Self::WsTicker
                | Self::WsTickerSubscribe
                | Self::WsTickerSnapshot
                | Self::WsSpotSnapshot
        )
    }

    pub const fn is_market_data_recovery_status_row(self) -> bool {
        matches!(
            self,
            Self::RestOrderbooks
                | Self::RestFundingRates
                | Self::RestMetadata
                | Self::RestPerpTickers
                | Self::RestSpotTicks
                | Self::RestFundingFallback
                | Self::RestTickerFallback
        )
    }

    pub const fn credential_probe_requires_private_read(self) -> bool {
        matches!(
            self,
            Self::CredentialProbeBalanceRead
                | Self::CredentialProbePositionsRead
                | Self::CredentialProbeOpenOrdersRead
                | Self::CredentialProbeAccountModeRead
        )
    }

    pub const fn credential_probe_requires_order_write(self) -> bool {
        matches!(self, Self::CredentialProbeOrderPermission)
    }

    pub const fn private_ws_requires_private_read(self) -> bool {
        matches!(
            self,
            Self::PrivateWsSession | Self::PrivateWsSubscribe | Self::PrivateWsAccountStream
        )
    }

    pub const fn private_ws_requires_order_write(self) -> bool {
        matches!(self, Self::PrivateWsOrderStream)
    }
}
