use shared_types::normalized_venue_name;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct MarketKey {
    venue: String,
    symbol: String,
}

impl MarketKey {
    pub(crate) fn new(venue: &str, symbol: &str) -> Self {
        let venue = normalized_venue_name(venue);
        Self {
            symbol: normalized_market_symbol(&venue, symbol),
            venue,
        }
    }

    pub(crate) fn spot(venue: &str, symbol: &str) -> Self {
        Self {
            venue: normalized_venue_name(venue),
            symbol: normalized_spot_market_symbol(symbol),
        }
    }

    pub(super) fn belongs_to_venue(&self, venue: &str) -> bool {
        shared_types::venue_names_equal(&self.venue, venue)
    }
}

fn normalized_spot_market_symbol(symbol: &str) -> String {
    symbol
        .trim()
        .to_ascii_uppercase()
        .replace(['-', '_', ':'], "/")
        .replace(' ', "")
}

fn normalized_market_symbol(venue: &str, symbol: &str) -> String {
    let symbol = exchange::strip_common_suffixes(symbol.trim());
    let Some(venue_namespace) = venue.strip_prefix("hyperliquid:") else {
        return symbol;
    };
    let Some((symbol_namespace, base)) = symbol.split_once(':') else {
        return symbol;
    };
    if !symbol_namespace.eq_ignore_ascii_case(venue_namespace) {
        return symbol;
    }
    exchange::strip_common_suffixes(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_key_normalizes_venue_and_symbol() {
        assert_eq!(
            MarketKey::new(" Hyperliquid:XYZ ", " mu "),
            MarketKey {
                venue: "hyperliquid:xyz".to_owned(),
                symbol: "MU".to_owned()
            }
        );
        assert_eq!(
            MarketKey::new("Binance", "MUUSDT"),
            MarketKey::new("binance", "MU")
        );
    }

    #[test]
    fn hyperliquid_builder_native_and_canonical_symbols_share_one_cache_key() {
        assert_eq!(
            MarketKey::new("hyperliquid:xyz", "xyz:ZHIPU"),
            MarketKey::new("hyperliquid:xyz", "ZHIPU")
        );
        assert_ne!(
            MarketKey::new("hyperliquid:xyz", "km:ZHIPU"),
            MarketKey::new("hyperliquid:xyz", "ZHIPU")
        );
    }

    #[test]
    fn spot_key_preserves_quote_identity() {
        assert_ne!(
            MarketKey::spot("kraken", "SOL/USD"),
            MarketKey::spot("kraken", "SOL/USDC")
        );
        assert_eq!(
            MarketKey::spot("okx", "SOL-USDC"),
            MarketKey::spot("OKX", "SOL/USDC")
        );
    }
}
