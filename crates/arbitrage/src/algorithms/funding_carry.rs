//! 单 venue 资金费 carry 观察入口。

use crate::models::RawOpportunity;
use shared_types::FundingRateData;

#[derive(Debug, Clone, Copy)]
pub struct FundingCarryConfig {
    pub min_rate_bps: f64,
    pub min_volume_24h: f64,
}

impl Default for FundingCarryConfig {
    fn default() -> Self {
        Self {
            min_rate_bps: 1.0,
            min_volume_24h: 100_000.0,
        }
    }
}

pub fn scan(
    _rates: &[FundingRateData],
    _venue: Option<&str>,
    _config: FundingCarryConfig,
) -> Vec<RawOpportunity> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_is_observation_only_in_p0() {
        let rows = scan(
            &[rate("binance", 0.0005)],
            None,
            FundingCarryConfig::default(),
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn one_hour_funding_does_not_create_executable_row() {
        let mut r = rate("hyperliquid", 0.0008);
        r.funding_interval = 1;
        let rows = scan(&[r], None, FundingCarryConfig::default());
        assert!(rows.is_empty());
    }

    #[test]
    fn venue_filter_does_not_reenable_executable_rows() {
        let rows = scan(
            &[rate("okx", -0.0005), rate("binance", 0.0005)],
            Some("OKX"),
            FundingCarryConfig::default(),
        );
        assert!(rows.is_empty());
    }

    fn rate(exchange: &str, rate_8h: f64) -> FundingRateData {
        FundingRateData {
            symbol: "BTC".into(),
            exchange: exchange.into(),
            rate: rate_8h,
            rate_8h,
            predicted_rate: None,
            next_funding_time: 1,
            funding_interval: 8,
            volume_24h: 1_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }
}
