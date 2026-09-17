//! 同 venue 三角套利观察入口。

use crate::models::RawOpportunity;
use rust_decimal::Decimal;
use shared_types::SpotTick;

#[derive(Debug, Clone, Copy)]
pub struct TriangularConfig {
    pub min_edge_bps: f64,
    pub min_volume_24h: f64,
}

impl Default for TriangularConfig {
    fn default() -> Self {
        Self {
            min_edge_bps: 3.0,
            min_volume_24h: 50_000.0,
        }
    }
}

pub fn scan(
    ticks: &[SpotTick],
    venue: Option<&str>,
    _config: TriangularConfig,
) -> Vec<RawOpportunity> {
    // P0 仅做观察：先按 fail-closed 规则筛出“两侧都有可执行报量”的腿，
    // 缺失 size（None）或 0 报量的 tick 一律剔除，绝不按 0 继续撮合三角腿。
    let _executable: Vec<&SpotTick> = ticks
        .iter()
        .filter(|tick| venue.is_none_or(|name| tick.venue == name))
        .filter(|tick| executable_leg_size(tick).is_some())
        .collect();
    Vec::new()
}

/// 三角腿可成交规模：两侧报量都必须真实存在且严格为正才返回 `Some`。
///
/// `None`（官方载荷未给出 size）与真实的 0 报量在执行语义上都不可成交，
/// 因此都回 `None`（fail-closed），绝不退化为按 0 数量下单。
fn executable_leg_size(tick: &SpotTick) -> Option<Decimal> {
    let bid = tick.bid_size?;
    let ask = tick.ask_size?;
    (bid > Decimal::ZERO && ask > Decimal::ZERO).then(|| bid.min(ask))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    #[test]
    fn scan_is_observation_only_in_p0() {
        let rows = scan(
            &[
                tick("binance", "BTC/USDT", dec!(100), dec!(100)),
                tick("binance", "ETH/BTC", dec!(2), dec!(2)),
                tick("binance", "ETH/USDT", dec!(205), dec!(206)),
            ],
            Some("binance"),
            TriangularConfig::default(),
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn venue_filter_does_not_reenable_executable_rows() {
        let rows = scan(
            &[
                tick("binance", "BTC/USDT", dec!(100), dec!(100)),
                tick("binance", "ETH/BTC", dec!(2), dec!(2)),
                tick("binance", "ETH/USDT", dec!(200.4), dec!(200.5)),
            ],
            Some("binance"),
            TriangularConfig::default(),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn executable_leg_size_requires_both_sides_present_and_positive() {
        let mut sized = tick("binance", "BTC/USDT", dec!(100), dec!(100));
        sized.bid_size = Some(dec!(3));
        sized.ask_size = Some(dec!(5));
        assert_eq!(executable_leg_size(&sized), Some(dec!(3)));
    }

    #[test]
    fn executable_leg_size_is_none_when_size_missing_or_zero() {
        let mut missing = tick("binance", "BTC/USDT", dec!(100), dec!(100));
        missing.bid_size = None;
        missing.ask_size = Some(dec!(5));
        assert_eq!(executable_leg_size(&missing), None);

        let mut zero = tick("binance", "BTC/USDT", dec!(100), dec!(100));
        zero.bid_size = Some(Decimal::ZERO);
        zero.ask_size = Some(dec!(5));
        assert_eq!(executable_leg_size(&zero), None);
    }

    fn tick(venue: &str, symbol: &str, bid: Decimal, ask: Decimal) -> SpotTick {
        SpotTick {
            venue: venue.into(),
            symbol: symbol.into(),
            bid,
            ask,
            last: ask,
            bid_size: Some(dec!(10)),
            ask_size: Some(dec!(10)),
            volume_24h: dec!(1000000),
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        }
    }
}
