//! Kraken Spot and Derivatives endpoint configuration.

use crate::venue_spec::VenueId;

pub(super) const SPOT_REST_URL: &str = "https://api.kraken.com";
pub(super) const SPOT_PUBLIC_WS_URL: &str = "wss://ws.kraken.com/v2";
pub(super) const SPOT_PRIVATE_WS_URL: &str = "wss://ws-auth.kraken.com/v2";
pub(super) const FUTURES_REST_URL: &str = "https://futures.kraken.com";
pub(super) const FUTURES_WS_URL: &str = "wss://futures.kraken.com/ws/v1";

#[derive(Debug, Clone)]
pub struct KrakenSpotCredentials {
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Debug, Clone)]
pub struct KrakenFuturesCredentials {
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Debug, Clone, Default)]
pub struct KrakenCredentials {
    pub spot: Option<KrakenSpotCredentials>,
    pub futures: Option<KrakenFuturesCredentials>,
}

#[derive(Debug, Clone)]
pub struct KrakenConfig {
    pub credentials: Option<KrakenCredentials>,
    pub allow_live_writes: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    pub spot_rest_url_override: Option<String>,
    pub spot_public_ws_url_override: Option<String>,
    pub spot_private_ws_url_override: Option<String>,
    pub futures_rest_url_override: Option<String>,
    pub futures_ws_url_override: Option<String>,
}

impl Default for KrakenConfig {
    fn default() -> Self {
        let defaults = VenueId::Kraken.defaults();
        Self {
            credentials: None,
            allow_live_writes: false,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            spot_rest_url_override: None,
            spot_public_ws_url_override: None,
            spot_private_ws_url_override: None,
            futures_rest_url_override: None,
            futures_ws_url_override: None,
        }
    }
}
