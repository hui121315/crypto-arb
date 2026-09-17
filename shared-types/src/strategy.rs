//! 策略类型与绩效 DTO。

pub use crate::strategy_capabilities::{
    is_diagnostic_only_strategy, is_live_executable_strategy, is_p0_executable_strategy,
    is_phase2_strategy, strategy_execution_cycle, StrategyExecutionCycle,
    LIVE_EXECUTABLE_STRATEGY_KINDS, P0_EXECUTABLE_STRATEGY_KINDS, PHASE2_STRATEGY_KINDS,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    PerpCross,
    PerpPriceSpread,
    SpotPerp,
    CrossSpotPerp,
    SpotCross,
    Triangular,
    FundingCarry,
    CashAndCarry,
    OptionsPerpBasis,
    QuarterlyPerp,
    OnchainDepeg,
}

impl StrategyKind {
    #[must_use]
    pub const fn label_zh(&self) -> &'static str {
        match self {
            Self::PerpCross => "永续跨所",
            Self::PerpPriceSpread => "永续价差",
            Self::SpotPerp => "现货-永续",
            Self::CrossSpotPerp => "跨所期现",
            Self::SpotCross => "现货跨所",
            Self::Triangular => "三角套利",
            Self::FundingCarry => "资金费 Carry",
            Self::CashAndCarry => "现货持有",
            Self::OptionsPerpBasis => "期权-永续基差",
            Self::QuarterlyPerp => "季度-永续",
            Self::OnchainDepeg => "链上后续",
        }
    }

    #[must_use]
    pub const fn label_en(&self) -> &'static str {
        match self {
            Self::PerpCross => "Perp Cross",
            Self::PerpPriceSpread => "Perp Price Spread",
            Self::SpotPerp => "Spot Perp",
            Self::CrossSpotPerp => "Cross Spot Perp",
            Self::SpotCross => "Spot Cross",
            Self::Triangular => "Triangular",
            Self::FundingCarry => "Funding Carry",
            Self::CashAndCarry => "Cash and Carry",
            Self::OptionsPerpBasis => "Options Perp Basis",
            Self::QuarterlyPerp => "Quarterly Perp",
            Self::OnchainDepeg => "Onchain Follow-up",
        }
    }

    #[must_use]
    pub const fn as_query_value(self) -> &'static str {
        match self {
            Self::PerpCross => "perp_cross",
            Self::PerpPriceSpread => "perp_price_spread",
            Self::SpotPerp => "spot_perp",
            Self::CrossSpotPerp => "cross_spot_perp",
            Self::SpotCross => "spot_cross",
            Self::Triangular => "triangular",
            Self::FundingCarry => "funding_carry",
            Self::CashAndCarry => "cash_and_carry",
            Self::OptionsPerpBasis => "options_perp_basis",
            Self::QuarterlyPerp => "quarterly_perp",
            Self::OnchainDepeg => "onchain_depeg",
        }
    }

    #[must_use]
    pub fn from_query_value(value: &str) -> Option<Self> {
        match value {
            "perp_cross" => Some(Self::PerpCross),
            "perp_price_spread" => Some(Self::PerpPriceSpread),
            "spot_perp" => Some(Self::SpotPerp),
            "cross_spot_perp" => Some(Self::CrossSpotPerp),
            "spot_cross" => Some(Self::SpotCross),
            "triangular" => Some(Self::Triangular),
            "funding_carry" => Some(Self::FundingCarry),
            "cash_and_carry" => Some(Self::CashAndCarry),
            "options_perp_basis" => Some(Self::OptionsPerpBasis),
            "quarterly_perp" => Some(Self::QuarterlyPerp),
            "onchain_depeg" => Some(Self::OnchainDepeg),
            _ => None,
        }
    }

    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::PerpCross => "不同交易所永续资金费与价格错配。",
            Self::PerpPriceSpread => "不同交易所同一永续标的的可执行买卖价差与收敛机会。",
            Self::SpotPerp => "同所现货与永续之间的基差或资金费机会。",
            Self::CrossSpotPerp => "跨交易所现货腿与永续腿组合机会。",
            Self::SpotCross => "不同交易所现货价格差。",
            Self::Triangular => "同一交易场所三币种路径错价。",
            Self::FundingCarry => "资金费方向性 carry 与持仓周期收益。",
            Self::CashAndCarry => "现货持有配合永续或交割合约套保。",
            Self::OptionsPerpBasis => "期权组合与永续基差之间的相对价值。",
            Self::QuarterlyPerp => "交割合约与永续之间的期限基差。",
            Self::OnchainDepeg => "链上池子或锚定资产的后续机会。",
        }
    }

    #[must_use]
    pub const fn category(&self) -> StrategyCategory {
        match self {
            Self::PerpCross
            | Self::PerpPriceSpread
            | Self::SpotPerp
            | Self::CrossSpotPerp
            | Self::CashAndCarry
            | Self::QuarterlyPerp => StrategyCategory::Futures,
            Self::SpotCross | Self::Triangular => StrategyCategory::Spot,
            Self::OnchainDepeg => StrategyCategory::Onchain,
            Self::FundingCarry => StrategyCategory::FundingYield,
            Self::OptionsPerpBasis => StrategyCategory::Options,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyCategory {
    Futures,
    Spot,
    Onchain,
    FundingYield,
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyExposure {
    MainP0,
    Diagnostic,
    Hidden,
}

impl StrategyExposure {
    #[must_use]
    pub const fn for_kind(kind: StrategyKind) -> Self {
        if is_p0_executable_strategy(kind) {
            Self::MainP0
        } else if is_diagnostic_only_strategy(kind) || is_phase2_strategy(kind) {
            Self::Diagnostic
        } else {
            Self::Hidden
        }
    }

    #[must_use]
    pub const fn is_main_p0(self) -> bool {
        matches!(self, Self::MainP0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyKindInfo {
    pub kind: StrategyKind,
    pub category: StrategyCategory,
    pub label_zh: String,
    pub label_en: String,
    pub description: String,
    pub implemented: bool,
    pub frontend_enabled: bool,
    pub exposure: StrategyExposure,
    #[serde(default)]
    pub engine_present: bool,
    #[serde(default)]
    pub data_contract_ready: bool,
    #[serde(default)]
    pub execution_supported: bool,
    #[serde(default)]
    pub live_execution_supported: bool,
}

impl StrategyKindInfo {
    #[must_use]
    pub fn from_kind(kind: StrategyKind, implemented: bool) -> Self {
        let exposure = StrategyExposure::for_kind(kind);
        let main_p0 = exposure.is_main_p0();
        let execution_supported = main_p0 && implemented;
        let live_execution_supported = execution_supported && is_live_executable_strategy(kind);
        Self {
            kind,
            category: kind.category(),
            label_zh: kind.label_zh().to_owned(),
            label_en: kind.label_en().to_owned(),
            description: kind.description().to_owned(),
            implemented,
            frontend_enabled: main_p0,
            exposure,
            engine_present: execution_supported,
            data_contract_ready: main_p0,
            execution_supported,
            live_execution_supported,
        }
    }

    #[must_use]
    pub const fn is_main_p0_executable(&self) -> bool {
        self.exposure.is_main_p0()
            && self.engine_present
            && self.data_contract_ready
            && self.execution_supported
    }

    #[must_use]
    pub const fn is_main_p0_live_executable(&self) -> bool {
        self.is_main_p0_executable() && self.live_execution_supported
    }
}

mod performance;
pub use performance::{StrategyPerformance, StrategyPerformanceSampleStatus};

#[cfg(test)]
#[path = "strategy_tests.rs"]
mod tests;
