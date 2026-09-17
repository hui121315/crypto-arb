//! 多腿期权策略数据模型 + PnL 计算 + Greeks 聚合。

use crate::black_scholes::{all_greeks, call_price, put_price};
use serde::{Deserialize, Serialize};
use shared_types::{OptionGreeks, OptionType};

/// 单条策略腿：买卖方向 + 期权类型 + 行权价 + 数量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leg {
    /// `long`（买入）或 `short`（卖出）。
    pub side: LegSide,
    /// Call / Put
    pub option_type: OptionType,
    pub strike: f64,
    /// 张数（合约数量）；不区分 sign，方向由 `side` 决定。
    pub quantity: f64,
    /// 入场权利金（单张）。
    pub premium: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegSide {
    Long,
    Short,
}

impl LegSide {
    /// `+1.0` for Long, `-1.0` for Short.
    pub fn signed_multiplier(self) -> f64 {
        match self {
            LegSide::Long => 1.0,
            LegSide::Short => -1.0,
        }
    }
}

/// 策略类型：决定典型用途与展示标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    LongCall,
    LongPut,
    ShortCall,
    ShortPut,
    BullCallSpread,
    BearPutSpread,
    LongStraddle,
    LongStrangle,
    ShortStraddle,
    IronCondor,
    Butterfly,
    ProtectivePut,
    CoveredCall,
    Custom,
}

/// 完整多腿策略。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub kind: StrategyKind,
    pub name: String,
    pub legs: Vec<Leg>,
    /// 假设的标的现价（创建时快照），用于 net premium 计算
    pub spot_price: f64,
}

impl Strategy {
    pub fn new(kind: StrategyKind, name: impl Into<String>, legs: Vec<Leg>, spot: f64) -> Self {
        Self {
            kind,
            name: name.into(),
            legs,
            spot_price: spot,
        }
    }

    /// 净权利金：买腿成本 - 卖腿收入。正数 = 净支付（debit），负数 = 净收入（credit）。
    pub fn net_premium(&self) -> f64 {
        self.legs
            .iter()
            .map(|l| l.side.signed_multiplier() * l.premium * l.quantity)
            .sum()
    }

    /// 到期时给定 spot 价格 `s` 的总盈亏。
    pub fn pnl_at_expiry(&self, s: f64) -> f64 {
        self.legs
            .iter()
            .map(|l| leg_pnl_at_expiry(l, s))
            .sum::<f64>()
    }

    /// 当前 spot/iv/rate 下的策略市值（每条腿用 BS 估值）。
    pub fn current_value(&self, s: f64, t: f64, r: f64, sigma: f64) -> f64 {
        self.legs
            .iter()
            .map(|l| {
                let theo = match l.option_type {
                    OptionType::Call => call_price(s, l.strike, t, r, sigma),
                    OptionType::Put => put_price(s, l.strike, t, r, sigma),
                };
                l.side.signed_multiplier() * theo * l.quantity
            })
            .sum()
    }

    /// 当前未实现盈亏 = 当前市值 - 已支付权利金。
    pub fn current_unrealized_pnl(&self, s: f64, t: f64, r: f64, sigma: f64) -> f64 {
        self.current_value(s, t, r, sigma) - self.net_premium()
    }

    /// 聚合 Greeks（带方向系数 + 数量加权）。
    pub fn aggregate_greeks(&self, s: f64, t: f64, r: f64, sigma: f64) -> OptionGreeks {
        let mut acc = OptionGreeks {
            delta: 0.0,
            gamma: 0.0,
            theta: 0.0,
            vega: 0.0,
            rho: 0.0,
            iv: sigma,
        };
        for l in &self.legs {
            let g = all_greeks(s, l.strike, t, r, sigma, l.option_type);
            let mult = l.side.signed_multiplier() * l.quantity;
            acc.delta += g.delta * mult;
            acc.gamma += g.gamma * mult;
            acc.theta += g.theta * mult;
            acc.vega += g.vega * mult;
            acc.rho += g.rho * mult;
        }
        acc
    }

    /// 在 `[lo, hi]` 价格区间均匀采样 `n` 个点的到期 PnL 曲线。
    pub fn pnl_curve(&self, lo: f64, hi: f64, n: usize) -> Vec<(f64, f64)> {
        if n < 2 || lo >= hi {
            return Vec::new();
        }
        let step = (hi - lo) / (n as f64 - 1.0);
        (0..n)
            .map(|i| {
                let s = lo + step * i as f64;
                (s, self.pnl_at_expiry(s))
            })
            .collect()
    }

    /// 在采样曲线内估算最大盈利 / 最大亏损 / 盈亏平衡点。
    ///
    /// 注意：这是数值近似，对极端策略可能不够精确；解析解推荐由具体策略子模块覆盖。
    pub fn risk_metrics(&self, lo: f64, hi: f64, n: usize) -> StrategyRiskMetrics {
        let curve = self.pnl_curve(lo, hi, n);
        if curve.is_empty() {
            return StrategyRiskMetrics::default();
        }
        let (mut max_p, mut min_p) = (f64::NEG_INFINITY, f64::INFINITY);
        let mut breakevens = Vec::new();
        for w in curve.windows(2) {
            let (s1, p1) = w[0];
            let (_s2, p2) = w[1];
            if p1 > max_p {
                max_p = p1;
            }
            if p1 < min_p {
                min_p = p1;
            }
            // 线性插值寻找盈亏平衡（PnL 过零点）
            if p1.signum() != p2.signum() && p1.is_finite() && p2.is_finite() {
                let (s_a, p_a) = w[0];
                let (s_b, p_b) = w[1];
                let s_zero = s_a + (0.0 - p_a) * (s_b - s_a) / (p_b - p_a);
                breakevens.push(s_zero);
            }
            let _ = s1;
        }
        // 末点
        if let Some(&(_, p_last)) = curve.last() {
            if p_last > max_p {
                max_p = p_last;
            }
            if p_last < min_p {
                min_p = p_last;
            }
        }
        StrategyRiskMetrics {
            max_profit: max_p,
            max_loss: min_p,
            breakeven_points: breakevens,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyRiskMetrics {
    pub max_profit: f64,
    pub max_loss: f64,
    pub breakeven_points: Vec<f64>,
}

fn leg_pnl_at_expiry(leg: &Leg, s: f64) -> f64 {
    let intrinsic = match leg.option_type {
        OptionType::Call => (s - leg.strike).max(0.0),
        OptionType::Put => (leg.strike - s).max(0.0),
    };
    let payoff = intrinsic - leg.premium;
    leg.side.signed_multiplier() * payoff * leg.quantity
}

#[cfg(test)]
mod tests;
