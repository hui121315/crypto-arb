//! Bitget shared market helpers (V2/V3 common surface).
//!
//! Now that PR-DP-13 · B-7 removed the V2-only DTOs and parsers, this module
//! only holds the constants and helpers that the V3 / UTA modules still
//! depend on: `parse_levels`, `valid_bitget_funding_interval`,
//! `bitget_depth_limit`, and `spot_symbol_matches`. Keeping them under the
//! historical filename avoids a no-op rename churn across the wider crate.

pub(super) fn parse_levels(levels: &[[f64; 2]]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .copied()
        .filter(|[price, size]| {
            price.is_finite() && size.is_finite() && *price > 0.0 && *size > 0.0
        })
        .collect()
}

const BITGET_FUNDING_INTERVALS: [u32; 4] = [1, 2, 4, 8];

pub(super) fn valid_bitget_funding_interval(hours: Option<u32>) -> Option<u32> {
    let hours = hours?;
    BITGET_FUNDING_INTERVALS.contains(&hours).then_some(hours)
}

pub(super) fn bitget_depth_limit(depth: u32) -> u32 {
    depth.clamp(1, 1_000)
}

pub(super) fn spot_symbol_matches(symbol: &str, symbols: Option<&[String]>) -> bool {
    let Some((base, quote)) = crate::spot::suffix_pair(symbol) else {
        return false;
    };
    crate::spot::symbol_matches(symbol, &base, &quote, symbols)
}

#[cfg(test)]
#[path = "bitget_market_data_tests.rs"]
mod tests;
