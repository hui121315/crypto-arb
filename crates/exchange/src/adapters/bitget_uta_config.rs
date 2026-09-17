//! Bitget V3 / Unified Trading Account (UTA) shared constants and request types.
//!
//! V3 UTA fully replaces Bitget V2 endpoints. Field naming, REST paths and WS
//! arg shapes all change between the two generations, so V3 lives in its own
//! module tree (`bitget_uta_*`) until the V2 modules can be retired.
//!
//! Official references:
//! - REST quick start: <https://www.bitget.com/api-doc/uta/guide>
//! - REST tickers: <https://www.bitget.com/api-doc/uta/public/Tickers>
//! - REST funding rate: <https://www.bitget.com/api-doc/uta/public/Get-Current-Fund-Rate>
//! - REST orderbook: <https://www.bitget.com/api-doc/uta/public/Get-OrderBook>
//! - WS intro: <https://www.bitget.com/api-doc/uta/websocket/Intro>
//! - WS public tickers channel: <https://www.bitget.com/api-doc/uta/websocket/public/Tickers-Channel>
//! - Changelog 2025-08-12 (added `nextFundingTime` to futures ticker): <https://www.bitget.com/api-doc/uta/changelog>

use serde::Serialize;

// -- REST endpoints ---------------------------------------------------------

/// Production REST host. UTA REST paths are all under `/api/v3/...`.
pub(super) const PROD_BASE: &str = "https://api.bitget.com";

// -- WebSocket endpoints ----------------------------------------------------

/// Production public market data WebSocket.
pub(super) const PROD_WS_PUBLIC: &str = "wss://ws.bitget.com/v3/ws/public";

/// Production private (account / order / position / fill) WebSocket.
///
/// Authentication payload is documented in §"Login" of the UTA WS intro;
/// connection requires the account to be in Unified Trading Account mode.
pub(super) const PROD_WS_PRIVATE: &str = "wss://ws.bitget.com/v3/ws/private";

/// Demo trading WebSocket endpoints. Wired here for parity with production
/// constants; not used by the live runtime.
#[cfg(test)]
pub(super) const DEMO_WS_PUBLIC: &str = "wss://wspap.bitget.com/v3/ws/public";
#[cfg(test)]
pub(super) const DEMO_WS_PRIVATE: &str = "wss://wspap.bitget.com/v3/ws/private";

// -- Category enum ----------------------------------------------------------

/// V3 UTA category routing key.
///
/// Used in two distinct shapes:
/// - REST query `category`: **upper-case** (`USDT-FUTURES`, `USDC-FUTURES`,
///   `COIN-FUTURES`, `SPOT`).
/// - WS trade `category`: **lower-case** (`usdt-futures`, `usdc-futures`,
///   `coin-futures`, `spot`). Private data subscriptions use `UTA` instead.
///
/// Mixing cases between REST and WS is a documented Bitget convention; the
/// helper methods below pin the casing so callers cannot accidentally swap
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BitgetUtaCategory {
    UsdtFutures,
    UsdcFutures,
    CoinFutures,
    Spot,
}

impl BitgetUtaCategory {
    /// REST query value (upper-case, e.g. `USDT-FUTURES`).
    pub(super) const fn as_query(self) -> &'static str {
        match self {
            Self::UsdtFutures => "USDT-FUTURES",
            Self::UsdcFutures => "USDC-FUTURES",
            Self::CoinFutures => "COIN-FUTURES",
            Self::Spot => "SPOT",
        }
    }

    /// WS args `instType` value (lower-case, e.g. `usdt-futures`).
    ///
    pub(super) const fn as_ws_inst_type(self) -> &'static str {
        match self {
            Self::UsdtFutures => "usdt-futures",
            Self::UsdcFutures => "usdc-futures",
            Self::CoinFutures => "coin-futures",
            Self::Spot => "spot",
        }
    }
}

// -- WebSocket subscribe args ----------------------------------------------

/// One entry of the `args` array for a V3 UTA subscribe / unsubscribe op.
///
/// Compared to V2:
/// - V2 used `{instType, channel, instId}`.
/// - V3 UTA uses `{instType, topic, symbol}` and lower-cases `instType`.
///
/// Source: ccxt-bridge SDK `websocket-client-v3.ts` confirms this shape:
/// `{op: "subscribe", args: [{instType: "spot", topic: "ticker", symbol: "BTCUSDT"}]}`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct BitgetUtaWsArgs {
    /// Lower-case category (`usdt-futures`, `spot`, ...).
    #[serde(rename = "instType")]
    pub(super) inst_type: &'static str,
    /// Channel name (`ticker`, `orderbook`, `publicTrade`, `account`, ...).
    pub(super) topic: &'static str,
    /// Trading pair (`BTCUSDT`, `ETHUSDT`, ...). Always upper-case.
    pub(super) symbol: String,
}

impl BitgetUtaWsArgs {
    pub(super) fn new(category: BitgetUtaCategory, topic: &'static str, symbol: &str) -> Self {
        Self {
            inst_type: category.as_ws_inst_type(),
            topic,
            symbol: symbol.to_ascii_uppercase(),
        }
    }
}

#[cfg(test)]
#[path = "bitget_uta_config_tests.rs"]
mod tests;
