//! 交易成本模型。
//!
//! 按策略的真实执行周期计算交易成本。
//!
//! 默认周期包含双腿开仓和平仓；现货跨所价差在首轮买卖时已经实现，后续搬砖成本
//! 由 transfer loop 单独核验，不能再虚构一组平仓交易。

use crate::models::CostBreakdown;
use shared_types::{StrategyExecutionCycle, VenueId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductType {
    Spot,
    Perp,
    Option,
}

#[derive(Debug, Clone, Copy)]
pub struct FeeProfile {
    pub vip_level: u8,
    pub uses_token_discount: bool,
    pub product_type: ProductType,
}

#[derive(Debug, Clone, Copy)]
pub struct LegCostContext<'a> {
    pub exchange: &'a str,
    pub product_type: ProductType,
}

impl Default for FeeProfile {
    fn default() -> Self {
        Self {
            vip_level: 0,
            uses_token_discount: false,
            product_type: ProductType::Perp,
        }
    }
}

/// 各交易所的 taker / maker 费率（占比，例如 0.0005 表示 0.05%）。
///
/// 这些是按公开费率页固化的默认档位；用户侧按标准档位评估，不读取账户/VIP 费率。
pub fn taker_fee(exchange: &str) -> f64 {
    taker_fee_with_profile(exchange, FeeProfile::default())
}

pub fn maker_fee(exchange: &str) -> f64 {
    maker_fee_with_profile(exchange, FeeProfile::default())
}

pub fn taker_fee_for_product(exchange: &str, product_type: ProductType) -> f64 {
    taker_fee_with_profile(
        exchange,
        FeeProfile {
            product_type,
            ..FeeProfile::default()
        },
    )
}

pub fn taker_fee_with_profile(exchange: &str, profile: FeeProfile) -> f64 {
    let base = base_taker_fee(exchange, profile.product_type);
    apply_fee_profile(exchange, base, profile)
}

pub fn maker_fee_with_profile(exchange: &str, profile: FeeProfile) -> f64 {
    let base = base_maker_fee(exchange, profile.product_type);
    apply_fee_profile(exchange, base, profile)
}

fn base_taker_fee(exchange: &str, product_type: ProductType) -> f64 {
    match venue_id(exchange) {
        Some(VenueId::Binance) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp => 0.000_4,
            ProductType::Option => 0.000_3,
        },
        Some(VenueId::Okx) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp => 0.000_5,
            ProductType::Option => 0.000_3,
        },
        Some(VenueId::Bybit) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp => 0.000_55,
            ProductType::Option => 0.000_3,
        },
        Some(VenueId::Bitget) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_6,
        },
        Some(VenueId::Gate) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_5,
        },
        Some(VenueId::Htx) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_5,
        },
        Some(VenueId::Kucoin) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_6,
        },
        Some(VenueId::Hyperliquid) => match product_type {
            ProductType::Spot => 0.000_7,
            ProductType::Perp | ProductType::Option => 0.000_45,
        },
        Some(VenueId::Kraken) => match product_type {
            ProductType::Spot => 0.004,
            ProductType::Perp | ProductType::Option => 0.000_5,
        },
        // CrossEx exposes the account's actual per-underlying fees through the
        // authenticated `/crossex/fee` endpoint. Keep this legacy estimate
        // conservative until that evidence replaces it in the execution path.
        Some(VenueId::GateCrossEx) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_6,
        },
        None => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_5,
        },
    }
}

fn base_maker_fee(exchange: &str, product_type: ProductType) -> f64 {
    match venue_id(exchange) {
        Some(VenueId::Binance) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp => 0.000_2,
            ProductType::Option => 0.000_1,
        },
        Some(VenueId::Okx) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp => 0.000_2,
            ProductType::Option => 0.000_2,
        },
        Some(VenueId::Bybit) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp => 0.000_2,
            ProductType::Option => 0.000_2,
        },
        Some(VenueId::Bitget) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_2,
        },
        Some(VenueId::Gate) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_15,
        },
        Some(VenueId::Htx) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_2,
        },
        Some(VenueId::Kucoin) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_2,
        },
        Some(VenueId::Hyperliquid) => match product_type {
            ProductType::Spot => 0.000_4,
            ProductType::Perp | ProductType::Option => 0.000_15,
        },
        Some(VenueId::Kraken) => match product_type {
            ProductType::Spot => 0.002_5,
            ProductType::Perp | ProductType::Option => 0.000_2,
        },
        Some(VenueId::GateCrossEx) => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_2,
        },
        None => match product_type {
            ProductType::Spot => 0.001,
            ProductType::Perp | ProductType::Option => 0.000_2,
        },
    }
}

fn apply_fee_profile(exchange: &str, base: f64, profile: FeeProfile) -> f64 {
    let vip_factor = vip_factor(profile.vip_level);
    let token_factor = token_discount_factor(exchange, profile.uses_token_discount);
    (base * vip_factor * token_factor).max(-0.000_2)
}

fn venue_id(exchange: &str) -> Option<VenueId> {
    VenueId::from_exchange_name(exchange)
}

fn vip_factor(vip_level: u8) -> f64 {
    match vip_level.min(9) {
        0 => 1.0,
        1 => 0.85,
        2 => 0.75,
        3 => 0.65,
        4 => 0.55,
        5 => 0.45,
        6 => 0.38,
        7 => 0.32,
        8 => 0.28,
        _ => 0.25,
    }
}

fn token_discount_factor(exchange: &str, enabled: bool) -> f64 {
    if !enabled {
        return 1.0;
    }
    match venue_id(exchange) {
        Some(VenueId::Binance) => 0.9,
        Some(VenueId::Okx | VenueId::Bitget | VenueId::Kucoin) => 0.8,
        _ => 1.0,
    }
}

/// 计算单边手续费（按 maker / taker 比例混合）。`taker_ratio` 默认 0.5。
pub fn average_fee(exchange: &str, taker_ratio: f64) -> f64 {
    average_fee_for_product(exchange, ProductType::Perp, taker_ratio)
}

pub fn average_fee_for_product(exchange: &str, product_type: ProductType, taker_ratio: f64) -> f64 {
    let r = taker_ratio.clamp(0.0, 1.0);
    taker_fee_for_product(exchange, product_type) * r
        + maker_fee_with_profile(
            exchange,
            FeeProfile {
                product_type,
                ..FeeProfile::default()
            },
        ) * (1.0 - r)
}

/// 计算套利完整开/平仓的总成本率。
///
/// - `long_exchange` / `short_exchange`：两侧交易所
/// - `slippage`：单边滑点率（如 0.000 2 = 0.02%）
/// - `taker_ratio`：成交中作为 taker 的比例
///
/// 总成本 = 2 笔开仓 + 2 笔平仓 + 4 笔单边滑点
pub fn round_trip_cost(
    long_exchange: &str,
    short_exchange: &str,
    slippage: f64,
    taker_ratio: f64,
) -> CostBreakdown {
    round_trip_cost_for_legs(
        LegCostContext {
            exchange: long_exchange,
            product_type: ProductType::Perp,
        },
        LegCostContext {
            exchange: short_exchange,
            product_type: ProductType::Perp,
        },
        slippage,
        taker_ratio,
    )
}

pub fn round_trip_cost_for_legs(
    long: LegCostContext<'_>,
    short: LegCostContext<'_>,
    slippage: f64,
    taker_ratio: f64,
) -> CostBreakdown {
    execution_cycle_cost_for_legs(
        long,
        short,
        slippage,
        taker_ratio,
        StrategyExecutionCycle::PairedOpenClose,
    )
}

pub fn execution_cycle_cost_for_legs(
    long: LegCostContext<'_>,
    short: LegCostContext<'_>,
    slippage: f64,
    taker_ratio: f64,
    cycle: StrategyExecutionCycle,
) -> CostBreakdown {
    let fee_long = average_fee_for_product(long.exchange, long.product_type, taker_ratio);
    let fee_short = average_fee_for_product(short.exchange, short.product_type, taker_ratio);
    let paired_fee = fee_long + fee_short;
    let (fee_total, slippage_total) = match cycle {
        StrategyExecutionCycle::PairedOpenClose => (paired_fee * 2.0, slippage * 4.0),
        StrategyExecutionCycle::PairedOpenRebalance => (paired_fee, slippage * 2.0),
    };
    let total = fee_total + slippage_total;
    CostBreakdown {
        fee_rate: (fee_long + fee_short) / 2.0,
        slippage,
        round_trip_cost: total,
    }
}

/// 净单次收益 = 单次毛收益 - 单次摊销的成本。
///
/// `min_holding_periods` 决定成本的摊销系数：成本是 `round_trip` 一次性的，分摊到 N 次结算。
pub fn net_single_yield(gross_yield: f64, cost: &CostBreakdown, min_holding_periods: u32) -> f64 {
    let n = (min_holding_periods.max(1)) as f64;
    gross_yield - cost.round_trip_cost / n
}

/// 给定毛收益与成本，反推必须持有的最少结算周期数（让净收益为正）。
pub fn min_holding_periods(gross_yield: f64, cost: &CostBreakdown) -> u32 {
    if gross_yield <= 0.0 {
        return u32::MAX;
    }
    let n = (cost.round_trip_cost / gross_yield).ceil() as i64;
    n.max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn taker_fee_known_exchanges() {
        assert!((taker_fee("binance") - 0.000_4).abs() < 1e-12);
        assert!((taker_fee("kucoin") - 0.000_6).abs() < 1e-12);
        assert!((taker_fee("unknown") - 0.000_5).abs() < 1e-12); // fallback
    }

    #[test]
    fn builder_dex_uses_parent_exchange_fee() {
        assert_eq!(taker_fee("hyperliquid:xyz"), taker_fee("hyperliquid"));
        assert_eq!(maker_fee("hyperliquid:km"), maker_fee("hyperliquid"));
        assert_eq!(taker_fee(" Hyperliquid:XYZ "), taker_fee("hyperliquid"));
    }

    #[test]
    fn kraken_uses_official_retail_spot_and_derivatives_tiers() {
        assert_eq!(taker_fee_for_product("kraken", ProductType::Spot), 0.004);
        assert_eq!(
            maker_fee_with_profile(
                "kraken",
                FeeProfile {
                    product_type: ProductType::Perp,
                    ..FeeProfile::default()
                }
            ),
            0.000_2
        );
    }

    #[test]
    fn average_fee_taker_only_equals_taker() {
        let f = average_fee("binance", 1.0);
        assert!((f - taker_fee("binance")).abs() < 1e-12);
    }

    #[test]
    fn average_fee_maker_only_equals_maker() {
        let f = average_fee("binance", 0.0);
        assert!((f - maker_fee("binance")).abs() < 1e-12);
    }

    #[test]
    fn round_trip_total() {
        let cost = round_trip_cost("binance", "okx", 0.000_2, 0.5);
        // (0.0003 + 0.00035) * 2 + 0.0002 * 4 = 0.001_3 + 0.000_8 = 0.002_1
        assert!((cost.round_trip_cost - 0.002_1).abs() < 1e-9);
    }

    #[test]
    fn round_trip_for_legs_uses_leg_product_type() {
        let perp_only = round_trip_cost("binance", "binance", 0.0, 0.5);
        let spot_perp = round_trip_cost_for_legs(
            LegCostContext {
                exchange: "binance",
                product_type: ProductType::Spot,
            },
            LegCostContext {
                exchange: "binance",
                product_type: ProductType::Perp,
            },
            0.0,
            0.5,
        );

        assert!((perp_only.round_trip_cost - 0.001_2).abs() < 1e-9);
        assert!((spot_perp.round_trip_cost - 0.002_6).abs() < 1e-9);
        assert!(spot_perp.round_trip_cost > perp_only.round_trip_cost);
    }

    #[test]
    fn spot_cross_cycle_charges_two_trades_before_transfer_rebalance() {
        let cycle = execution_cycle_cost_for_legs(
            LegCostContext {
                exchange: "binance",
                product_type: ProductType::Spot,
            },
            LegCostContext {
                exchange: "okx",
                product_type: ProductType::Spot,
            },
            0.000_2,
            1.0,
            StrategyExecutionCycle::PairedOpenRebalance,
        );

        // One buy + one sell, followed by transfer rebalance rather than two synthetic closes.
        assert!((cycle.round_trip_cost - 0.002_4).abs() < 1e-9);
    }

    #[test]
    fn fee_profile_applies_product_vip_and_token_discount() {
        let retail_spot = taker_fee_for_product("binance", ProductType::Spot);
        let discounted = taker_fee_with_profile(
            "binance",
            FeeProfile {
                vip_level: 3,
                uses_token_discount: true,
                product_type: ProductType::Spot,
            },
        );
        assert!(retail_spot > taker_fee("binance"));
        assert!(discounted < retail_spot);
    }

    #[test]
    fn net_yield_amortizes_cost() {
        let cost = CostBreakdown {
            fee_rate: 0.0,
            slippage: 0.0,
            round_trip_cost: 0.001,
        };
        // 毛 0.0005，成本 0.001 摊销到 4 次：0.0005 - 0.00025 = 0.00025
        let n = net_single_yield(0.000_5, &cost, 4);
        assert!((n - 0.000_25).abs() < 1e-9);
    }

    #[test]
    fn min_holding_periods_for_breakeven() {
        let cost = CostBreakdown {
            fee_rate: 0.0,
            slippage: 0.0,
            round_trip_cost: 0.001,
        };
        // 0.001 / 0.000_3 = 3.33 → 4 个周期
        assert_eq!(min_holding_periods(0.000_3, &cost), 4);
    }

    #[test]
    fn min_holding_periods_zero_yield_returns_max() {
        let cost = CostBreakdown {
            fee_rate: 0.0,
            slippage: 0.0,
            round_trip_cost: 0.001,
        };
        assert_eq!(min_holding_periods(0.0, &cost), u32::MAX);
    }
}
