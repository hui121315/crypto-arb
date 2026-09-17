//! OKX live/demo trading adapter configuration.

use crate::venue_spec::VenueId;

pub(super) const PROD_BASE: &str = "https://openapi.okx.com";
pub(super) const PROD_WS_PRIVATE: &str = "wss://ws.okx.com:8443/ws/v5/private";
pub(super) const TESTNET_WS_PRIVATE: &str = "wss://wspap.okx.com:8443/ws/v5/private";
pub(super) const SWAP_SUFFIX: &str = "-USDT-SWAP";
pub(super) const TIME_SYNC_INTERVAL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct OkxLiveCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
}

/// OKX V5 `tdMode` for order placement.
///
/// Official docs: <https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-place-order>.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OkxTdMode {
    #[default]
    Cross,
    Isolated,
    Cash,
    SpotIsolated,
}

impl OkxTdMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Cross => "cross",
            Self::Isolated => "isolated",
            Self::Cash => "cash",
            Self::SpotIsolated => "spot_isolated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OkxLiveConfig {
    pub credentials: OkxLiveCredentials,
    pub testnet: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    pub base_url_override: Option<String>,
    /// OKX `tdMode` used for order placement. Default is cross margin.
    pub td_mode: OkxTdMode,
}

impl Default for OkxLiveConfig {
    fn default() -> Self {
        let defaults = VenueId::Okx.defaults();
        Self {
            credentials: OkxLiveCredentials {
                api_key: String::new(),
                api_secret: String::new(),
                passphrase: String::new(),
            },
            testnet: true,
            timeout_secs: defaults.timeout_secs.min(10),
            qps: 5,
            base_url_override: None,
            td_mode: OkxTdMode::Cross,
        }
    }
}
