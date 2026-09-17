#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 期权定价与 Greeks 计算（Black-Scholes-Merton）+ IV 求解 + 多腿策略 + 0DTE 筛选。

pub mod black_scholes;
pub mod iv_solver;
pub mod strategies;
pub mod strategy;
pub mod zero_dte;

pub use black_scholes::{
    all_greeks, call_price, delta, gamma, greeks_from_days, put_price, rho, theta, vega,
};
pub use iv_solver::{solve_iv, solve_iv_brent, solve_iv_from_days, solve_iv_newton, IvError};
pub use strategy::{Leg, LegSide, Strategy, StrategyKind, StrategyRiskMetrics};
pub use zero_dte::{filter_zero_dte, hours_to_expiry, ZeroDteCandidate, ZERO_DTE_HOURS};
