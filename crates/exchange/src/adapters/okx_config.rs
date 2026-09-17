//! OKX adapter configuration types.

use crate::venue_spec::VenueId;

/// OKX public REST default base URL.
///
/// Official global production endpoint. Regional OKX entities may require a
/// regional endpoint; use `OkxConfig::base_url_override` for that deployment.
pub(super) const PROD_BASE: &str = "https://openapi.okx.com";
/// Concurrent window for single-instId funding-rate fanout.
pub(super) const FUNDING_RATE_CONCURRENCY: usize = 8;
pub(super) const SWAP_SUFFIX: &str = "-USDT-SWAP";

#[derive(Debug, Clone)]
pub struct OkxCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
}

#[derive(Debug, Clone)]
pub struct OkxConfig {
    pub credentials: Option<OkxCredentials>,
    pub timeout_secs: u64,
    pub qps: u32,
    pub base_url_override: Option<String>,
}

impl Default for OkxConfig {
    fn default() -> Self {
        let defaults = VenueId::Okx.defaults();
        Self {
            credentials: None,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            base_url_override: None,
        }
    }
}
