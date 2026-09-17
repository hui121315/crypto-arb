//! WebSocket 基础设施：通用 supervisor，10 家适配器复用其重连 / 心跳 / 熔断逻辑。

mod connect_budget;
pub mod manager;
mod proxy;
pub(crate) mod trade_session;
pub mod trading;

pub use manager::{
    ingest_snapshots, WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsIngestSnapshot,
    WsIngestStats, WsManager, WsServerPing, WsState,
};
pub use trading::{trading_ws_operation_registry, trading_ws_venues, TRADING_WS_VENUE_COUNT};
