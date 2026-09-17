#!/usr/bin/env python3
"""
Black-Scholes scipy comparison fixture generator.

使用 Python scipy.stats.norm 实现的 Black-Scholes-Merton 模型，
为多组参数计算期权理论价格 + 全部 Greeks，输出为 JSON 供 Rust 测试比对。
"""
import json
import math
from pathlib import Path

from scipy.stats import norm


def d1(S, K, T, r, sigma):
    if T <= 0 or sigma <= 0:
        return 0.0
    return (math.log(S / K) + (r + 0.5 * sigma ** 2) * T) / (sigma * math.sqrt(T))


def d2(S, K, T, r, sigma):
    return d1(S, K, T, r, sigma) - sigma * math.sqrt(T)


def call_price(S, K, T, r, sigma):
    if T <= 0:
        return max(0.0, S - K)
    return S * norm.cdf(d1(S, K, T, r, sigma)) - K * math.exp(-r * T) * norm.cdf(
        d2(S, K, T, r, sigma)
    )


def put_price(S, K, T, r, sigma):
    if T <= 0:
        return max(0.0, K - S)
    return K * math.exp(-r * T) * norm.cdf(-d2(S, K, T, r, sigma)) - S * norm.cdf(
        -d1(S, K, T, r, sigma)
    )


def delta(S, K, T, r, sigma, kind):
    if T <= 0:
        if kind == "call":
            return 1.0 if S > K else 0.0
        return -1.0 if S < K else 0.0
    d_1 = d1(S, K, T, r, sigma)
    return norm.cdf(d_1) if kind == "call" else norm.cdf(d_1) - 1


def gamma(S, K, T, r, sigma):
    if T <= 0 or sigma <= 0:
        return 0.0
    d_1 = d1(S, K, T, r, sigma)
    return norm.pdf(d_1) / (S * sigma * math.sqrt(T))


def vega(S, K, T, r, sigma):
    """Vega per 1% volatility change."""
    if T <= 0:
        return 0.0
    d_1 = d1(S, K, T, r, sigma)
    return S * math.sqrt(T) * norm.pdf(d_1) / 100.0


def theta(S, K, T, r, sigma, kind):
    """Theta per day."""
    if T <= 0:
        return 0.0
    d_1 = d1(S, K, T, r, sigma)
    d_2 = d2(S, K, T, r, sigma)
    term1 = -(S * norm.pdf(d_1) * sigma) / (2 * math.sqrt(T))
    if kind == "call":
        annual = term1 - r * K * math.exp(-r * T) * norm.cdf(d_2)
    else:
        annual = term1 + r * K * math.exp(-r * T) * norm.cdf(-d_2)
    return annual / 365.0


def rho(S, K, T, r, sigma, kind):
    """Rho per 1% interest rate change."""
    if T <= 0:
        return 0.0
    d_2 = d2(S, K, T, r, sigma)
    if kind == "call":
        return K * T * math.exp(-r * T) * norm.cdf(d_2) / 100.0
    return -K * T * math.exp(-r * T) * norm.cdf(-d_2) / 100.0


def fixture(S, K, days, r, sigma):
    T = days / 365.0
    return {
        "spot_price": S,
        "strike": K,
        "days_to_expiry": days,
        "risk_free_rate": r,
        "volatility": sigma,
        "call_price": call_price(S, K, T, r, sigma),
        "put_price": put_price(S, K, T, r, sigma),
        "call_delta": delta(S, K, T, r, sigma, "call"),
        "put_delta": delta(S, K, T, r, sigma, "put"),
        "gamma": gamma(S, K, T, r, sigma),
        "vega": vega(S, K, T, r, sigma),
        "call_theta": theta(S, K, T, r, sigma, "call"),
        "put_theta": theta(S, K, T, r, sigma, "put"),
        "call_rho": rho(S, K, T, r, sigma, "call"),
        "put_rho": rho(S, K, T, r, sigma, "put"),
    }


def main():
    cases = []

    # ATM 标准案例（不同到期天数）
    for days in [1, 7, 30, 60, 90, 180, 365]:
        cases.append(fixture(30000.0, 30000.0, days, 0.05, 0.6))

    # OTM / ITM Call & Put（30 天到期）
    for K in [25000.0, 28000.0, 30000.0, 32000.0, 35000.0]:
        cases.append(fixture(30000.0, K, 30, 0.05, 0.6))

    # 不同波动率（30 天到期 ATM）
    for sigma in [0.2, 0.4, 0.6, 0.8, 1.0, 1.5]:
        cases.append(fixture(30000.0, 30000.0, 30, 0.05, sigma))

    # 不同利率
    for r in [0.0, 0.02, 0.05, 0.08, 0.10]:
        cases.append(fixture(30000.0, 30000.0, 30, r, 0.6))

    # 边界：极短到期 + 0DTE
    cases.append(fixture(30000.0, 30000.0, 0, 0.05, 0.6))  # T=0
    cases.append(fixture(30000.0, 28000.0, 0, 0.05, 0.6))
    cases.append(fixture(30000.0, 32000.0, 0, 0.05, 0.6))

    # ETH 量级（数值范围验证）
    cases.append(fixture(2000.0, 2000.0, 30, 0.05, 0.6))
    cases.append(fixture(2000.0, 1800.0, 60, 0.05, 0.7))
    cases.append(fixture(2000.0, 2200.0, 90, 0.05, 0.5))

    out = {
        "schema_version": 1,
        "generator": "scipy.stats.norm",
        "tolerance": 1e-9,
        "case_count": len(cases),
        "cases": cases,
    }

    target = Path(__file__).parent / "black_scholes_scipy.json"
    target.write_text(json.dumps(out, indent=2))
    print(f"wrote {len(cases)} cases to {target}")


if __name__ == "__main__":
    main()
