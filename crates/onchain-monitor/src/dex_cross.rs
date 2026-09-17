use shared_types::{OnchainComparisonConfig, OnchainDexComparisonDirection};

use crate::ProviderQuote;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnchainDexCrossRouteQuote {
    pub direction: OnchainDexComparisonDirection,
    pub buy: ProviderQuote,
    pub sell: ProviderQuote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnchainDexCrossQuoteSet {
    pub chain: String,
    pub base_address: String,
    pub quote_address: String,
    pub primary_provider: String,
    pub peer_provider: String,
    pub routes: Vec<OnchainDexCrossRouteQuote>,
    pub observed_at_ms: i64,
    pub request_latency_ms: i64,
}

impl OnchainDexCrossQuoteSet {
    pub fn matches_config(&self, config: &OnchainComparisonConfig) -> bool {
        config.dex_comparison.enabled
            && self.chain.eq_ignore_ascii_case(&config.chain)
            && self.base_address.eq_ignore_ascii_case(&config.base_mint)
            && self.quote_address.eq_ignore_ascii_case(&config.quote_mint)
            && self.primary_provider.eq_ignore_ascii_case(&config.provider)
            && self
                .peer_provider
                .eq_ignore_ascii_case(&config.dex_comparison.peer_provider)
    }
}
