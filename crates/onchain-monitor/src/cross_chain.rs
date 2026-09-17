use shared_types::{OnchainComparisonConfig, OnchainUnsignedTransaction};

use crate::ProviderQuote;

#[derive(Debug, Clone, PartialEq)]
pub struct OnchainBridgeQuote {
    pub provider: String,
    pub route_id: String,
    pub transaction_id: String,
    pub tool: String,
    pub route_tools: Vec<String>,
    pub from_chain_id: u64,
    pub to_chain_id: u64,
    pub from_token: String,
    pub to_token: String,
    pub from_address: String,
    pub to_address: String,
    pub from_amount_raw: String,
    pub to_amount_raw: String,
    pub to_amount_min_raw: String,
    pub fee_usd: Option<f64>,
    pub gas_usd: Option<f64>,
    pub execution_duration_seconds: Option<u64>,
    pub approval_address: Option<String>,
    pub transaction: OnchainUnsignedTransaction,
    pub official_docs_url: String,
    pub observed_at_ms: i64,
    pub valid_until_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnchainCrossChainQuoteSet {
    pub peer_item_id: String,
    pub source_chain: String,
    pub source_provider: String,
    pub source_base_address: String,
    pub source_quote_address: String,
    pub source_wallet_address: String,
    pub peer_chain: String,
    pub peer_provider: String,
    pub peer_base_address: String,
    pub peer_quote_address: String,
    pub peer_wallet_address: String,
    pub source_swap: ProviderQuote,
    pub outbound_bridge: OnchainBridgeQuote,
    pub target_swap: ProviderQuote,
    pub return_bridge: OnchainBridgeQuote,
    pub observed_at_ms: i64,
    pub request_latency_ms: i64,
}

impl OnchainCrossChainQuoteSet {
    pub fn matches_configs(
        &self,
        source: &OnchainComparisonConfig,
        peer_item_id: &str,
        peer: &OnchainComparisonConfig,
    ) -> bool {
        source.cross_chain.enabled
            && self.peer_item_id == peer_item_id
            && self.source_chain.eq_ignore_ascii_case(&source.chain)
            && self.source_provider.eq_ignore_ascii_case(&source.provider)
            && self
                .source_base_address
                .eq_ignore_ascii_case(&source.base_mint)
            && self
                .source_quote_address
                .eq_ignore_ascii_case(&source.quote_mint)
            && self
                .source_wallet_address
                .eq_ignore_ascii_case(&source.wallet_address)
            && self.peer_chain.eq_ignore_ascii_case(&peer.chain)
            && self.peer_provider.eq_ignore_ascii_case(&peer.provider)
            && self.peer_base_address.eq_ignore_ascii_case(&peer.base_mint)
            && self
                .peer_quote_address
                .eq_ignore_ascii_case(&peer.quote_mint)
            && self
                .peer_wallet_address
                .eq_ignore_ascii_case(&peer.wallet_address)
    }
}
