//! KuCoin adapter configuration types.

use crate::venue_spec::VenueId;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KucoinMarginMode {
    #[default]
    Isolated,
    Cross,
}

#[derive(Debug, Clone)]
pub struct KucoinCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
}

#[derive(Debug, Clone)]
pub struct KucoinConfig {
    pub credentials: Option<KucoinCredentials>,
    pub allow_live_writes: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    pub base_url_override: Option<String>,
    pub margin_mode: KucoinMarginMode,
    pub default_leverage: u32,
}

impl Default for KucoinConfig {
    fn default() -> Self {
        let defaults = VenueId::Kucoin.defaults();
        Self {
            credentials: None,
            allow_live_writes: false,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            base_url_override: None,
            margin_mode: KucoinMarginMode::default(),
            default_leverage: 1,
        }
    }
}
