//! 业务枚举。

use serde::{Deserialize, Serialize};

/// 套利类型。
///
/// 对应 Python `core/arbitrage/interfaces.py` 中的 `ArbitrageType`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArbitrageType {
    /// 跨所期期：A 所永续 vs B 所永续。
    CrossExchange,
    /// 同所期现：现货做多 + 永续做空（同一交易所）。
    SpotFutures,
    /// 跨所期现：A 所现货 + B 所永续。
    CrossSpotFutures,
    /// 跨所现货：A 所买入现货 + B 所卖出现货。
    SpotCross,
    /// 同 venue 三角套利路径。
    Triangular,
    /// 单腿资金费方向 carry。
    FundingCarry,
    /// 期权时间价值与永续资金费相对价值。
    OptionsPerpBasis,
}

/// 风险等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

/// 推荐等级（中文标签会在序列化层做映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recommendation {
    StrongBuy,
    Buy,
    Hold,
    Avoid,
}

/// 期权类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionType {
    Call,
    Put,
}

/// 订单方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// 订单类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Market,
    Limit,
    PostOnly,
}

/// 订单状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Pending,
    Open,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn arbitrage_type_serializes_to_snake_case() -> Result<(), serde_json::Error> {
        let s = serde_json::to_string(&ArbitrageType::CrossExchange)?;
        assert_eq!(s, "\"cross_exchange\"");
        let s = serde_json::to_string(&ArbitrageType::FundingCarry)?;
        assert_eq!(s, "\"funding_carry\"");
        Ok(())
    }

    #[test]
    fn risk_level_round_trip() -> Result<(), serde_json::Error> {
        let r: RiskLevel = serde_json::from_str("\"medium\"")?;
        assert_eq!(r, RiskLevel::Medium);
        Ok(())
    }
}
