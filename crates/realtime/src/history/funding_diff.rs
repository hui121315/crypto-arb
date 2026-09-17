use super::types::FundingDiffRow;
use shared_types::{venue_names_equal, FundingRateData};
use std::collections::BTreeMap;

pub(super) fn derive_rows(rows: &[FundingRateData], occurred_at_ms: i64) -> Vec<FundingDiffRow> {
    let mut by_symbol: BTreeMap<&str, Vec<&FundingRateData>> = BTreeMap::new();
    for row in rows.iter().filter(|row| valid_rate(row)) {
        by_symbol.entry(&row.symbol).or_default().push(row);
    }
    let mut out = Vec::new();
    for (symbol, mut entries) in by_symbol {
        entries.sort_by(|a, b| {
            a.rate_8h
                .partial_cmp(&b.rate_8h)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        append_symbol_diffs(symbol, &entries, occurred_at_ms, &mut out);
    }
    out
}

fn append_symbol_diffs(
    symbol: &str,
    entries: &[&FundingRateData],
    occurred_at_ms: i64,
    out: &mut Vec<FundingDiffRow>,
) {
    for (low_idx, low) in entries.iter().enumerate() {
        for high in entries.iter().skip(low_idx + 1).rev() {
            if venue_names_equal(&low.exchange, &high.exchange) {
                continue;
            }
            let diff_bps = (high.rate_8h - low.rate_8h) * 10_000.0;
            if diff_bps <= f64::EPSILON {
                continue;
            }
            out.push(row(symbol, low, high, diff_bps, occurred_at_ms));
        }
    }
}

fn row(
    symbol: &str,
    low: &FundingRateData,
    high: &FundingRateData,
    gross_diff_bps: f64,
    occurred_at_ms: i64,
) -> FundingDiffRow {
    FundingDiffRow {
        occurred_at_ms,
        symbol: symbol.to_owned(),
        long_exchange: low.exchange.clone(),
        short_exchange: high.exchange.clone(),
        long_rate_8h: low.rate_8h,
        short_rate_8h: high.rate_8h,
        gross_diff_bps,
        long_next_funding_ms: low.next_funding_time,
        short_next_funding_ms: high.next_funding_time,
        window_alignment_minutes: alignment_minutes(low.next_funding_time, high.next_funding_time),
        long_interval_hours: low.funding_interval,
        short_interval_hours: high.funding_interval,
        min_volume_24h: low.volume_24h.min(high.volume_24h),
    }
}

fn valid_rate(row: &FundingRateData) -> bool {
    row.rate_8h.is_finite()
        && row.volume_24h.is_finite()
        && row.funding_interval > 0
        && row.next_funding_time > 0
        && !row.symbol.is_empty()
        && !row.exchange.is_empty()
}

fn alignment_minutes(left_ms: i64, right_ms: i64) -> i32 {
    if left_ms <= 0 || right_ms <= 0 {
        return 0;
    }
    ((left_ms - right_ms) / 60_000).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_low_rate_long_high_rate_short_pairs() {
        let rows = vec![
            funding("BTC", "binance", 0.0001),
            funding("BTC", "okx", 0.0005),
            funding("ETH", "gate", 0.0002),
        ];

        let diffs = derive_rows(&rows, 42);

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].symbol, "BTC");
        assert_eq!(diffs[0].long_exchange, "binance");
        assert_eq!(diffs[0].short_exchange, "okx");
        assert_eq!(diffs[0].gross_diff_bps, 4.0);
    }

    #[test]
    fn skips_unknown_funding_alignment() {
        let mut unknown = funding("BTC", "binance", 0.0001);
        unknown.next_funding_time = 0;
        let rows = vec![unknown, funding("BTC", "okx", 0.0005)];

        let diffs = derive_rows(&rows, 42);

        assert!(diffs.is_empty());
    }

    fn funding(symbol: &str, exchange: &str, rate_8h: f64) -> FundingRateData {
        FundingRateData {
            symbol: symbol.into(),
            exchange: exchange.into(),
            rate: rate_8h,
            rate_8h,
            predicted_rate: None,
            next_funding_time: 100_000,
            funding_interval: 8,
            volume_24h: 1_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }
}
