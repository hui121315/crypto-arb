use super::{
    AdapterCredentials, KrakenAdapterCredentials, SelectAdapterError, TradingService,
    LIVE_ROUTER_ADAPTER_ID,
};
#[cfg(test)]
use super::{
    BINANCE_LIVE_ADAPTER_ID, BITGET_LIVE_ADAPTER_ID, BYBIT_LIVE_ADAPTER_ID, GATE_LIVE_ADAPTER_ID,
    HYPERLIQUID_LIVE_ADAPTER_ID, KUCOIN_LIVE_ADAPTER_ID, OKX_LIVE_ADAPTER_ID,
};
use exchange::{
    Binance, BinanceConfig, BinanceCredentials, Bitget, BitgetConfig, BitgetCredentials, Bybit,
    BybitConfig, BybitCredentials, ExchangeCapabilities, ExchangeError, ExchangeResult, Gate,
    GateConfig, GateCredentials, GateCrossEx, GateCrossExConfig, GateCrossExCredentials,
    Hyperliquid, HyperliquidConfig, HyperliquidCredentials, Kraken, KrakenConfig,
    KrakenCredentials, KrakenFuturesCredentials, KrakenSpotCredentials, Kucoin, KucoinConfig,
    KucoinCredentials, LiveTradingAdapter, OkxLive, OkxLiveConfig, OkxLiveCredentials,
    VenueAccountRead,
};
use futures::{stream::FuturesUnordered, StreamExt};
use parking_lot::Mutex;
use shared_types::{
    is_hyperliquid_builder_venue, normalized_venue_name, CancelOrderRequest, ExecutionEnvironment,
    FundingPaymentData, MarginMode, OrderAck, OrderInfo, OrderIntent, OrderSubmissionContext,
    PositionInfo, TradingAdapterCapabilities, TradingVenueCapability, VenueAccountModeInfo,
    VenueBalanceInfo,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use trading::RiskConfig;

type LiveAdapter = Arc<dyn LiveTradingAdapter>;
type LiveRouteMap = BTreeMap<String, LiveAdapter>;

const HYPERLIQUID_LIVE_MARKETS: &[exchange::HyperliquidMarket] =
    exchange::HyperliquidMarket::CONFIGURED_MARKETS;
const ACCOUNT_EVIDENCE_ROUTE_TIMEOUT: Duration = Duration::from_secs(10);
const BALANCE_ROUTE_TIMEOUT: Duration = Duration::from_millis(3_500);
// Five-minute ledger reconciliation includes shared rate-limiter queue time and never gates execution.
const FUNDING_PAYMENT_ROUTE_TIMEOUT: Duration = Duration::from_secs(10);
const HYPERLIQUID_OPEN_ORDER_ROUTE_TIMEOUT: Duration = Duration::from_secs(10);
const HYPERLIQUID_ACCOUNT_ROUTE_TIMEOUT: Duration = Duration::from_secs(6);
const OPEN_ORDER_ROUTE_TIMEOUT: Duration = Duration::from_millis(3_500);
const POSITION_ROUTE_TIMEOUT: Duration = Duration::from_millis(3_500);
const PRIVATE_READ_CONCURRENCY: usize = 4;

mod constructors;
mod failures;
#[cfg(test)]
mod route_tests;
mod router;
mod routing;
mod select;
mod selection;

use constructors::*;
pub(crate) use failures::*;
pub(crate) use router::LiveVenueRouter;
pub(crate) use routing::*;
