//! 隐含波动率（IV）求解器。
//!
//! - **Newton-Raphson**：起步快但对初值敏感；适合 ATM
//! - **Brent**：bisection + secant + inverse quadratic 混合法；稳健但慢
//!
//! 默认 [`solve_iv`] 先尝试牛顿，失败时回退到 Brent。

use crate::black_scholes::{call_price, put_price, vega};
use shared_types::OptionType;
use std::f64::consts::TAU;

/// IV 求解默认参数。
pub const DEFAULT_INITIAL_GUESS: f64 = 0.5;
pub const DEFAULT_MAX_ITERATIONS: u32 = 100;
pub const DEFAULT_TOLERANCE: f64 = 1e-8;
pub const SIGMA_LOWER_BOUND: f64 = 0.001;
pub const SIGMA_UPPER_BOUND: f64 = 5.0;

#[derive(Debug, thiserror::Error)]
pub enum IvError {
    #[error("market price must be positive (got {0})")]
    NonPositivePrice(f64),
    #[error("expired option (T <= 0)")]
    Expired,
    #[error("newton-raphson failed to converge after {0} iterations")]
    NewtonNotConverged(u32),
    #[error("brent failed: bracket does not span zero (f_low={f_low}, f_high={f_high})")]
    BrentNoBracket { f_low: f64, f_high: f64 },
    #[error("brent failed to converge")]
    BrentNotConverged,
}

fn theoretical_price(s: f64, k: f64, t: f64, r: f64, sigma: f64, kind: OptionType) -> f64 {
    match kind {
        OptionType::Call => call_price(s, k, t, r, sigma),
        OptionType::Put => put_price(s, k, t, r, sigma),
    }
}

/// 牛顿-拉弗森法求 IV。
pub fn solve_iv_newton(
    market_price: f64,
    s: f64,
    k: f64,
    t: f64,
    r: f64,
    kind: OptionType,
) -> Result<f64, IvError> {
    if market_price <= 0.0 {
        return Err(IvError::NonPositivePrice(market_price));
    }
    if t <= 0.0 {
        return Err(IvError::Expired);
    }

    let mut sigma = initial_guess(market_price, s, t);
    for _ in 0..DEFAULT_MAX_ITERATIONS {
        let price = theoretical_price(s, k, t, r, sigma, kind);
        let diff = price - market_price;
        if diff.abs() < DEFAULT_TOLERANCE {
            return Ok(sigma.clamp(SIGMA_LOWER_BOUND, SIGMA_UPPER_BOUND));
        }
        // vega 已乘以 1/100，反推时需 ×100
        let v = vega(s, k, t, r, sigma) * 100.0;
        if v.abs() < 1e-12 {
            break;
        }
        sigma -= diff / v;
        sigma = sigma.clamp(SIGMA_LOWER_BOUND, SIGMA_UPPER_BOUND);
    }
    Err(IvError::NewtonNotConverged(DEFAULT_MAX_ITERATIONS))
}

fn initial_guess(market_price: f64, spot: f64, t: f64) -> f64 {
    if spot <= 0.0 || t <= 0.0 {
        return DEFAULT_INITIAL_GUESS;
    }
    let normalized_price = (market_price / spot).max(f64::EPSILON);
    (TAU / t)
        .sqrt()
        .mul_add(normalized_price, 0.0)
        .clamp(SIGMA_LOWER_BOUND, SIGMA_UPPER_BOUND)
}

/// Brent 法求根 `f(sigma) = bs_price(sigma) - market_price = 0`。
pub fn solve_iv_brent(
    market_price: f64,
    s: f64,
    k: f64,
    t: f64,
    r: f64,
    kind: OptionType,
) -> Result<f64, IvError> {
    if market_price <= 0.0 {
        return Err(IvError::NonPositivePrice(market_price));
    }
    if t <= 0.0 {
        return Err(IvError::Expired);
    }

    let f = |sigma: f64| -> f64 { theoretical_price(s, k, t, r, sigma, kind) - market_price };
    let mut a = SIGMA_LOWER_BOUND;
    let mut b = SIGMA_UPPER_BOUND;
    let mut fa = f(a);
    let mut fb = f(b);

    if fa * fb > 0.0 {
        // 同号无法套环；尝试边界返回
        if fa.abs() < DEFAULT_TOLERANCE {
            return Ok(a);
        }
        if fb.abs() < DEFAULT_TOLERANCE {
            return Ok(b);
        }
        return Err(IvError::BrentNoBracket {
            f_low: fa,
            f_high: fb,
        });
    }

    if fa.abs() < fb.abs() {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut fa, &mut fb);
    }

    let mut c = a;
    let mut fc = fa;
    let mut d = b - a;
    let mut e = d;
    let mut mflag = true;

    for _ in 0..DEFAULT_MAX_ITERATIONS {
        if fb.abs() < DEFAULT_TOLERANCE {
            return Ok(b);
        }
        if (b - a).abs() < DEFAULT_TOLERANCE * 0.5 {
            return Ok(b);
        }

        let s_new = if (fa - fc).abs() > DEFAULT_TOLERANCE && (fb - fc).abs() > DEFAULT_TOLERANCE {
            // 反二次插值
            a * fb * fc / ((fa - fb) * (fa - fc))
                + b * fa * fc / ((fb - fa) * (fb - fc))
                + c * fa * fb / ((fc - fa) * (fc - fb))
        } else {
            // 割线法
            b - fb * (b - a) / (fb - fa)
        };

        let lo = (3.0 * a + b) / 4.0;
        let hi = b;
        let in_range = (s_new - lo) * (s_new - hi) <= 0.0;
        // 经典 Brent 的"是否优先 bisection"判据：四种情况任一触发都退化到对分。
        let half = (s_new - b).abs();
        let cond_mflag_step = mflag && half >= (b - c).abs() / 2.0;
        let cond_nonmflag_step = !mflag && half >= (c - d).abs() / 2.0;
        let cond_mflag_tol = mflag && (b - c).abs() < DEFAULT_TOLERANCE;
        let cond_nonmflag_tol = !mflag && (c - d).abs() < DEFAULT_TOLERANCE;
        let bisect_better =
            cond_mflag_step || cond_nonmflag_step || cond_mflag_tol || cond_nonmflag_tol;

        let s_actual = if !in_range || bisect_better {
            mflag = true;
            (a + b) / 2.0
        } else {
            mflag = false;
            s_new
        };

        let fs = f(s_actual);
        d = c;
        c = b;
        fc = fb;

        if fa * fs < 0.0 {
            b = s_actual;
            fb = fs;
        } else {
            a = s_actual;
            fa = fs;
        }
        if fa.abs() < fb.abs() {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut fa, &mut fb);
        }
        let _ = e; // 保持变量存在以匹配经典 Brent；当前实现未使用
        e = d;
    }

    Err(IvError::BrentNotConverged)
}

/// 自动选择最佳方法：先牛顿，失败后回退 Brent。
pub fn solve_iv(
    market_price: f64,
    s: f64,
    k: f64,
    t: f64,
    r: f64,
    kind: OptionType,
) -> Result<f64, IvError> {
    match solve_iv_newton(market_price, s, k, t, r, kind) {
        Ok(iv) => Ok(iv),
        Err(_) => solve_iv_brent(market_price, s, k, t, r, kind),
    }
}

/// `便捷封装：days_to_expiry` → 年。
pub fn solve_iv_from_days(
    market_price: f64,
    s: f64,
    k: f64,
    days: f64,
    r: f64,
    kind: OptionType,
) -> Result<f64, IvError> {
    let t = days / 365.0;
    solve_iv(market_price, s, k, t, r, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::black_scholes::call_price;

    #[test]
    fn newton_recovers_known_iv() {
        let s = 30000.0;
        let k = 30000.0;
        let t = 30.0 / 365.0;
        let r = 0.05;
        let true_sigma = 0.6;
        let market = call_price(s, k, t, r, true_sigma);
        let iv = solve_iv_newton(market, s, k, t, r, OptionType::Call).unwrap();
        assert!((iv - true_sigma).abs() < 1e-6);
    }

    #[test]
    fn brent_recovers_known_iv() {
        let s = 30000.0;
        let k = 28000.0;
        let t = 60.0 / 365.0;
        let r = 0.05;
        let true_sigma = 0.45;
        let market = call_price(s, k, t, r, true_sigma);
        let iv = solve_iv_brent(market, s, k, t, r, OptionType::Call).unwrap();
        assert!((iv - true_sigma).abs() < 1e-6);
    }

    #[test]
    fn auto_solve_recovers_various_iv_levels() {
        let s = 30000.0;
        let k = 30000.0;
        let t = 30.0 / 365.0;
        let r = 0.05;
        for true_sigma in [0.1, 0.3, 0.6, 1.0, 1.5] {
            let market = call_price(s, k, t, r, true_sigma);
            let iv = solve_iv(market, s, k, t, r, OptionType::Call)
                .unwrap_or_else(|e| panic!("solver failed for sigma={true_sigma}: {e}"));
            assert!(
                (iv - true_sigma).abs() < 1e-5,
                "iv mismatch: {iv} vs {true_sigma}"
            );
        }
    }

    #[test]
    fn initial_guess_scales_with_normalized_option_price() {
        let low = initial_guess(1.0, 100.0, 30.0 / 365.0);
        let high = initial_guess(20.0, 100.0, 30.0 / 365.0);

        assert!(low < DEFAULT_INITIAL_GUESS);
        assert!(high > low);
    }

    #[test]
    fn errors_on_non_positive_price() {
        let err = solve_iv(0.0, 100.0, 100.0, 0.5, 0.05, OptionType::Call).unwrap_err();
        assert!(matches!(err, IvError::NonPositivePrice(_)));
    }

    #[test]
    fn errors_on_expired_option() {
        let err = solve_iv(1.0, 100.0, 100.0, 0.0, 0.05, OptionType::Call).unwrap_err();
        assert!(matches!(err, IvError::Expired));
    }

    #[test]
    fn from_days_helper_works() {
        let s = 30000.0;
        let k = 30000.0;
        let r = 0.05;
        let true_sigma = 0.6;
        let market = call_price(s, k, 30.0 / 365.0, r, true_sigma);
        let iv = solve_iv_from_days(market, s, k, 30.0, r, OptionType::Call).unwrap();
        assert!((iv - true_sigma).abs() < 1e-6);
    }

    #[test]
    fn put_iv_recovers_correctly() {
        let s = 30000.0;
        let k = 32000.0;
        let t = 30.0 / 365.0;
        let r = 0.05;
        let true_sigma = 0.7;
        let market = crate::black_scholes::put_price(s, k, t, r, true_sigma);
        let iv = solve_iv(market, s, k, t, r, OptionType::Put).unwrap();
        assert!((iv - true_sigma).abs() < 1e-5);
    }
}
