//! Gate adapter configuration types.

use crate::venue_spec::VenueId;

pub(super) const PROD_BASE: &str = "https://api.gateio.ws";
pub(super) const PROD_WS_TRADE: &str = "wss://fx-ws.gateio.ws/v4/ws/usdt";
pub(super) const SPOT_PROD_WS_TRADE: &str = "wss://api.gateio.ws/ws/v4/";
pub(super) const SPOT_TESTNET_WS_TRADE: &str = "wss://ws-testnet.gate.com/v4/ws/spot";
/// Gate.io futures testnet endpoint.
///
/// Official announcement: <https://www.gate.com/docs/developers/futures/testnet/>
pub(super) const TESTNET_BASE: &str = "https://api-testnet.gateapi.io";
pub(super) const TESTNET_WS_TRADE: &str = "wss://ws-testnet.gate.com/v4/ws/futures/usdt";

#[derive(Debug, Clone)]
pub struct GateCredentials {
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Debug, Clone)]
pub struct GateConfig {
    pub credentials: Option<GateCredentials>,
    pub allow_live_writes: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    pub base_url_override: Option<String>,
    /// Enables Gate.io futures testnet when `base_url_override` is not set.
    pub testnet: bool,
}

impl Default for GateConfig {
    fn default() -> Self {
        let defaults = VenueId::Gate.defaults();
        Self {
            credentials: None,
            allow_live_writes: false,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            base_url_override: None,
            testnet: false,
        }
    }
}
