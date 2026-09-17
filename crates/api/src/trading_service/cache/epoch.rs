use super::*;

impl TradingService {
    pub(in crate::trading_service) fn set_adapter_name(&self, adapter_name: &'static str) {
        *self.adapter_name.write() = adapter_name;
        self.invalidate_account_credentials();
    }

    pub(crate) fn account_cache_epoch(&self) -> u64 {
        self.account_cache_epoch.load(Ordering::Relaxed)
    }
}

pub(super) fn uses_event_driven_position_cache(venue: &str) -> bool {
    let venue = normalized_venue_name(venue);
    matches!(
        venue.as_str(),
        "binance" | "bitget" | "bybit" | "gate" | "gate_crossex" | "kraken" | "kucoin" | "okx"
    ) || venue == "hyperliquid"
        || venue.starts_with("hyperliquid:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_private_position_stream_can_extend_a_seeded_cache() {
        for venue in [
            "binance",
            "bitget",
            "bybit",
            "gate",
            "gate_crossex",
            "kraken",
            "kucoin",
            "okx",
            "hyperliquid",
            "hyperliquid:xyz",
        ] {
            assert!(uses_event_driven_position_cache(venue), "{venue}");
        }
        assert!(!uses_event_driven_position_cache("mock"));
    }
}
