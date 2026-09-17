use crate::adapter::strip_common_suffixes;
use serde::Deserialize;
use shared_types::FundingRateData;

const EXCHANGE: &str = "okx";
const OKX_FUNDING_INTERVALS: [u32; 5] = [1, 2, 4, 6, 8];

#[derive(Debug, Clone, Deserialize)]
pub(super) struct FundingRateItem {
    #[serde(rename = "instId")]
    pub(super) inst_id: String,
    #[serde(default, rename = "fundingRate")]
    pub(super) funding_rate: String,
    #[serde(default, rename = "nextFundingTime")]
    next_funding_time: String,
    #[serde(default, rename = "fundingTime")]
    funding_time: String,
    /// OKX V5 also returns the predicted next funding rate. Empty string means unavailable.
    #[serde(default, rename = "nextFundingRate")]
    next_funding_rate: String,
}

pub(super) fn parse_funding(item: &FundingRateItem, volume_24h: f64) -> Option<FundingRateData> {
    let rate = item
        .funding_rate
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())?;
    let next_t = item.next_funding_time.parse::<i64>().ok()?;
    let last_t = item.funding_time.parse::<i64>().ok()?;
    if last_t <= 0 || next_t <= last_t {
        return None;
    }
    let interval_hours = funding_interval_hours(last_t, next_t);
    Some(FundingRateData {
        symbol: strip_common_suffixes(&item.inst_id),
        exchange: EXCHANGE.into(),
        rate,
        rate_8h: rate * (8.0 / interval_hours as f64),
        predicted_rate: predicted_rate(&item.next_funding_rate),
        next_funding_time: next_t,
        funding_interval: interval_hours,
        volume_24h,
        timestamp: last_t,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

fn funding_interval_hours(last_t: i64, next_t: i64) -> u32 {
    if last_t <= 0 || next_t <= last_t {
        return 8;
    }
    let dt_ms = (next_t - last_t) as f64;
    let hours = (dt_ms / 3_600_000.0).round() as u32;
    snap_okx_interval(hours)
}

fn snap_okx_interval(hours: u32) -> u32 {
    OKX_FUNDING_INTERVALS
        .iter()
        .copied()
        .min_by_key(|value| value.abs_diff(hours))
        .unwrap_or(8)
}

fn predicted_rate(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|rate| rate.is_finite())
}

#[cfg(test)]
#[path = "okx_funding_tests.rs"]
mod tests;
