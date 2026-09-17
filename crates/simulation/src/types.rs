//! 模拟交易核心数据模型。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PositionSide {
    Long,
    Short,
}

impl PositionSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            PositionSide::Long => "long",
            PositionSide::Short => "short",
        }
    }
}

/// 模拟仓位。`current_price` 由 `update_marks` 写入；其余字段在开仓时锁定。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimPosition {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    pub side: PositionSide,
    pub quantity: f64,
    pub entry_price: f64,
    pub current_price: f64,
    #[serde(default = "default_leverage")]
    pub leverage: f64,
    pub opened_at: DateTime<Utc>,
    #[serde(default)]
    pub fees_paid: f64,
    #[serde(default)]
    pub note: String,
}

impl SimPosition {
    /// 未实现盈亏（按 `current_price` 计算）。
    pub fn unrealized_pnl(&self) -> f64 {
        self.mark_pnl() - self.fees_paid
    }

    /// 仅价格变化产生的未实现盈亏，不重复扣除已从现金扣掉的开仓手续费。
    pub fn mark_pnl(&self) -> f64 {
        match self.side {
            PositionSide::Long => (self.current_price - self.entry_price) * self.quantity,
            PositionSide::Short => (self.entry_price - self.current_price) * self.quantity,
        }
    }

    /// 仓位名义价值（USD），按当前价格计算。
    pub fn notional_value(&self) -> f64 {
        self.current_price * self.quantity
    }

    /// 入场名义价值（USD），按入场价格计算。
    pub fn entry_notional(&self) -> f64 {
        self.entry_price * self.quantity
    }

    /// 已占用保证金（USD），按入场名义价值 / 杠杆计算。
    pub fn margin_used(&self) -> f64 {
        self.entry_notional() / self.leverage.max(1.0)
    }
}

/// 已平仓交易记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub position: SimPosition,
    pub close_price: f64,
    pub closed_at: DateTime<Utc>,
    pub realized_pnl: f64,
    #[serde(default)]
    pub close_fees: f64,
}

/// 开仓请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRequest {
    pub symbol: String,
    pub exchange: String,
    pub side: PositionSide,
    pub quantity: f64,
    pub entry_price: f64,
    #[serde(default = "default_leverage")]
    pub leverage: f64,
    #[serde(default)]
    pub fees: f64,
    #[serde(default)]
    pub note: String,
}

pub fn default_leverage() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn position(side: PositionSide, entry: f64, current: f64, qty: f64) -> SimPosition {
        SimPosition {
            id: "p1".into(),
            symbol: "BTC".into(),
            exchange: "binance".into(),
            side,
            quantity: qty,
            entry_price: entry,
            current_price: current,
            leverage: 1.0,
            opened_at: Utc::now(),
            fees_paid: 0.0,
            note: String::new(),
        }
    }

    #[test]
    fn long_pnl_positive_when_price_rises() {
        let p = position(PositionSide::Long, 100.0, 110.0, 2.0);
        assert!((p.unrealized_pnl() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn long_pnl_negative_when_price_falls() {
        let p = position(PositionSide::Long, 100.0, 90.0, 1.0);
        assert!((p.unrealized_pnl() - (-10.0)).abs() < 1e-9);
    }

    #[test]
    fn short_pnl_positive_when_price_falls() {
        let p = position(PositionSide::Short, 100.0, 90.0, 2.0);
        assert!((p.unrealized_pnl() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn short_pnl_negative_when_price_rises() {
        let p = position(PositionSide::Short, 100.0, 110.0, 1.0);
        assert!((p.unrealized_pnl() - (-10.0)).abs() < 1e-9);
    }

    #[test]
    fn fees_subtracted_from_pnl() {
        let mut p = position(PositionSide::Long, 100.0, 110.0, 2.0);
        p.fees_paid = 5.0;
        assert!((p.unrealized_pnl() - 15.0).abs() < 1e-9);
    }

    #[test]
    fn notional_values_match() {
        let p = position(PositionSide::Long, 100.0, 110.0, 0.5);
        assert!((p.entry_notional() - 50.0).abs() < 1e-9);
        assert!((p.notional_value() - 55.0).abs() < 1e-9);
        assert!((p.margin_used() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn position_side_serializes_lowercase() {
        let s = serde_json::to_string(&PositionSide::Long).unwrap();
        assert_eq!(s, "\"long\"");
    }
}
