use crate::panels::modules::timestamp::{local_date_hm, local_hms, now_ms};
use shared_types::{AutomationDecisionKind, AutomationRuntimeState, ExecutionEnvironment};

pub(super) const fn runtime_label(state: AutomationRuntimeState) -> &'static str {
    match state {
        AutomationRuntimeState::Disabled => "已关闭",
        AutomationRuntimeState::Paused => "已暂停",
        AutomationRuntimeState::Watching => "监控候选",
        AutomationRuntimeState::Previewing => "预检中",
        AutomationRuntimeState::Submitting => "提交中",
        AutomationRuntimeState::Hedged => "已对冲",
        AutomationRuntimeState::CoolingDown => "冷却中",
        AutomationRuntimeState::Blocked => "已阻断",
        AutomationRuntimeState::Error => "运行异常",
    }
}

pub(super) const fn runtime_tone(state: AutomationRuntimeState) -> &'static str {
    match state {
        AutomationRuntimeState::Watching
        | AutomationRuntimeState::Previewing
        | AutomationRuntimeState::Submitting
        | AutomationRuntimeState::Hedged => "is-positive",
        AutomationRuntimeState::Paused | AutomationRuntimeState::CoolingDown => "is-warning",
        AutomationRuntimeState::Blocked | AutomationRuntimeState::Error => "is-danger",
        AutomationRuntimeState::Disabled => "is-neutral",
    }
}

pub(super) const fn decision_label(kind: AutomationDecisionKind) -> &'static str {
    match kind {
        AutomationDecisionKind::CandidateSelected => "已选择候选",
        AutomationDecisionKind::OpportunityQualified => "工件已就绪",
        AutomationDecisionKind::NoEligibleCandidate => "无合格候选",
        AutomationDecisionKind::PreviewBlocked => "预检阻断",
        AutomationDecisionKind::Submitted => "已提交",
        AutomationDecisionKind::Replayed => "已重放",
        AutomationDecisionKind::Paused => "已暂停",
        AutomationDecisionKind::EmergencyStopped => "策略已急停",
        AutomationDecisionKind::Failed => "运行失败",
    }
}

pub(super) const fn decision_tone(kind: AutomationDecisionKind) -> &'static str {
    match kind {
        AutomationDecisionKind::CandidateSelected
        | AutomationDecisionKind::OpportunityQualified
        | AutomationDecisionKind::Submitted
        | AutomationDecisionKind::Replayed => "is-positive",
        AutomationDecisionKind::NoEligibleCandidate | AutomationDecisionKind::Paused => {
            "is-neutral"
        }
        AutomationDecisionKind::PreviewBlocked => "is-warning",
        AutomationDecisionKind::EmergencyStopped | AutomationDecisionKind::Failed => "is-danger",
    }
}

pub(super) const fn environment_label(environment: ExecutionEnvironment) -> &'static str {
    match environment {
        ExecutionEnvironment::Paper => "模拟盘",
        ExecutionEnvironment::Live => "实盘",
    }
}

pub(super) fn time_label(ms: i64) -> String {
    local_hms(ms).unwrap_or_else(|| "未知".to_owned())
}

pub(super) fn date_time_label(ms: i64) -> String {
    local_date_hm(ms).unwrap_or_else(|| "日期时间未知".to_owned())
}

pub(super) fn cooldown_label(state: AutomationRuntimeState, until_ms: Option<i64>) -> String {
    if state == AutomationRuntimeState::Disabled {
        return "未运行".to_owned();
    }
    let Some(until_ms) = until_ms else {
        return if state == AutomationRuntimeState::CoolingDown {
            "时间未知"
        } else {
            "待触发"
        }
        .to_owned();
    };
    let remaining_ms = until_ms.saturating_sub(now_ms());
    if remaining_ms <= 0 {
        "已结束".to_owned()
    } else {
        format!("{}s", (remaining_ms + 999) / 1_000)
    }
}

pub(super) fn decision_reason_label(reason: &str) -> String {
    match reason {
        "automation is disabled" => "自动化已关闭".to_owned(),
        "automation is paused" => "自动化已暂停".to_owned(),
        "automation mode does not match the trading runtime environment" => {
            "自动化模式与交易运行环境不一致".to_owned()
        }
        "live automation requires a separate restart-scoped unlock" => {
            "历史阻断：旧版本要求实盘解锁".to_owned()
        }
        "trading kill switch blocks new automated entries" => {
            "交易 Kill Switch 已阻止新的自动入场".to_owned()
        }
        "automatic entry requires take-profit, stop-loss, or liquidation protection" => {
            "请先启用至少一项退出保护：止盈、止损或单腿强平保护".to_owned()
        }
        "automatic take-profit amount exceeds 10% of automation capital" => {
            "止盈金额超过自动化资金的 10%，请应用推荐组合并保存".to_owned()
        }
        "automatic stop-loss amount exceeds automation capital" => {
            "止损金额超过自动化资金，请应用推荐组合并保存".to_owned()
        }
        "automation cooldown is active" => "自动化正在冷却".to_owned(),
        "maximum concurrent automated positions reached" => "已达到自动持仓并发上限".to_owned(),
        "automation paused by operator" => "操作员已暂停自动化".to_owned(),
        "automation emergency stop disabled new entries"
        | "automation emergency stop disabled new entries and cleared live unlock" => {
            "策略急停已关闭新入场".to_owned()
        }
        "automation control updated" => "自动化控制状态已更新".to_owned(),
        "no preview-ready opportunity passed strategy, risk scope, verified positive net edge and freshness gates" => {
            "暂无同时通过策略范围、风控范围、费后正收益与新鲜度门槛的预检候选".to_owned()
        }
        "automatic hedge preview blocked" => "自动对冲预检被阻断".to_owned(),
        "deterministic opportunity artifact is ready for paper execution" => {
            "确定性机会已生成工件，准备进入模拟执行".to_owned()
        }
        "deterministic opportunity is ready; live execution requires manual confirmation" => {
            "历史状态：确定性机会已生成工件，旧版本要求人工确认".to_owned()
        }
        "deterministic opportunity artifact is ready for automatic live execution" => {
            "确定性机会已生成工件，正在自动提交实盘双腿".to_owned()
        }
        "automation state changed after preview; automatic submission suppressed" => {
            "预检后自动化状态已变化，本次自动提交已取消".to_owned()
        }
        "automatic hedge submission failed" => "自动对冲提交失败".to_owned(),
        _ => reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_and_decision_labels_preserve_operator_meaning() {
        assert_eq!(runtime_label(AutomationRuntimeState::Watching), "监控候选");
        assert_eq!(runtime_tone(AutomationRuntimeState::Blocked), "is-danger");
        assert_eq!(
            decision_label(AutomationDecisionKind::PreviewBlocked),
            "预检阻断"
        );
        assert_eq!(
            decision_tone(AutomationDecisionKind::Submitted),
            "is-positive"
        );
    }

    #[test]
    fn environment_and_time_labels_fail_visible() {
        assert_eq!(environment_label(ExecutionEnvironment::Paper), "模拟盘");
        assert_eq!(environment_label(ExecutionEnvironment::Live), "实盘");
        assert_eq!(time_label(i64::MAX), "未知");
        assert_eq!(
            cooldown_label(AutomationRuntimeState::Disabled, None),
            "未运行"
        );
        assert_eq!(
            cooldown_label(AutomationRuntimeState::Watching, None),
            "待触发"
        );
        assert_eq!(
            cooldown_label(AutomationRuntimeState::CoolingDown, None),
            "时间未知"
        );
        assert_eq!(
            decision_reason_label(
                "automatic entry requires take-profit, stop-loss, or liquidation protection"
            ),
            "请先启用至少一项退出保护：止盈、止损或单腿强平保护"
        );
        assert_eq!(
            decision_reason_label("automatic take-profit amount exceeds 10% of automation capital"),
            "止盈金额超过自动化资金的 10%，请应用推荐组合并保存"
        );
        assert_eq!(
            decision_reason_label(
                "no preview-ready opportunity passed strategy, risk scope, verified positive net edge and freshness gates"
            ),
            "暂无同时通过策略范围、风控范围、费后正收益与新鲜度门槛的预检候选"
        );
    }
}
