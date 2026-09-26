//! 动作账本的状态/类别/问题/结果纯文案派生（含平仓 run 结果状态收敛）。
//! 表格与详情渲染见 `rows.rs`，标签页装配见父模块 `action_runs.rs`。

use shared_types::{
    ActionMutationChange, ActionMutationDiff, ActionRun, ActionRunKind, ActionRunStatus,
    ApiProblem, CloseRunStatus,
};

#[path = "hedge_confirm.rs"]
mod hedge_confirm;

use hedge_confirm::hedge_confirm_payload_label;
pub(super) use hedge_confirm::hedge_confirm_result_summary;

pub(super) fn kind_label(kind: ActionRunKind) -> &'static str {
    match kind {
        ActionRunKind::TradingRiskConfigUpdate => "风控参数",
        ActionRunKind::TradingAdapterSelect => "执行模式",
        ActionRunKind::TradingKillSwitch => "Kill Switch",
        ActionRunKind::TradingFeeSnapshotUpsert => "费率快照",
        ActionRunKind::TradingOrderSubmit => "提交订单",
        ActionRunKind::TradingOrderCancel => "撤销订单",
        ActionRunKind::TradingOrderReconcile => "订单对账",
        ActionRunKind::AutomationConfigUpdate => "自动套利配置",
        ActionRunKind::AutomationControl => "自动套利控制",
        ActionRunKind::AutomationLiveUnlock => "实盘自动化解锁",
        ActionRunKind::HedgeConfirm => "提交对冲",
        ActionRunKind::WebhookConfigUpdate => "Webhook 配置",
        ActionRunKind::WebhookTest => "测试消息入队",
        ActionRunKind::MarketSubscriptionsUpdate => "行情订阅配置",
        ActionRunKind::GateCrossExModeUpdate => "Gate CrossEx 模式配置",
        ActionRunKind::StockBatchUpdate => "股票批量监控配置",
        ActionRunKind::StockMonitorUpdate => "股票单股监控配置",
        ActionRunKind::StockPlanBuild => "股票计划构建与预留",
        ActionRunKind::StockPeerPlanBuild => "股票跨所计划构建与预留",
        ActionRunKind::OnchainComparisonConfigUpdate => "链上监控配置",
        ActionRunKind::OnchainBatchAdd => "加入链上批量监控",
        ActionRunKind::OnchainBatchRemove => "移除链上批量市场",
        ActionRunKind::VenueCredentialsUpdate => "凭证更新",
        ActionRunKind::VenueCredentialsClear => "清空凭证",
        ActionRunKind::VenueCredentialsMigrate => "迁移凭证",
        ActionRunKind::OnchainProviderCredentialsUpdate => "链上 报价服务 凭证更新",
        ActionRunKind::OnchainProviderCredentialsClear => "链上 报价服务 凭证清除",
        ActionRunKind::PortfolioClosePosition => "关闭仓位",
        ActionRunKind::PortfolioClosePair => "关闭交易对",
        ActionRunKind::PortfolioCloseAll => "全部平仓",
        ActionRunKind::PortfolioCloseCompensation => "平仓补偿",
        ActionRunKind::PortfolioCloseManualTerminal => "平仓人工终结",
    }
}

fn status_label(status: ActionRunStatus) -> &'static str {
    match status {
        ActionRunStatus::Accepted => "处理中",
        ActionRunStatus::Succeeded => "成功",
        ActionRunStatus::Failed => "失败",
    }
}

pub(super) fn status_label_for_run(run: &ActionRun) -> String {
    if run.kind == ActionRunKind::HedgeConfirm {
        if let Some(result) = run.result.as_ref() {
            return hedge_confirm_payload_label(result).to_owned();
        }
        if run.status == ActionRunStatus::Succeeded {
            return "提交结果缺失".to_owned();
        }
    }
    if is_portfolio_close_kind(run.kind) {
        if let Some(result) = run.result.as_ref() {
            let status = close_run_payload_status(result);
            return close_run_status_label(status).to_owned();
        }
        if run.status == ActionRunStatus::Succeeded {
            return "结果状态缺失".to_owned();
        }
    }
    if run.status == ActionRunStatus::Succeeded
        && matches!(
            run.kind,
            ActionRunKind::TradingOrderSubmit | ActionRunKind::TradingOrderCancel
        )
    {
        return "请求成功，最终结果见订单".to_owned();
    }
    status_label(run.status).to_owned()
}

fn is_portfolio_close_kind(kind: ActionRunKind) -> bool {
    matches!(
        kind,
        ActionRunKind::PortfolioClosePosition
            | ActionRunKind::PortfolioClosePair
            | ActionRunKind::PortfolioCloseAll
            | ActionRunKind::PortfolioCloseCompensation
            | ActionRunKind::PortfolioCloseManualTerminal
    )
}

#[derive(Clone, Copy)]
enum CloseRunPayloadStatus {
    Known(CloseRunStatus),
    Missing,
    Invalid,
}

fn close_run_payload_status(result: &serde_json::Value) -> CloseRunPayloadStatus {
    let Some(status) = result.get("status") else {
        return CloseRunPayloadStatus::Missing;
    };
    match serde_json::from_value::<CloseRunStatus>(status.clone()) {
        Ok(status) => CloseRunPayloadStatus::Known(status),
        Err(_) => CloseRunPayloadStatus::Invalid,
    }
}

fn close_run_status_label(status: CloseRunPayloadStatus) -> &'static str {
    match status {
        CloseRunPayloadStatus::Known(CloseRunStatus::Submitted) => "已提交",
        CloseRunPayloadStatus::Known(CloseRunStatus::Succeeded) => "已完成",
        CloseRunPayloadStatus::Known(CloseRunStatus::PartiallySubmitted) => "部分提交",
        CloseRunPayloadStatus::Known(CloseRunStatus::UnwindRequired) => "需补偿",
        CloseRunPayloadStatus::Known(CloseRunStatus::CompensationSubmitted) => "补偿中",
        CloseRunPayloadStatus::Known(CloseRunStatus::Compensated) => "已补偿",
        CloseRunPayloadStatus::Known(CloseRunStatus::CompensationFailed) => "补偿失败",
        CloseRunPayloadStatus::Known(CloseRunStatus::ManuallyResolved) => "已人工终结",
        CloseRunPayloadStatus::Known(CloseRunStatus::Failed) => "失败",
        CloseRunPayloadStatus::Missing => "结果状态缺失",
        CloseRunPayloadStatus::Invalid => "结果状态异常",
    }
}

pub(super) fn status_class(status: ActionRunStatus) -> &'static str {
    match status {
        ActionRunStatus::Accepted => "status-pill pending",
        ActionRunStatus::Succeeded => "status-pill ready",
        ActionRunStatus::Failed => "status-pill blocked",
    }
}

pub(super) fn optional_text(value: Option<String>) -> String {
    value
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "-".into())
}

pub(super) fn status_class_for_run(run: &ActionRun) -> &'static str {
    if is_portfolio_close_kind(run.kind) {
        return match run.result.as_ref().map(close_run_payload_status) {
            Some(CloseRunPayloadStatus::Known(CloseRunStatus::Succeeded)) => "status-pill ready",
            Some(CloseRunPayloadStatus::Known(
                CloseRunStatus::Failed
                | CloseRunStatus::CompensationFailed
                | CloseRunStatus::UnwindRequired,
            )) => "status-pill blocked",
            _ if run.status == ActionRunStatus::Failed => "status-pill blocked",
            _ => "status-pill pending",
        };
    }
    if matches!(
        run.kind,
        ActionRunKind::HedgeConfirm
            | ActionRunKind::TradingOrderSubmit
            | ActionRunKind::TradingOrderCancel
    ) {
        return if run.status == ActionRunStatus::Failed {
            "status-pill blocked"
        } else {
            "status-pill pending"
        };
    }
    status_class(run.status)
}

pub(super) fn problem_summary(problem: Option<ApiProblem>) -> String {
    problem_parts(problem, false).map_or_else(|| "-".into(), |parts| parts.join(" · "))
}

pub(super) fn problem_detail(problem: Option<ApiProblem>) -> Option<String> {
    problem_parts(problem, true).map(|parts| parts.join(" · "))
}

fn problem_parts(problem: Option<ApiProblem>, verbose: bool) -> Option<Vec<String>> {
    let problem = problem?;
    let mut parts = if verbose {
        vec![format!("{}：{}", problem.code, problem.message)]
    } else {
        vec![problem.message]
    };
    if verbose {
        if let Some(status) = problem.status {
            parts.push(format!("HTTP {status}"));
        }
        if let Some(source) = problem.source {
            parts.push(format!("source {source}"));
        }
    }
    if let Some(request_id) = problem.request_id {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    Some(parts)
}

pub(super) fn result_json(result: Option<serde_json::Value>) -> String {
    result.map_or_else(
        || "-".into(),
        |value| {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "result_encode_failed".into())
        },
    )
}

pub(super) fn mutation_detail(mutation: Option<ActionMutationDiff>) -> Option<String> {
    let mutation = mutation?;
    let changes = mutation
        .changes
        .iter()
        .map(mutation_change_label)
        .collect::<Vec<_>>()
        .join("；");
    Some(format!(
        "生效时间 {} · {changes}",
        time_label(mutation.effective_at_ms)
    ))
}

fn mutation_change_label(change: &ActionMutationChange) -> String {
    match change {
        ActionMutationChange::Adapter { before, after } => {
            format!("adapter {before} -> {after}")
        }
        ActionMutationChange::ExecutionEnvironment { before, after } => {
            format!("执行环境 {before} -> {after}")
        }
        ActionMutationChange::LiveTradingEnabled { before, after } => {
            format!("实盘开关 {before} -> {after}")
        }
        ActionMutationChange::KillSwitchActive { before, after } => {
            format!("Kill Switch {before} -> {after}")
        }
        ActionMutationChange::MaxOrderNotional { before, after } => {
            format!("单笔上限 {before} -> {after}")
        }
        ActionMutationChange::MaxOpenOrders { before, after } => {
            format!("最大挂单 {before} -> {after}")
        }
        ActionMutationChange::MaxHedgeImbalancePct { before, after } => {
            format!("对冲偏差 {before} -> {after}")
        }
        ActionMutationChange::LiquidationWarnPct { before, after } => {
            format!("强平预警 {before} -> {after}")
        }
        ActionMutationChange::LiquidationDangerPct { before, after } => {
            format!("强平危险 {before} -> {after}")
        }
        ActionMutationChange::AllowedExchanges { before, after } => {
            format!("允许交易所 {} -> {}", before.join(","), after.join(","))
        }
        ActionMutationChange::AllowedSymbols { before, after } => {
            format!("允许标的 {} -> {}", before.join(","), after.join(","))
        }
        ActionMutationChange::ProtectedPositions { before, after } => format!(
            "受保护持仓 {} -> {}",
            protected_positions_label(before),
            protected_positions_label(after)
        ),
        ActionMutationChange::AutoProfitClose { before, after } => format!(
            "自动双边退出 {} -> {}",
            auto_profit_close_label(before),
            auto_profit_close_label(after)
        ),
    }
}

fn protected_positions_label(values: &[shared_types::ProtectedPositionFingerprint]) -> String {
    if values.is_empty() {
        return "无".to_owned();
    }
    values
        .iter()
        .map(|value| {
            format!(
                "{}:{}:{} {}@{} #{}",
                value.venue,
                value.native_symbol,
                value.side,
                value.quantity,
                value.entry_price,
                value.opening_identity
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn auto_profit_close_label(config: &shared_types::AutoProfitCloseConfig) -> String {
    format!(
        "止盈{} ${:.2}/{:.4}% / 止损{} ${:.2}/{:.4}% / 强平{} {:.2}% / 连续 {} 次 / 冷却 {} 秒",
        switch_label(config.enabled),
        config.min_net_profit_usd,
        config.min_roi_bps / 100.0,
        switch_label(config.stop_loss_enabled),
        config.max_net_loss_usd,
        config.max_loss_roi_bps / 100.0,
        switch_label(config.liquidation_guard_enabled),
        config.liquidation_exit_distance_pct,
        config.confirmation_samples,
        config.cooldown_secs
    )
}

const fn switch_label(enabled: bool) -> &'static str {
    if enabled {
        "开"
    } else {
        "关"
    }
}

pub(super) fn time_label(ms: i64) -> String {
    if ms <= 0 {
        return "--:--".into();
    }
    crate::panels::modules::timestamp::local_date_hm(ms).unwrap_or_else(|| "--:--".into())
}

#[cfg(test)]
#[path = "labels_tests.rs"]
mod tests;
