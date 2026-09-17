//! 机会列表视图模型的展示文案与默认下单参数：尺寸/资本/杠杆默认值、成本/周期文案。
//! 结构与构造见 `model.rs`，派生/格式化助手见 `format.rs`。

use shared_types::{RiskLevel, SpotLegMode, StrategyKind};

use crate::panels::modules::cost_copy::fee_evidence_label;

use super::format::{countdown, money, row_freshness_line};
use super::model::OpportunityListViewModel;
use crate::panels::modules::rate_format::unsigned_bps_percent;

impl OpportunityListViewModel {
    pub(crate) fn default_size_usd(&self) -> f64 {
        if self.optimal_position.is_finite() && self.optimal_position > 0.0 {
            self.optimal_position
        } else if self.max_position.is_finite() && self.max_position > 0.0 {
            self.max_position
        } else {
            0.0
        }
    }

    pub(crate) fn default_capital_usd(&self) -> f64 {
        let capital = self.default_size_usd() * 0.5;
        if capital <= f64::EPSILON {
            0.0
        } else {
            capital.min(1_000_000.0)
        }
    }

    pub(crate) fn default_leverage(&self) -> f64 {
        match self.risk_level {
            RiskLevel::Low => 2.0,
            RiskLevel::Medium => 1.5,
            RiskLevel::High => 1.0,
        }
    }

    /// 行内短形式：按策略显示真实兑现条件，而不是把无结算周期误写为 `0s`。
    pub(crate) fn realization_label(&self) -> String {
        match self.strategy_kind {
            Some(StrategyKind::SpotCross) => "双腿终态确认".into(),
            Some(StrategyKind::PerpPriceSpread) => "价差收敛后确认".into(),
            _ => countdown(
                self.settlement_countdown_seconds,
                self.time_to_settlement_ms,
            ),
        }
    }

    pub(crate) fn tte(&self) -> String {
        row_freshness_line(
            &self.realization_label(),
            &self.data_source,
            self.updated_at_ms,
        )
    }

    pub(crate) fn opportunity_size_label(&self) -> String {
        let size = self.default_size_usd();
        if size > f64::EPSILON {
            money(size)
        } else {
            "构建时核验".into()
        }
    }

    pub(crate) fn depth_evidence_label(&self) -> String {
        "点击构建后核验".into()
    }

    pub(crate) fn depth_detail(&self) -> String {
        "点击构建对冲后读取双腿实时 0.05% 盘口，并按目标金额核验".into()
    }

    pub(crate) fn cost_evidence_label(&self) -> String {
        fee_evidence_label(self.fee_evidence_count, self.fee_evidence_complete)
    }

    pub(crate) fn cost_detail(&self) -> String {
        if !self.cost_verified {
            return format!("{} · {}", self.round_trip_cost, self.cost_evidence_label());
        }
        format!(
            "回合成本 {} · 磨损 {} · 单次净利 {} · {}",
            self.round_trip_cost,
            unsigned_bps_percent(self.cost_wear_bps),
            self.one_cycle_net,
            self.cost_evidence_label()
        )
    }

    pub(crate) fn opportunity_reason(&self) -> String {
        if let Some(blocker) = self.execution_blockers.first() {
            blocker.clone()
        } else {
            format!(
                "{} 与 {} 存在可执行 funding / basis 差。",
                self.long_venue, self.short_venue
            )
        }
    }

    pub(crate) fn execution_blocker_summary(&self) -> Option<&'static str> {
        if self.execution_eligible {
            return None;
        }
        Some(
            self.execution_blockers
                .first()
                .map_or("存在执行阻断", |reason| blocker_summary(reason)),
        )
    }

    pub(crate) fn spot_leg_mode_label(&self) -> Option<&'static str> {
        self.spot_leg_mode.map(SpotLegMode::label_zh)
    }

    pub(crate) fn cycle_label(&self) -> String {
        self.realization_label()
    }

    pub(crate) fn cycle_detail(&self) -> String {
        self.tte()
    }
}

fn blocker_summary(reason: &str) -> &'static str {
    if contains_any(
        reason,
        &[
            "资金费率",
            "funding rate",
            "funding evidence",
            "funding interval",
        ],
    ) {
        "Funding 证据不完整"
    } else if contains_any(
        reason,
        &[
            "价差收敛证据",
            "历史闭环",
            "盈利闭环",
            "convergence evidence",
        ],
    ) {
        "价差收敛证据不足"
    } else if contains_any(
        reason,
        &["退出", "借贷", "持有成本", "exit", "borrow", "holding cost"],
    ) {
        "退出/持有规则未闭环"
    } else if contains_any(reason, &["结算", "settlement", "funding time"]) {
        "结算窗口未对齐"
    } else if contains_any(reason, &["深度", "盘口", "depth", "DEPTH"]) {
        "等待构建时深度"
    } else if contains_any(reason, &["身份", "identity", "IDENTITY"]) {
        "标的身份未通过"
    } else if contains_any(reason, &["指数成分", "底层一致", "index composition"]) {
        "指数成分未通过"
    } else if contains_any(
        reason,
        &[
            "挂牌",
            "规格",
            "registry",
            "REGISTRY",
            "instrument",
            "INSTRUMENT",
        ],
    ) {
        "执行规格未通过"
    } else if contains_any(reason, &["报价资产", "quote asset", "stablecoin"]) {
        "报价资产未对齐"
    } else if contains_any(reason, &["成本", "费率", "cost", "COST", "fee", "FEE"]) {
        "成本证据不完整"
    } else if contains_any(
        reason,
        &["行情", "价格", "market data", "MARKET_DATA", "WS"],
    ) {
        "行情证据不完整"
    } else if contains_any(
        reason,
        &[
            "凭证",
            "权限",
            "账户",
            "余额",
            "credential",
            "account",
            "balance",
        ],
    ) {
        "账户或权限未就绪"
    } else {
        "存在执行阻断"
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

#[cfg(test)]
mod decision_display_tests {
    use super::blocker_summary;

    #[test]
    fn decision_display_groups_verbose_blockers_without_hiding_the_source() {
        assert_eq!(
            blocker_summary("缺交易所挂牌证据或可执行规格：GATE 官方已挂牌但执行规格未通过"),
            "执行规格未通过"
        );
        assert_eq!(blocker_summary("等待双腿 0.05% 盘口深度"), "等待构建时深度");
        assert_eq!(
            blocker_summary("永续价差缺少双腿资金费率与结算频率证据，仅观察不执行"),
            "Funding 证据不完整"
        );
        assert_eq!(
            blocker_summary("永续价差收敛证据不足：可执行报价样本 0/60，历史闭环 0/3，盈利闭环 0"),
            "价差收敛证据不足"
        );
        assert_eq!(
            blocker_summary("期现策略尚缺票据绑定的退出、借贷与持有成本下限，仅观察不执行"),
            "退出/持有规则未闭环"
        );
        assert_eq!(blocker_summary("经济标的身份未通过"), "标的身份未通过");
        assert_eq!(
            blocker_summary("COTI 双边指数成分重合度 37% 低于 75%"),
            "指数成分未通过"
        );
        assert_eq!(blocker_summary("unknown blocker"), "存在执行阻断");
    }
}
