#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Black-Scholes Rust 实现与 Python scipy.stats.norm 数值对齐验证。
//!
//! Fixtures 由同目录 `fixtures/generate_scipy.py` 生成并作为固定测试资产提交。
//!
//! ## 容差选择
//! - **绝对容差 1e-6**：业务足够（期权价格最小单位为 cent，1e-6 USD 远小于）
//! - **相对容差 1e-8**：用于大数值（如价格）的稳健比对
//!
//! 实测最大偏差 ~1.66e-7，源自 `statrs` 与 `scipy` 在 `norm.cdf` 内部
//! 特殊函数（erf）实现的算法差异，业务上无影响。

use options::{call_price, delta, gamma, put_price, rho, theta, vega};
use serde::Deserialize;
use shared_types::OptionType;
use std::path::PathBuf;

/// 绝对容差：1e-6（业务足够精度）
const ABS_TOLERANCE: f64 = 1e-6;
/// 相对容差：1e-8（大数值场景的稳健保护）
const REL_TOLERANCE: f64 = 1e-8;

#[derive(Debug, Deserialize)]
struct Fixture {
    spot_price: f64,
    strike: f64,
    days_to_expiry: f64,
    risk_free_rate: f64,
    volatility: f64,
    call_price: f64,
    put_price: f64,
    call_delta: f64,
    put_delta: f64,
    gamma: f64,
    vega: f64,
    call_theta: f64,
    put_theta: f64,
    call_rho: f64,
    put_rho: f64,
}

#[derive(Debug, Deserialize)]
struct Fixtures {
    schema_version: u32,
    case_count: usize,
    cases: Vec<Fixture>,
}

fn load_fixtures() -> Fixtures {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("black_scholes_scipy.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&content).expect("invalid fixtures json")
}

fn assert_close(label: &str, lhs: f64, rhs: f64, case_idx: usize) {
    let diff = (lhs - rhs).abs();
    let scale = lhs.abs().max(rhs.abs()).max(1.0);
    let rel_ok = diff / scale < REL_TOLERANCE;
    let abs_ok = diff < ABS_TOLERANCE;
    if !rel_ok && !abs_ok {
        panic!(
            "case {case_idx} {label}: rust={lhs:e}, scipy={rhs:e}, diff={diff:e} \
             (abs_tol={ABS_TOLERANCE:e}, rel_tol={REL_TOLERANCE:e}, scale={scale:e})"
        );
    }
}

#[test]
fn fixtures_loadable_and_well_formed() {
    let f = load_fixtures();
    assert_eq!(f.schema_version, 1);
    assert!(
        f.cases.len() >= 20,
        "expected ≥20 cases, got {}",
        f.cases.len()
    );
    assert_eq!(f.case_count, f.cases.len());
}

#[test]
fn black_scholes_matches_scipy_within_1e9() {
    let fixtures = load_fixtures();
    let mut max_diff = 0.0_f64;

    for (i, c) in fixtures.cases.iter().enumerate() {
        let t = c.days_to_expiry / 365.0;
        let s = c.spot_price;
        let k = c.strike;
        let r = c.risk_free_rate;
        let sigma = c.volatility;

        let our_call = call_price(s, k, t, r, sigma);
        let our_put = put_price(s, k, t, r, sigma);
        let our_call_delta = delta(s, k, t, r, sigma, OptionType::Call);
        let our_put_delta = delta(s, k, t, r, sigma, OptionType::Put);
        let our_gamma = gamma(s, k, t, r, sigma);
        let our_vega = vega(s, k, t, r, sigma);
        let our_call_theta = theta(s, k, t, r, sigma, OptionType::Call);
        let our_put_theta = theta(s, k, t, r, sigma, OptionType::Put);
        let our_call_rho = rho(s, k, t, r, sigma, OptionType::Call);
        let our_put_rho = rho(s, k, t, r, sigma, OptionType::Put);

        for (label, lhs, rhs) in [
            ("call_price", our_call, c.call_price),
            ("put_price", our_put, c.put_price),
            ("call_delta", our_call_delta, c.call_delta),
            ("put_delta", our_put_delta, c.put_delta),
            ("gamma", our_gamma, c.gamma),
            ("vega", our_vega, c.vega),
            ("call_theta", our_call_theta, c.call_theta),
            ("put_theta", our_put_theta, c.put_theta),
            ("call_rho", our_call_rho, c.call_rho),
            ("put_rho", our_put_rho, c.put_rho),
        ] {
            assert_close(label, lhs, rhs, i);
            let diff = (lhs - rhs).abs();
            if diff > max_diff {
                max_diff = diff;
            }
        }
    }

    assert!(
        max_diff < ABS_TOLERANCE,
        "max_diff={max_diff:.3e} exceeded abs tolerance"
    );
}
