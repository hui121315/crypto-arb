use shared_types::{RiskStatusSlot, SystemHealth};

use super::problem_ledger::problem_breakdown;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SystemHealthSummary {
    pub state: &'static str,
    pub label: &'static str,
    pub detail: String,
}

pub(super) fn summarize_system_health(snapshot: Option<&SystemHealth>) -> SystemHealthSummary {
    let Some(snapshot) = snapshot else {
        return SystemHealthSummary {
            state: "unknown",
            label: "运行状态待证",
            detail: "等待后端运行态样本".to_owned(),
        };
    };

    let mut summary =
        summarize_health_fields(snapshot.risk, snapshot.degraded, snapshot.problems.len());
    if !snapshot.problems.is_empty() {
        summary.detail = problem_breakdown(&snapshot.problems).summary_label();
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
        format!("{problem_count} 项运行证据需查看")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_keeps_unknown_distinct_from_healthy() {
        let summary = summarize_system_health(None);

        assert_eq!(summary.state, "unknown");
        assert_eq!(summary.label, "运行状态待证");
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
        assert_eq!(warning.detail, "3 项运行证据需查看");
        assert_eq!(blocked.state, "blocked");
        assert_eq!(blocked.label, "风险已阻断");
        assert_eq!(blocked.detail, "4 项运行证据需查看");
    }

    #[test]
    fn summary_reports_clean_snapshot_without_overclaiming_connectivity() {
        let summary = summarize_health_fields(RiskStatusSlot::Ok, false, 0);

        assert_eq!(summary.state, "healthy");
        assert_eq!(summary.label, "运行正常");
        assert_eq!(summary.detail, "当前快照未报告运行问题");
    }
}
