//! Gate `CrossEx` endpoint and credential configuration.

use crate::venue_spec::VenueId;

pub(super) const CROSSEX_REST_URL: &str = "https://api.gateio.ws/api/v4";
pub(super) const CROSSEX_PUBLIC_WS_URL: &str = "wss://api.gateio.ws/ws/crossex/public";
pub(super) const CROSSEX_PRIVATE_WS_URL: &str = "wss://api.gateio.ws/ws/crossex";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCrossExCredentials {
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Debug, Clone)]
pub struct GateCrossExConfig {
    pub credentials: Option<GateCrossExCredentials>,
    pub allow_live_writes: bool,
    pub rest_url_override: Option<String>,
    pub public_ws_url_override: Option<String>,
    pub private_ws_url_override: Option<String>,
    pub qps: u32,
    pub timeout_secs: u64,
}

impl Default for GateCrossExConfig {
    fn default() -> Self {
        let defaults = VenueId::GateCrossEx.defaults();
        Self {
            credentials: None,
            allow_live_writes: false,
            rest_url_override: None,
            public_ws_url_override: None,
            private_ws_url_override: None,
            qps: defaults.qps,
            timeout_secs: defaults.timeout_secs,
        }
    }
}
