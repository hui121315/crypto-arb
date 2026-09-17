use super::*;

#[derive(Debug, Default, Clone)]
pub(crate) struct AdapterCredentials {
    pub(crate) binance_live: Option<(String, String)>,
    pub(crate) bitget_live: Option<(String, String, String)>,
    pub(crate) bybit_live: Option<(String, String)>,
    pub(crate) gate_live: Option<(String, String)>,
    pub(crate) gate_crossex_live: Option<(String, String)>,
    pub(crate) hyperliquid_live: Option<HyperliquidAdapterCredentials>,
    pub(crate) kucoin_live: Option<(String, String, String)>,
    pub(crate) kraken_live: Option<KrakenAdapterCredentials>,
    pub(crate) okx_live: Option<(String, String, String)>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct KrakenAdapterCredentials {
    pub(crate) spot: Option<(String, String)>,
    pub(crate) futures: Option<(String, String)>,
}

impl KrakenAdapterCredentials {
    pub(crate) fn is_configured(&self) -> bool {
        self.spot.is_some() || self.futures.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HyperliquidAdapterCredentials {
    pub(crate) account_address: String,
    pub(crate) private_key: String,
    pub(crate) vault_address: Option<String>,
}

#[derive(Debug, Error)]
pub(crate) enum SelectAdapterError {
    #[error(
        "cannot switch trading adapter while open orders exist unless the kill switch is active"
    )]
    OpenOrders,
    #[error("missing adapter credentials")]
    MissingCredentials,
    #[error("unsupported adapter: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Exchange(#[from] exchange::ExchangeError),
}

pub(crate) struct TradingService {
    pub(super) engine: Arc<ExecutionEngine>,
    pub(super) journal: Arc<OrderJournal>,
    pub(super) risk: RiskEngine,
    pub(super) live_order_proof_health:
        Arc<crate::services::live_order_proof_health::LiveOrderProofHealthStore>,
    pub(super) adapter_name: RwLock<&'static str>,
    pub(super) account_reader: ArcSwapOption<live_adapters::LiveVenueRouter>,
    pub(super) account_cache_epoch: AtomicU64,
    pub(super) balance_cache: VenueBalanceCache,
    pub(super) account_summaries: DashMap<String, VenueAccountSummary>,
    pub(super) asset_valuations: DashMap<(String, String), VenueAssetValuation>,
    pub(super) account_evidence_refresh_after_ms: DashMap<String, i64>,
    pub(super) balance_fetch_locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
    pub(super) balance_fetch_backoffs: DashMap<String, BalanceFetchBackoff>,
    pub(super) open_order_fetch_lock: tokio::sync::Mutex<()>,
    pub(super) open_order_fetch_backoffs: DashMap<String, BalanceFetchBackoff>,
    pub(super) open_order_cache: VenueOpenOrderCache,
    pub(super) position_fetch_lock: tokio::sync::Mutex<()>,
    pub(super) position_fetch_backoffs: DashMap<String, BalanceFetchBackoff>,
    pub(super) position_cache: VenuePositionCache,
    pub(super) route_failures: Arc<RouteFailureSink>,
    pub(super) latest_funding_payment_ingest: ArcSwapOption<PrivateFundingPaymentIngestReport>,
}

#[derive(Debug, Default)]
pub(super) struct BalanceCacheRead {
    pub(super) merged: Vec<VenueBalanceInfo>,
    pub(super) fresh_set: HashSet<String>,
    pub(super) missing: Vec<String>,
}

#[derive(Debug)]
pub(super) struct BalanceReplaySeed {
    pub(super) rows: Vec<VenueBalanceInfo>,
    pub(super) observed_at_ms: i64,
}

pub(super) enum ConfiguredBalanceFetch {
    Credentials(Box<AdapterCredentials>),
    #[cfg(test)]
    Adapter,
}

#[derive(Debug, Clone)]
pub(super) struct BalanceFetchBackoff {
    pub(super) retry_until_ms: i64,
    pub(super) error: CachedExchangeError,
}

#[derive(Debug, Clone)]
pub(super) enum CachedExchangeError {
    Network(String),
    Timeout {
        seconds: u64,
    },
    RateLimited {
        retry_after_secs: u64,
    },
    Auth(String),
    Http {
        status: u16,
        body: String,
    },
    Parse(String),
    Api {
        exchange: String,
        code: String,
        message: String,
    },
    WsClosed(String),
    CircuitBreaker {
        exchange: String,
    },
    UnsupportedSymbol(String),
    UnsupportedCapability(&'static str),
    NotImplemented(&'static str),
}

impl BalanceCacheRead {
    pub(super) fn push_fresh(&mut self, venue: &str, rows: Vec<VenueBalanceInfo>) {
        self.merged.extend(rows);
        self.fresh_set.insert(venue.to_owned());
    }

    pub(super) fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }

    pub(super) fn merge_fetched(&mut self, rows: Vec<VenueBalanceInfo>) {
        self.merged.extend(
            rows.into_iter()
                .filter(|row| !self.fresh_set.contains(&row.venue)),
        );
    }
}

impl BalanceReplaySeed {
    pub(super) fn new(observed_at_ms: i64) -> Self {
        Self {
            rows: Vec::new(),
            observed_at_ms,
        }
    }

    pub(super) fn push(&mut self, row: VenueBalanceInfo, observed_at_ms: i64) {
        self.rows.push(row);
        self.observed_at_ms = self.observed_at_ms.max(observed_at_ms);
    }
}

impl CachedExchangeError {
    pub(super) fn to_exchange_error(&self) -> exchange::ExchangeError {
        match self {
            Self::Network(message) => exchange::ExchangeError::Network(message.clone()),
            Self::Timeout { seconds } => exchange::ExchangeError::Timeout { seconds: *seconds },
            Self::RateLimited { retry_after_secs } => exchange::ExchangeError::RateLimited {
                retry_after_secs: *retry_after_secs,
            },
            Self::Auth(message) => exchange::ExchangeError::Auth(message.clone()),
            Self::Http { status, body } => exchange::ExchangeError::Http {
                status: *status,
                body: body.clone(),
            },
            Self::Parse(message) => exchange::ExchangeError::Parse(message.clone()),
            Self::Api {
                exchange,
                code,
                message,
            } => exchange::ExchangeError::Api {
                exchange: exchange.clone(),
                code: code.clone(),
                message: message.clone(),
            },
            Self::WsClosed(message) => exchange::ExchangeError::WsClosed(message.clone()),
            Self::CircuitBreaker { exchange } => exchange::ExchangeError::CircuitBreaker {
                exchange: exchange.clone(),
            },
            Self::UnsupportedSymbol(symbol) => {
                exchange::ExchangeError::UnsupportedSymbol(symbol.clone())
            }
            Self::UnsupportedCapability(capability) => {
                exchange::ExchangeError::UnsupportedCapability(capability)
            }
            Self::NotImplemented(feature) => exchange::ExchangeError::NotImplemented(feature),
        }
    }
}

impl From<&exchange::ExchangeError> for CachedExchangeError {
    fn from(error: &exchange::ExchangeError) -> Self {
        match error {
            exchange::ExchangeError::Network(message) => Self::Network(message.clone()),
            exchange::ExchangeError::Timeout { seconds } => Self::Timeout { seconds: *seconds },
            exchange::ExchangeError::RateLimited { retry_after_secs } => Self::RateLimited {
                retry_after_secs: *retry_after_secs,
            },
            exchange::ExchangeError::Auth(message) => Self::Auth(message.clone()),
            exchange::ExchangeError::Http { status, body } => Self::Http {
                status: *status,
                body: body.clone(),
            },
            exchange::ExchangeError::Parse(message) => Self::Parse(message.clone()),
            exchange::ExchangeError::Api {
                exchange,
                code,
                message,
            } => Self::Api {
                exchange: exchange.clone(),
                code: code.clone(),
                message: message.clone(),
            },
            exchange::ExchangeError::WsClosed(message) => Self::WsClosed(message.clone()),
            exchange::ExchangeError::CircuitBreaker { exchange } => Self::CircuitBreaker {
                exchange: exchange.clone(),
            },
            exchange::ExchangeError::UnsupportedSymbol(symbol) => {
                Self::UnsupportedSymbol(symbol.clone())
            }
            exchange::ExchangeError::UnsupportedCapability(capability) => {
                Self::UnsupportedCapability(capability)
            }
            exchange::ExchangeError::NotImplemented(feature) => Self::NotImplemented(feature),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountCacheQuality {
    Fresh,
    Stale,
    Expired,
    WrongEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountCacheSnapshot {
    pub(crate) venue: String,
    pub(crate) rows: u64,
    pub(crate) freshness_ms: i64,
    pub(crate) observed_at_ms: i64,
    pub(crate) quality: AccountCacheQuality,
}
