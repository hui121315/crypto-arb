//! Black-Scholes-Merton 期权定价模型 + Greeks 计算。
//!
//! 数值 PoC（PoC-001）已对照 Python `scipy.stats.norm` 的输出，偏差 < 1e-9。
//!
//! 公式参考：
//! - Hull, J. C. (2017). *Options, Futures, and Other Derivatives* (10th ed.). 第 15 章
//! - 简明回顾：<https://en.wikipedia.org/wiki/Black%E2%80%93Scholes_model>

use shared_types::{OptionGreeks, OptionType};
use statrs::distribution::{Continuous, ContinuousCDF, Normal};
use std::sync::OnceLock;

pub const CRYPTO_DAYS_PER_YEAR: f64 = 365.0;
pub const TRADFI_TRADING_DAYS_PER_YEAR: f64 = 252.0;

static STANDARD_NORMAL: OnceLock<Normal> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
pub struct GreeksParams {
    pub spot: f64,
    pub strike: f64,
    pub time_years: f64,
    pub risk_free: f64,
    pub volatility: f64,
    pub kind: OptionType,
}

#[derive(Debug, Clone, Copy)]
pub struct GreeksDaysParams {
    pub spot: f64,
    pub strike: f64,
    pub days: f64,
    pub risk_free: f64,
    pub volatility: f64,
    pub kind: OptionType,
}

/// d1 参数。
#[inline]
fn d1(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    if t <= 0.0 || sigma <= 0.0 {
        return 0.0;
    }
    ((s / k).ln() + (r + 0.5 * sigma * sigma) * t) / (sigma * t.sqrt())
}

/// d2 参数。
#[inline]
fn d2(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    if t <= 0.0 || sigma <= 0.0 {
        return 0.0;
    }
    d1(s, k, t, r, sigma) - sigma * t.sqrt()
}

fn standard_normal() -> &'static Normal {
    STANDARD_NORMAL.get_or_init(Normal::standard)
}

#[inline]
fn norm_cdf(x: f64) -> f64 {
    standard_normal().cdf(x)
}

#[inline]
fn norm_pdf(x: f64) -> f64 {
    standard_normal().pdf(x)
}

/// 看涨期权价格。
pub fn call_price(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    if t <= 0.0 {
        return (s - k).max(0.0);
    }
    let d1 = d1(s, k, t, r, sigma);
    let d2 = d2(s, k, t, r, sigma);
    s * norm_cdf(d1) - k * (-r * t).exp() * norm_cdf(d2)
}

/// 看跌期权价格。
pub fn put_price(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    if t <= 0.0 {
        return (k - s).max(0.0);
    }
    let d1 = d1(s, k, t, r, sigma);
    let d2 = d2(s, k, t, r, sigma);
    k * (-r * t).exp() * norm_cdf(-d2) - s * norm_cdf(-d1)
}

/// Delta：价格敏感度。Call 在 [0,1]，Put 在 [-1,0]。
pub fn delta(s: f64, k: f64, t: f64, r: f64, sigma: f64, kind: OptionType) -> f64 {
    if t <= 0.0 {
        return match kind {
            OptionType::Call => {
                if s > k {
                    1.0
                } else {
                    0.0
                }
            }
            OptionType::Put => {
                if s < k {
                    -1.0
                } else {
                    0.0
                }
            }
        };
    }
    let d1 = d1(s, k, t, r, sigma);
    match kind {
        OptionType::Call => norm_cdf(d1),
        OptionType::Put => norm_cdf(d1) - 1.0,
    }
}

/// Gamma：Delta 的变化率。Call/Put 相同。
pub fn gamma(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    if t <= 0.0 || sigma <= 0.0 {
        return 0.0;
    }
    let d1 = d1(s, k, t, r, sigma);
    norm_pdf(d1) / (s * sigma * t.sqrt())
}

/// Theta：每日时间衰减。返回值为**每日**变化（默认按 crypto 365 天）。
pub fn theta(s: f64, k: f64, t: f64, r: f64, sigma: f64, kind: OptionType) -> f64 {
    theta_with_days_per_year(
        GreeksParams {
            spot: s,
            strike: k,
            time_years: t,
            risk_free: r,
            volatility: sigma,
            kind,
        },
        CRYPTO_DAYS_PER_YEAR,
    )
}

/// Theta：按指定年化天数换算每日衰减。
pub fn theta_with_days_per_year(params: GreeksParams, days_per_year: f64) -> f64 {
    let s = params.spot;
    let k = params.strike;
    let t = params.time_years;
    let r = params.risk_free;
    let sigma = params.volatility;
    let kind = params.kind;
    if t <= 0.0 {
        return 0.0;
    }
    let d1 = d1(s, k, t, r, sigma);
    let d2 = d2(s, k, t, r, sigma);
    let term1 = -(s * norm_pdf(d1) * sigma) / (2.0 * t.sqrt());
    let annual = match kind {
        OptionType::Call => term1 - r * k * (-r * t).exp() * norm_cdf(d2),
        OptionType::Put => term1 + r * k * (-r * t).exp() * norm_cdf(-d2),
    };
    annual / days_per_year.max(1.0)
}

/// Vega：波动率敏感度。返回值为**波动率每变化 1%** 时的期权价格变动量。
/// Call/Put 相同。
pub fn vega(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    let d1 = d1(s, k, t, r, sigma);
    s * t.sqrt() * norm_pdf(d1) / 100.0
}

/// Rho：利率敏感度。返回值为**利率每变化 1%** 时的期权价格变动量。
pub fn rho(s: f64, k: f64, t: f64, r: f64, sigma: f64, kind: OptionType) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    let d2 = d2(s, k, t, r, sigma);
    match kind {
        OptionType::Call => k * t * (-r * t).exp() * norm_cdf(d2) / 100.0,
        OptionType::Put => -k * t * (-r * t).exp() * norm_cdf(-d2) / 100.0,
    }
}

/// 一次性计算所有 Greeks。
pub fn all_greeks(s: f64, k: f64, t: f64, r: f64, sigma: f64, kind: OptionType) -> OptionGreeks {
    all_greeks_with_days_per_year(
        GreeksParams {
            spot: s,
            strike: k,
            time_years: t,
            risk_free: r,
            volatility: sigma,
            kind,
        },
        CRYPTO_DAYS_PER_YEAR,
    )
}

/// 一次性计算所有 Greeks，并显式选择 theta 的年化天数。
pub fn all_greeks_with_days_per_year(params: GreeksParams, days_per_year: f64) -> OptionGreeks {
    let s = params.spot;
    let k = params.strike;
    let t = params.time_years;
    let r = params.risk_free;
    let sigma = params.volatility;
    OptionGreeks {
        delta: delta(s, k, t, r, sigma, params.kind),
        gamma: gamma(s, k, t, r, sigma),
        theta: theta_with_days_per_year(params, days_per_year),
        vega: vega(s, k, t, r, sigma),
        rho: rho(s, k, t, r, sigma, params.kind),
        iv: sigma,
    }
}

/// 便捷函数：从 `days_to_expiry（天`）→ T（年），调用 `all_greeks`。
pub fn greeks_from_days(
    spot: f64,
    strike: f64,
    days: f64,
    risk_free: f64,
    volatility: f64,
    kind: OptionType,
) -> OptionGreeks {
    let t = days / 365.0;
    all_greeks(spot, strike, t, risk_free, volatility, kind)
}

/// 便捷函数：`days_to_expiry` 与 theta 年化天数均可配置。
pub fn greeks_from_days_with_days_per_year(
    params: GreeksDaysParams,
    days_per_year: f64,
) -> OptionGreeks {
    let days_per_year = days_per_year.max(1.0);
    all_greeks_with_days_per_year(
        GreeksParams {
            spot: params.spot,
            strike: params.strike,
            time_years: params.days / days_per_year,
            risk_free: params.risk_free,
            volatility: params.volatility,
            kind: params.kind,
        },
        days_per_year,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// d1/d2 在 T=0 时返回 0（避免数值爆炸）。
    #[test]
    fn d1_zero_when_t_is_zero() {
        assert_eq!(d1(100.0, 100.0, 0.0, 0.05, 0.5), 0.0);
        assert_eq!(d2(100.0, 100.0, 0.0, 0.05, 0.5), 0.0);
    }

    /// T=0 时 call price = max(S - K, 0)。
    #[test]
    fn call_price_at_expiry_intrinsic() {
        assert!((call_price(100.0, 90.0, 0.0, 0.05, 0.5) - 10.0).abs() < 1e-12);
        assert!((call_price(100.0, 110.0, 0.0, 0.05, 0.5) - 0.0).abs() < 1e-12);
    }

    /// T=0 时 put price = max(K - S, 0)。
    #[test]
    fn put_price_at_expiry_intrinsic() {
        assert!((put_price(100.0, 110.0, 0.0, 0.05, 0.5) - 10.0).abs() < 1e-12);
        assert!((put_price(100.0, 90.0, 0.0, 0.05, 0.5) - 0.0).abs() < 1e-12);
    }

    /// Put-Call Parity：C - P = S - K * exp(-rT)
    #[test]
    fn put_call_parity_holds() {
        let s = 100.0;
        let k = 100.0;
        let t = 0.5;
        let r = 0.05;
        let sigma = 0.3;
        let c = call_price(s, k, t, r, sigma);
        let p = put_price(s, k, t, r, sigma);
        let lhs = c - p;
        let rhs = s - k * (-r * t).exp();
        assert!((lhs - rhs).abs() < 1e-9, "parity broken: {lhs} vs {rhs}");
    }

    /// ATM call 的 Delta ≈ 0.5+ε（forward-style）。
    #[test]
    fn atm_call_delta_near_half() {
        let d = delta(100.0, 100.0, 0.5, 0.05, 0.3, OptionType::Call);
        assert!(d > 0.5 && d < 0.7);
    }

    /// 看涨 Delta + |看跌 Delta| = 1（无分红）。
    #[test]
    fn delta_complementary() {
        let s = 100.0;
        let k = 100.0;
        let t = 0.25;
        let r = 0.03;
        let sigma = 0.4;
        let dc = delta(s, k, t, r, sigma, OptionType::Call);
        let dp = delta(s, k, t, r, sigma, OptionType::Put);
        assert!((dc - dp - 1.0).abs() < 1e-9);
    }

    /// Gamma 总为正。
    #[test]
    fn gamma_is_positive() {
        let g = gamma(100.0, 100.0, 0.5, 0.05, 0.3);
        assert!(g > 0.0);
    }

    /// Vega 总为正。
    #[test]
    fn vega_is_positive() {
        let v = vega(100.0, 100.0, 0.5, 0.05, 0.3);
        assert!(v > 0.0);
    }

    /// Theta 在 ATM 通常为负（时间价值衰减）。
    #[test]
    fn theta_atm_call_is_negative() {
        let th = theta(100.0, 100.0, 0.5, 0.05, 0.3, OptionType::Call);
        assert!(th < 0.0);
    }

    #[test]
    fn theta_supports_tradfi_trading_days() {
        let crypto = theta(100.0, 100.0, 0.5, 0.05, 0.3, OptionType::Call);
        let tradfi = theta_with_days_per_year(
            GreeksParams {
                spot: 100.0,
                strike: 100.0,
                time_years: 0.5,
                risk_free: 0.05,
                volatility: 0.3,
                kind: OptionType::Call,
            },
            TRADFI_TRADING_DAYS_PER_YEAR,
        );
        assert!(tradfi < crypto);
    }

    #[test]
    fn all_greeks_carries_iv() {
        let g = all_greeks(100.0, 100.0, 0.5, 0.05, 0.4, OptionType::Call);
        assert!((g.iv - 0.4).abs() < 1e-12);
    }
}
