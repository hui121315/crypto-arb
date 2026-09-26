use super::slots::CategoryReadiness;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus};
use shared_types::{RiskStatusSlot, SystemHealth, TradingStatusResponse};

use super::problem_ledger::problem_breakdown;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SystemHealthSummary {
    pub state: &'static str,
    pub label: &'static str,
    pub detail: String,
}

impl SystemHealthSummary {
    pub(super) fn with_trading_status(mut self, status: &LoadState<TradingStatusResponse>) -> Self {
        if self.state == "healthy" && !matches!(status, LoadState::Ready(_)) {
            self.state = "unknown";
            self.label = "交易状态待确认";
            self.detail = status.problem().map_or_else(
                || "等待当前连接的交易模式与风控配置".into(),
                |problem| problem.message.clone(),
            );
        }
        self
    }

    pub(super) fn with_module(mut self, name: &str, module: &ModuleRuntimeState) -> Self {
        let (state, label) = match module.status {
            ModuleRuntimeStatus::Ready => return self,
            ModuleRuntimeStatus::Error => ("degraded", "当前模块异常"),
            ModuleRuntimeStatus::Stale => ("warning", "当前数据已过期"),
            ModuleRuntimeStatus::SetupRequired => ("unknown", "当前模块待配置"),
            ModuleRuntimeStatus::Loading => ("unknown", "当前模块加载中"),
            ModuleRuntimeStatus::Pending => ("unknown", "当前操作待确认"),
        };
        let detail = format!(
            "{name} · {}",
            module.pending_label.as_deref().unwrap_or(module.label())
        );
        // Page state must not downgrade a backend risk block or warning.
        if self.state == "healthy" || (self.state == "unknown" && state != "unknown") {
            self.state = state;
            self.label = label;
            self.detail = detail;
        } else {
            self.detail = format!("{} · {detail}", self.detail);
        }
        self
    }

    pub(super) fn with_readiness(mut self, categories: &[CategoryReadiness]) -> Self {
        let Some(issue) = categories
            .iter()
            .filter(|item| item.readiness.needs_attention())
            .max_by_key(|item| item.readiness)
        else {
            return self;
        };
        // Explicit account risk remains primary; missing evidence cannot look healthy.
        if self.state == "healthy"
            || (self.state == "unknown" && issue.readiness.state() != "unknown")
        {
            self.state = issue.readiness.state();
            self.label = match self.state {
                "degraded" => "运行状态异常",
                "warning" => "运行状态降级",
                _ => "运行状态待确认",
            };
            self.detail = format!(
                "{} · {} · {}",
                issue.category.label(),
                issue.readiness.label(),
                issue.detail
            );
        }
        self
    }
}

pub(super) fn summarize_system_health(snapshot: Option<&SystemHealth>) -> SystemHealthSummary {
    let Some(snapshot) = snapshot else {
        return SystemHealthSummary {
            state: "unknown",
            label: "运行状态待确认",
            detail: "等待后端运行状态样本".to_owned(),
        };
    };

    let mut summary =
        summarize_health_fields(snapshot.risk, snapshot.degraded, snapshot.problems.len());
    if !snapshot.problems.is_empty() {
        summary.detail = problem_breakdown(&snapshot.problems).summary_label();
    }
    summary
}

pub(super) fn summarize_system_state(state: &LoadState<SystemHealth>) -> SystemHealthSummary {
    let mut summary = summarize_system_health(state.value());
    let Some(problem) = state.problem() else {
        return summary;
    };
    let detail = if state.value().is_some() {
        "风险与资金数值未确认，仅供参考"
    } else {
        "尚无可用的风险与资金快照"
    };
    // Retained risk warnings still matter; a failed refresh cannot clear them.
    if matches!(summary.state, "blocked" | "warning") {
        summary.detail = format!("{} · {detail} · {}", summary.detail, problem.message);
    } else {
        summary.state = "degraded";
        summary.label = if state.value().is_some() {
            "系统数据待确认"
        } else {
            "系统数据读取失败"
        };
        summary.detail = format!("{detail} · {}", problem.message);
    }
    summary
}

fn summarize_health_fields(
    risk: RiskStatusSlot,
    degraded: bool,
    problem_count: usize,
) -> SystemHealthSummary {
    match risk {
        RiskStatusSlot::Block => SystemHealthSummary {
            state: "blocked",
            label: "风险已阻断",
            detail: problem_detail(problem_count, "高风险动作当前不可提交"),
        },
        RiskStatusSlot::Warn => SystemHealthSummary {
            state: "warning",
            label: "风险警告",
            detail: problem_detail(problem_count, "进入持仓/风控查看约束"),
        },
        RiskStatusSlot::Ok if degraded || problem_count > 0 => SystemHealthSummary {
            state: "degraded",
            label: "运行降级",
            detail: problem_detail(problem_count, "当前快照未附带问题明细"),
        },
        RiskStatusSlot::Ok => SystemHealthSummary {
            state: "healthy",
            label: "运行正常",
            detail: "当前快照未报告运行问题".to_owned(),
        },
    }
}

fn problem_detail(problem_count: usize, empty: &'static str) -> String {
    if problem_count == 0 {
        empty.to_owned()
    } else {
        format!("{problem_count} 项运行数据依据需查看")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_keeps_unknown_distinct_from_healthy() {
        let summary = summarize_system_health(None);

        assert_eq!(summary.state, "unknown");
        assert_eq!(summary.label, "运行状态待确认");
    }

    #[test]
    fn summary_uses_degraded_flag_even_without_problem_rows() {
        let summary = summarize_health_fields(RiskStatusSlot::Ok, true, 0);

        assert_eq!(summary.state, "degraded");
        assert_eq!(summary.detail, "当前快照未附带问题明细");
    }

    #[test]
    fn summary_prioritizes_risk_over_generic_degradation() {
        let warning = summarize_health_fields(RiskStatusSlot::Warn, true, 3);
        let blocked = summarize_health_fields(RiskStatusSlot::Block, true, 4);

        assert_eq!(warning.state, "warning");
        assert_eq!(warning.label, "风险警告");
        assert_eq!(warning.detail, "3 项运行数据依据需查看");
        assert_eq!(blocked.state, "blocked");
        assert_eq!(blocked.label, "风险已阻断");
        assert_eq!(blocked.detail, "4 项运行数据依据需查看");
    }

    #[test]
    fn summary_reports_clean_snapshot_without_overclaiming_connectivity() {
        let summary = summarize_health_fields(RiskStatusSlot::Ok, false, 0);

        assert_eq!(summary.state, "healthy");
        assert_eq!(summary.label, "运行正常");
        assert_eq!(summary.detail, "当前快照未报告运行问题");
    }
}
