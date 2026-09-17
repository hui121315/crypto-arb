use shared_types::{OnchainComparisonConfig, OnchainQuoteEvidence};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderQuote {
    pub input_address: String,
    pub output_address: String,
    pub input_amount_raw: String,
    pub output_amount_raw: String,
    pub router: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnchainQuotePair {
    pub chain: String,
    pub provider: String,
    pub endpoint: String,
    pub official_docs_url: String,
    pub base_address: String,
    pub quote_address: String,
    pub forward: ProviderQuote,
    pub reverse: ProviderQuote,
    pub observed_at_ms: i64,
    pub request_latency_ms: i64,
    pub quote_interval_ms: i64,
}

impl OnchainQuotePair {
    pub fn matches_config(&self, config: &OnchainComparisonConfig) -> bool {
        self.chain.eq_ignore_ascii_case(&config.chain)
            && self.provider == config.provider
            && self.base_address.eq_ignore_ascii_case(&config.base_mint)
            && self.quote_address.eq_ignore_ascii_case(&config.quote_mint)
    }

    pub fn evidence(&self) -> Vec<OnchainQuoteEvidence> {
        [&self.forward, &self.reverse]
            .into_iter()
            .map(|quote| OnchainQuoteEvidence {
                provider: self.provider.clone(),
                endpoint: self.endpoint.clone(),
                official_docs_url: self.official_docs_url.clone(),
                input_mint: quote.input_address.clone(),
                output_mint: quote.output_address.clone(),
                input_amount_raw: quote.input_amount_raw.clone(),
                output_amount_raw: quote.output_amount_raw.clone(),
                router: quote.router.clone(),
                transaction_requested: false,
                observed_at_ms: self.observed_at_ms,
            })
            .collect()
    }
}
