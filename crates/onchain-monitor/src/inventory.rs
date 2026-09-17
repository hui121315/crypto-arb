use shared_types::OnchainComparisonConfig;

#[derive(Debug, Clone, PartialEq)]
pub struct OnchainWalletAssetBalance {
    pub address: String,
    pub available: Option<f64>,
    pub source: String,
    pub problem: Option<String>,
}

impl OnchainWalletAssetBalance {
    pub fn available(
        address: impl Into<String>,
        available: f64,
        source: impl Into<String>,
    ) -> Self {
        Self {
            address: address.into(),
            available: Some(available),
            source: source.into(),
            problem: None,
        }
    }

    pub fn unavailable(
        address: impl Into<String>,
        source: impl Into<String>,
        problem: impl Into<String>,
    ) -> Self {
        Self {
            address: address.into(),
            available: None,
            source: source.into(),
            problem: Some(problem.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnchainWalletInventory {
    pub chain: String,
    pub wallet_address: String,
    pub base: OnchainWalletAssetBalance,
    pub quote: OnchainWalletAssetBalance,
    pub gas: OnchainWalletAssetBalance,
    pub observed_at_ms: i64,
}

impl OnchainWalletInventory {
    pub fn matches_config(&self, config: &OnchainComparisonConfig) -> bool {
        self.chain.eq_ignore_ascii_case(&config.chain)
            && self
                .wallet_address
                .eq_ignore_ascii_case(&config.wallet_address)
            && self.base.address.eq_ignore_ascii_case(&config.base_mint)
            && self.quote.address.eq_ignore_ascii_case(&config.quote_mint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_is_bound_to_wallet_chain_and_token_addresses() {
        let mut config = OnchainComparisonConfig {
            wallet_address: "wallet-a".to_owned(),
            ..OnchainComparisonConfig::default()
        };
        let inventory = OnchainWalletInventory {
            chain: config.chain.clone(),
            wallet_address: config.wallet_address.clone(),
            base: OnchainWalletAssetBalance::available(&config.base_mint, 1.0, "rpc"),
            quote: OnchainWalletAssetBalance::available(&config.quote_mint, 100.0, "rpc"),
            gas: OnchainWalletAssetBalance::available("native-gas", 0.1, "rpc"),
            observed_at_ms: 1,
        };

        assert!(inventory.matches_config(&config));
        config.wallet_address = "wallet-b".to_owned();
        assert!(!inventory.matches_config(&config));
    }
}
