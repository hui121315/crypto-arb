use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueOperationStatus {
    Ok,
    Warn,
    Blocked,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueOperationClass {
    Api,
    PrivateWs,
    AppWs,
    MarketData,
    BackgroundTask,
    Storage,
    Unknown,
}

impl VenueOperationClass {
    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::Api => "API",
            Self::PrivateWs => "私有 WS",
            Self::AppWs => "应用 WS",
            Self::MarketData => "行情",
            Self::BackgroundTask => "后台任务",
            Self::Storage => "存储",
            Self::Unknown => "未知",
        }
    }

    pub const fn product_explanation_zh(self) -> &'static str {
        match self {
            Self::Api => "用于判断交易所 REST、凭证、账户读取、下单权限或查询能力是否可用。",
            Self::PrivateWs => "用于判断交易所私有 WebSocket 会话、订阅和订单/账户事件流是否可用。",
            Self::AppWs => "用于判断浏览器与 CROSSLINE 后端之间的频道广播是否发生积压和丢帧。",
            Self::MarketData => "用于判断交易所行情热路径或冷启动基线数据是否可用。",
            Self::BackgroundTask => "用于判断后端周期任务是否仍在按预期运行。",
            Self::Storage => "用于判断历史、净值或执行账本等本地持久化是否可用。",
            Self::Unknown => "未被共享契约识别，不能用于健康汇总或执行准入。",
        }
    }
}

pub const OP_PRIVATE_READ: &str = "private_read";
pub const OP_ORDER_WRITE: &str = "order_write";
pub const OP_ORDER_FINALITY: &str = "order_finality";
pub const OP_BALANCE: &str = "balance";
pub const OP_POSITIONS: &str = "positions";
pub const OP_ORDER_RECONCILIATION: &str = "order_reconciliation";
pub const OP_PRIVATE_WS_SESSION: &str = "private_ws_session";
pub const OP_PRIVATE_WS_SUBSCRIBE: &str = "private_ws_subscribe";
pub const OP_PRIVATE_WS_ORDER_STREAM: &str = "private_ws_order_stream";
pub const OP_PRIVATE_WS_ACCOUNT_STREAM: &str = "private_ws_account_stream";
pub const OP_REST_ORDERBOOKS: &str = "rest_orderbooks";
pub const OP_REST_FUNDING_RATES: &str = "rest_funding_rates";
pub const OP_REST_INDEX_COMPOSITIONS: &str = "rest_index_compositions";
pub const OP_REST_INSTRUMENT_SPECS: &str = "rest_instrument_specs";
pub const OP_REST_METADATA: &str = "rest_metadata";
pub const OP_REST_PERP_TICKERS: &str = "rest_perp_tickers";
pub const OP_REST_SPOT_TICKS: &str = "rest_spot_ticks";
pub const OP_REST_FUNDING_FALLBACK: &str = "rest_funding_fallback";
pub const OP_REST_TICKER_FALLBACK: &str = "rest_ticker_fallback";
pub const OP_WS_FUNDING: &str = "ws_funding";
pub const OP_WS_FUNDING_SUBSCRIBE: &str = "ws_funding_subscribe";
pub const OP_WS_FUNDING_SNAPSHOT: &str = "ws_funding_snapshot";
pub const OP_WS_TICKER: &str = "ws_ticker";
pub const OP_WS_TICKER_SUBSCRIBE: &str = "ws_ticker_subscribe";
pub const OP_WS_TICKER_SNAPSHOT: &str = "ws_ticker_snapshot";
pub const OP_WS_SPOT_SNAPSHOT: &str = "ws_spot_snapshot";
pub const OP_OPPORTUNITY_SNAPSHOT: &str = "opportunity_snapshot";
pub const OP_WATCHLIST_PREWARM: &str = "watchlist_prewarm";
pub const OP_APP_WS_BROADCAST_PREFIX: &str = "app_ws_broadcast:";
pub const OP_BACKGROUND_TASKS: &str = "background_tasks";
pub const OP_STORAGE_AUDIT_LOG: &str = "storage:audit_log";
pub const OP_STORAGE_HISTORY: &str = "storage:history";
pub const OP_STORAGE_PORTFOLIO_NAV: &str = "storage:portfolio_nav";
pub const OP_STORAGE_EXECUTION_LEDGER: &str = "storage:execution_ledger";
pub const OP_STORAGE_ORDER_SNAPSHOT: &str = "storage:order_snapshot";
pub const OP_STORAGE_TRADING_SQL_MIGRATIONS: &str = "storage:trading_sql_migrations";
pub const OP_STORAGE_TRADING_SQL_LEDGER: &str = "storage:trading_sql_ledger";
pub const OP_STORAGE_WATCHLIST_ALERTS: &str = "storage:watchlist_alerts";
pub const OP_CREDENTIAL_PROBE_PREFIX: &str = "credential_probe:";
pub const OP_HTTP_REST_PREFIX: &str = "http_rest:";
pub const OP_HOST_GATE_PREFIX: &str = "host_gate:";
pub const OP_RATE_LIMITER_PREFIX: &str = "rate_limiter:";
pub const OP_BACKGROUND_TASK_PREFIX: &str = "background_task:";

pub fn credential_probe_operation(kind: &str) -> String {
    format!("{OP_CREDENTIAL_PROBE_PREFIX}{kind}")
}
