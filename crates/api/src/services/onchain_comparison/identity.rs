use onchain_monitor::ProviderQuote;
use shared_types::{OnchainComparisonConfig, OrderBookInfo};

pub(super) fn validate_quote_identity(
    config: &OnchainComparisonConfig,
    forward: &ProviderQuote,
    reverse: &ProviderQuote,
) -> Result<(), &'static str> {
    if same_provider_address(config, &forward.input_address, &config.base_mint)
        && same_provider_address(config, &forward.output_address, &config.quote_mint)
        && same_provider_address(config, &reverse.input_address, &config.quote_mint)
        && same_provider_address(config, &reverse.output_address, &config.base_mint)
    {
        Ok(())
    } else {
        Err("provider quote identity does not match the configured token addresses")
    }
}

fn same_provider_address(
    config: &OnchainComparisonConfig,
    observed: &str,
    configured: &str,
) -> bool {
    let expected = if config.provider == "cow_protocol" {
        super::cow::quote_token_address(&config.chain, configured).unwrap_or(configured)
    } else {
        configured
    };
    same_address(&config.chain, observed, expected)
}

fn same_address(chain: &str, left: &str, right: &str) -> bool {
    if chain.eq_ignore_ascii_case("solana") {
        left == right
    } else {
        left.eq_ignore_ascii_case(right)
    }
}

pub(super) fn validate_book_identity(
    config: &OnchainComparisonConfig,
    book: &OrderBookInfo,
) -> Result<(), &'static str> {
    if onchain_monitor::normalized_pair_symbol(&book.symbol)
        == onchain_monitor::normalized_pair_symbol(&config.cex_symbol)
    {
        Ok(())
    } else {
        Err("CEX orderbook identity does not match the configured spot symbol")
    }
}
