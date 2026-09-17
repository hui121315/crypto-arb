//! 风控配置页的状态条视图、输入解析/校验与请求构造纯派生。
//! 顶层 `RiskConfigTab` 组件见父模块 `risk_config.rs`。

use leptos::prelude::*;
use shared_types::{
    ApiProblem, AutoProfitCloseConfigPatch, ExecutionEnvironment, KillSwitchRequest,
    RiskConfigPatch,
};

use crate::api::rest::TradingStatusResponse;
use crate::panels::shared::{execution_environment_label, KILL_SWITCH_POLICY_LABEL};
use crate::state::action_state::ActionState;
use crate::state::load_state::LoadState;

use super::super::super::data::{settings_value, SettingsResource};
use super::super::{action_message, problem_cell, problem_message};

pub(super) fn status_strip(state: LoadState<TradingStatusResponse>) -> AnyView {
    let (status, stale_problem) = match state {
        LoadState::Ready(status) => (status, None),
        LoadState::Stale {
            value: status,
            problem,
        } => (status, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取风控状态失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取风控状态"</div> }.into_any();
        }
    };
    let mode = environment_label(status.environment);
    let kill = if status.risk.kill_switch_active {
        "Kill Switch 开启"
    } else {
        "Kill Switch 关闭"
    };
    let live_write = if status.risk.live_trading_enabled {
        "实盘写入启用"
    } else {
        "实盘写入停用"
    };
    let protection = protection_status(&status.risk.auto_profit_close);
    view! {
        <>
            <div class="settings-note-grid">
                <span>{mode}</span>
                <span>{status.adapter}</span>
                <span>{live_write}</span>
                <span>{kill}</span>
                <span>{format!("挂单 {}", status.open_order_count)}</span>
                <span>{format!("强平预警 {:.2}%", status.risk.liquidation_warn_pct)}</span>
                <span>{format!("强平危险 {:.2}%", status.risk.liquidation_danger_pct)}</span>
                <span>{protection}</span>
            </div>
            <em class="settings-message" data-risk-policy="kill-switch">
                {KILL_SWITCH_POLICY_LABEL}
            </em>
            {risk_status_stale_message(stale_problem.as_ref()).map(|message| view! {
                <em class="settings-message is-error">{message}</em>
            })}
        </>
    }
    .into_any()
}

fn protection_status(config: &shared_types::AutoProfitCloseConfig) -> String {
    format!(
        "双边保护 止盈{} · 止损{} · 强平{}({:.2}%)",
        switch_label(config.enabled),
        switch_label(config.stop_loss_enabled),
        switch_label(config.liquidation_guard_enabled),
        config.liquidation_exit_distance_pct
    )
}

const fn switch_label(enabled: bool) -> &'static str {
    if enabled {
        "开"
    } else {
        "关"
    }
}

fn environment_label(environment: ExecutionEnvironment) -> String {
    format!("后端{}环境", execution_environment_label(environment))
}

fn risk_status_stale_message(problem: Option<&ApiProblem>) -> Option<String> {
    problem.map(|problem| problem_message("风控状态刷新失败，显示上次结果", problem))
}

pub(super) fn risk_action_message(
    default: &str,
    save_state: &ActionState,
    kill_state: &ActionState,
) -> String {
    if !matches!(kill_state, ActionState::Idle) {
        return action_message(default, kill_state);
    }
    action_message(default, save_state)
}

pub(super) struct RiskThresholdInputs {
    pub max_order: String,
    pub max_open: String,
    pub imbalance_pct: String,
    pub allowed_exchanges: String,
    pub allowed_symbols: String,
}

pub(super) struct AutoProfitCloseInputs {
    pub enabled: bool,
    pub min_net_profit_usd: String,
    pub min_roi_pct: String,
    pub exit_buffer_pct: String,
    pub stop_loss_enabled: bool,
    pub max_net_loss_usd: String,
    pub max_loss_roi_pct: String,
    pub liquidation_guard_enabled: bool,
    pub liquidation_exit_distance_pct: String,
    pub confirmation_samples: String,
    pub cooldown_secs: String,
}

pub(super) fn risk_patch_from_inputs(
    risk: &RiskThresholdInputs,
    auto_close: &AutoProfitCloseInputs,
) -> Result<RiskConfigPatch, String> {
    let max_order_notional = parse_positive_f64("单笔名义上限", &risk.max_order)?;
    let max_open_orders = parse_positive_usize("最大挂单数", &risk.max_open)?;
    let imbalance = parse_ratio_percent("双腿偏差", &risk.imbalance_pct)?;
    let min_net_profit_usd = parse_positive_f64("最低净利润", &auto_close.min_net_profit_usd)?;
    let min_roi_bps = parse_percent_bps("最低净收益率", &auto_close.min_roi_pct, 0.0001, 100.0)?;
    let exit_buffer_bps =
        parse_percent_bps("平仓安全缓冲", &auto_close.exit_buffer_pct, 0.0, 10.0)?;
    let max_net_loss_usd = parse_positive_f64("最大净亏损", &auto_close.max_net_loss_usd)?;
    let max_loss_roi_bps =
        parse_percent_bps("最大亏损率", &auto_close.max_loss_roi_pct, 0.0001, 100.0)?;
    let liquidation_exit_distance_pct = parse_percentage(
        "强平自动退出距离",
        &auto_close.liquidation_exit_distance_pct,
        0.1,
        100.0,
    )?;
    let confirmation_samples =
        parse_u16_range("连续确认样本", &auto_close.confirmation_samples, 2, 30)?;
    let cooldown_secs = parse_u64_range("触发冷却", &auto_close.cooldown_secs, 10, 3_600)?;
    Ok(RiskConfigPatch {
        max_order_notional: Some(max_order_notional),
        max_open_orders: Some(max_open_orders),
        max_hedge_imbalance_pct: Some(imbalance),
        allowed_exchanges: Some(split_list(&risk.allowed_exchanges)),
        allowed_symbols: Some(split_list(&risk.allowed_symbols)),
        protected_positions: None,
        auto_profit_close: Some(AutoProfitCloseConfigPatch {
            enabled: Some(auto_close.enabled),
            min_net_profit_usd: Some(min_net_profit_usd),
            min_roi_bps: Some(min_roi_bps),
            exit_buffer_bps: Some(exit_buffer_bps),
            stop_loss_enabled: Some(auto_close.stop_loss_enabled),
            max_net_loss_usd: Some(max_net_loss_usd),
            max_loss_roi_bps: Some(max_loss_roi_bps),
            liquidation_guard_enabled: Some(auto_close.liquidation_guard_enabled),
            liquidation_exit_distance_pct: Some(liquidation_exit_distance_pct),
            confirmation_samples: Some(confirmation_samples),
            cooldown_secs: Some(cooldown_secs),
        }),
    })
}

fn parse_positive_f64(label: &str, value: &str) -> Result<f64, String> {
    let parsed = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label}必须是数字"))?;
    if parsed.is_finite() && parsed > 0.0 {
        Ok(parsed)
    } else {
        Err(format!("{label}必须大于 0"))
    }
}

fn parse_positive_usize(label: &str, value: &str) -> Result<usize, String> {
    let parsed = value
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("{label}必须是整数"))?;
    if parsed > 0 {
        Ok(parsed)
    } else {
        Err(format!("{label}必须大于 0"))
    }
}

fn parse_ratio_percent(label: &str, value: &str) -> Result<f64, String> {
    let parsed = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label}必须是数字"))?;
    if parsed <= 100.0 {
        if parsed >= 0.0 {
            Ok(parsed / 100.0)
        } else {
            Err(format!("{label}不能小于 0%"))
        }
    } else {
        Err(format!("{label}不能超过 100%"))
    }
}

fn parse_percent_bps(label: &str, value: &str, min: f64, max: f64) -> Result<f64, String> {
    parse_percentage(label, value, min, max).map(|value| value * 100.0)
}

fn parse_percentage(label: &str, value: &str, min: f64, max: f64) -> Result<f64, String> {
    let parsed = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label}必须是数字"))?;
    if parsed.is_finite() && parsed >= min && parsed <= max {
        Ok(parsed)
    } else {
        Err(format!("{label}必须在 {min}% 到 {max}% 之间"))
    }
}

fn parse_u16_range(label: &str, value: &str, min: u16, max: u16) -> Result<u16, String> {
    let parsed = value
        .trim()
        .parse::<u16>()
        .map_err(|_| format!("{label}必须是整数"))?;
    if (min..=max).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!("{label}必须在 {min} 到 {max} 之间"))
    }
}

fn parse_u64_range(label: &str, value: &str, min: u64, max: u64) -> Result<u64, String> {
    let parsed = value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("{label}必须是整数"))?;
    if (min..=max).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!("{label}必须在 {min} 到 {max} 之间"))
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn status_value(
    resource: SettingsResource<TradingStatusResponse>,
) -> Option<TradingStatusResponse> {
    settings_value(resource)
}

pub(super) fn kill_switch_request(
    status: &TradingStatusResponse,
    active: bool,
) -> KillSwitchRequest {
    KillSwitchRequest {
        active,
        expected_active: Some(status.risk.kill_switch_active),
        expected_open_order_count: Some(status.open_order_count),
        reason: kill_switch_reason(active).to_owned(),
    }
}

fn kill_switch_reason(active: bool) -> &'static str {
    if active {
        "settings.kill_switch.enable"
    } else {
        "settings.kill_switch.disable"
    }
}

#[cfg(test)]
#[path = "form/tests.rs"]
mod tests;
