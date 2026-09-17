//! Binance adapter configuration types and endpoint defaults.

use crate::venue_spec::VenueId;

pub(super) const PROD_BASE: &str = "https://fapi.binance.com";
pub(super) const TESTNET_BASE: &str = "https://testnet.binancefuture.com";
pub(super) const SPOT_PROD_BASE: &str = "https://api.binance.com";
pub(super) const PROD_WS_TRADE: &str = "wss://ws-fapi.binance.com/ws-fapi/v1";
pub(super) const SPOT_PROD_WS_TRADE: &str = "wss://ws-api.binance.com:443/ws-api/v3";
pub(super) const TESTNET_WS_TRADE: &str = "wss://testnet.binancefuture.com/ws-fapi/v1";
pub(super) const TIME_SYNC_INTERVAL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct BinanceCredentials {
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Debug, Clone)]
pub struct BinanceConfig {
    pub credentials: Option<BinanceCredentials>,
    pub testnet: bool,
    pub allow_live_writes: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    /// Custom base URL for tests or controlled gateways.
    pub base_url_override: Option<String>,
}

impl Default for BinanceConfig {
    fn default() -> Self {
        let defaults = VenueId::Binance.defaults();
        Self {
            credentials: None,
            testnet: false,
            allow_live_writes: false,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            base_url_override: None,
        }
    }
}
