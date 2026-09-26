mod live_adapters;
pub(crate) mod private_ws_events;
pub(crate) mod private_ws_mapper;
mod private_ws_session;
pub(crate) use private_ws_session::PrivateWsAccountLease;
mod venue_balance_cache;
mod venue_open_order_cache;
mod venue_position_cache;

use arc_swap::ArcSwapOption;
use dashmap::DashMap;
use exchange::{ExchangeCapabilities, LiveTradingAdapter, VenueAccountRead};
pub(crate) use funding_payments::PrivateFundingPaymentIngestReport;
pub(crate) use live_adapters::RouteFailure;
use live_adapters::RouteFailureSink;
use parking_lot::RwLock;
use shared_types::{
    is_hyperliquid_builder_venue, normalized_venue_name, ExecutionLedgerEvent, ExecutionMode,
    LiveOrderState, OrderIntent, OrderRecord, OrderSubmissionContext, OrderUpdateSource,
    PositionInfo, RiskDecision, VenueAccountModeInfo, VenueAccountSummary, VenueAssetValuation,
    VenueBalanceInfo, VenueId,
};
use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use trading::{
    ExecutionEngine, ExecutionLedgerOrderContext, ExecutionLedgerQuery,
    ExecutionLedgerStorageSnapshot, MockLiveAdapter, OrderJournal, OrderSnapshotStorageSnapshot,
    RiskConfig, RiskEngine, SqlBalanceLedgerReplayEvent, SqlLedgerInit, SqlLedgerStorageSnapshot,
    SqlRunFinalityLedgerEvent, TradingError, TradingResult,
};
use venue_balance_cache::VenueBalanceCache;
use venue_open_order_cache::VenueOpenOrderCache;
use venue_position_cache::VenuePositionCache;

use crate::middleware::audit;

mod adapters;
mod balances;
mod cache;
mod construct;
mod funding_payments;
mod helpers;
mod open_orders;
mod orders;
mod positions;
mod reconcile;
mod submit;
mod types;

pub(crate) const BALANCE_CACHE_TTL_MS: i64 = 15_000;
const BALANCE_CACHE_MAX_STALE_MS: i64 = 60_000;
// A portfolio snapshot is published every two seconds. Keep a failed venue out
// of the retry loop long enough for the transport circuit to recover instead
// of reissuing the same account request every few snapshots.
const BALANCE_FETCH_ERROR_BACKOFF_MS: u64 = 30_000;
// Active private WS sessions extend cache freshness. These values are only the
// bounded REST reconciliation fallback when a venue stream is unavailable.
const POSITION_CACHE_TTL_MS: i64 = 15_000;
pub(crate) const POSITION_CACHE_MAX_STALE_MS: i64 = 60_000;
const OPEN_ORDER_CACHE_TTL_MS: i64 = 30_000;
const OPEN_ORDER_CACHE_MAX_STALE_MS: i64 = 60_000;

#[cfg(test)]
pub(crate) const BINANCE_LIVE_ADAPTER_ID: &str = "binance_live";
#[cfg(test)]
pub(crate) const BITGET_LIVE_ADAPTER_ID: &str = "bitget_live";
#[cfg(test)]
pub(crate) const BYBIT_LIVE_ADAPTER_ID: &str = "bybit_live";
#[cfg(test)]
pub(crate) const GATE_LIVE_ADAPTER_ID: &str = "gate_live";
#[cfg(test)]
pub(crate) const HYPERLIQUID_LIVE_ADAPTER_ID: &str = "hyperliquid_live";
#[cfg(test)]
pub(crate) const KUCOIN_LIVE_ADAPTER_ID: &str = "kucoin_live";
pub(crate) const LIVE_ROUTER_ADAPTER_ID: &str = "live";
#[cfg(test)]
pub(crate) const OKX_LIVE_ADAPTER_ID: &str = "okx_live";

pub(crate) use helpers::*;
pub(crate) use types::*;

#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod balance_replay_tests;
#[cfg(test)]
mod tests;
