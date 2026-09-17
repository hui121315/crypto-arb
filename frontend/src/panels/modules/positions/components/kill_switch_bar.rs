use leptos::prelude::*;
use shared_types::{
    KillSwitchRequest, PositionRow, PositionSide, RiskSnapshot,
    CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE,
};

use super::super::data::{CloseAllPositionsAction, PositionsKillSwitchAction};
use super::section_state::SectionData;
use crate::panels::shared::KILL_SWITCH_POLICY_LABEL;
use crate::state::action_state::ActionState;

pub(in crate::panels::modules::positions) fn kill_switch_bar(
    risk: Memo<SectionData<Option<RiskSnapshot>>>,
    position_count: Memo<usize>,
    rows: Memo<SectionData<Vec<PositionRow>>>,
    kill_switch_action: PositionsKillSwitchAction,
    close_all_action: CloseAllPositionsAction,
    visible: Memo<bool>,
) -> impl IntoView {
    let phrase = RwSignal::new(String::new());
    let kill_active = Memo::new(move |_| {
        risk.get()
            .value
            .map(|value| value.hard_limits.kill_switch_active)
            .unwrap_or(false)
    });
    let kill_status_known = Memo::new(move |_| {
        let section = risk.get();
        section.has_fresh_value() && section.value.is_some()
    });
    let kill_busy = Memo::new(move |_| kill_switch_action.state.get().is_pending());
    let close_all_busy = Memo::new(move |_| close_all_action.state.get().is_pending());
    let close_all_confirmed =
        Memo::new(move |_| phrase.get().trim() == CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE);
    let close_all_scope = Memo::new(move |_| close_all_scope_label(&rows.get().value));
    Effect::new(move |_| {
        if matches!(
            close_all_action.state.get(),
            ActionState::Accepted { .. } | ActionState::Succeeded { .. }
        ) {
            phrase.set(String::new());
        }
    });

    let toggle_kill = move |_| {
        if kill_busy.get_untracked() || !kill_status_known.get_untracked() {
            return;
        }
        let Some(request) = kill_switch_request(risk.get_untracked().value, "positions") else {
            return;
        };
        kill_switch_action.submit.run(request);
    };

    let close_all = move || {
        let confirmation = phrase.get_untracked();
        if close_all_busy.get_untracked()
            || position_count.get_untracked() == 0
            || confirmation.trim() != CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE
        {
            return;
        }
        close_all_action.submit.run(confirmation.trim().to_owned());
    };

    view! {
        <div class="kill-switch-bar" hidden=move || !visible.get()>
            <section
                class=move || if kill_active.get() {
                    "positions-control-card kill-switch-control active"
                } else {
                    "positions-control-card kill-switch-control"
                }
                data-risk-policy="kill-switch"
                title=KILL_SWITCH_POLICY_LABEL
            >
                <header class="positions-control-card-header">
                    <span>"风控总闸"</span>
                    <strong>{move || kill_switch_label(kill_active.get(), kill_status_known.get())}</strong>
                </header>
                <div class="positions-control-card-body">
                    <p class="positions-control-scope">
                        {move || format!("当前 {} 个持仓", position_count.get())}
                    </p>
                    <p class="positions-control-description">
                        {move || kill_switch_effect_label(kill_active.get(), kill_status_known.get())}
                    </p>
                </div>
                <button
                    class="row-action positions-control-primary-action"
                    disabled=move || kill_busy.get() || !kill_status_known.get()
                    on:click=toggle_kill
                >
                    {move || kill_switch_button_label(
                        kill_switch_action.state.get().is_pending(),
                        kill_active.get(),
                        kill_status_known.get(),
                    )}
                </button>
                <em class="positions-action-message" aria-live="polite">
                    {move || risk_action_message(
                        &kill_switch_action.state.get(),
                        "总闸状态与当前风险快照一致",
                    )}
                </em>
            </section>
            <form
                class="positions-control-card close-all-control"
                on:submit=move |event| {
                    event.prevent_default();
                    close_all();
                }
            >
                <header class="positions-control-card-header">
                    <span>"全部平仓"</span>
                    <strong class="positions-control-danger-state">"破坏性动作"</strong>
                </header>
                <label class="close-all-copy" for="positions-close-all-confirmation">
                    <span class="positions-control-scope">{move || close_all_scope.get()}</span>
                    <small id="positions-close-all-help">
                        {format!("精确输入 {CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE}")}
                    </small>
                </label>
                <div class="close-all-fields">
                    <input
                        id="positions-close-all-confirmation"
                        type="text"
                        autocomplete="off"
                        spellcheck="false"
                        aria-describedby="positions-close-all-help"
                        placeholder=CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE
                        value=move || phrase.get()
                        on:input=move |ev| phrase.set(event_target_value(&ev))
                    />
                    <button
                        type="submit"
                        class="danger-action"
                        disabled=move || close_all_busy.get()
                            || position_count.get() == 0
                            || !close_all_confirmed.get()
                    >
                        {move || close_all_button_label(
                            close_all_action.state.get().is_pending(),
                            position_count.get(),
                            close_all_confirmed.get(),
                        )}
                    </button>
                </div>
                <em class="positions-action-message" aria-live="polite">
                    {move || risk_action_message(
                        &close_all_action.state.get(),
                        "reduce-only 市价平仓 · 以交易所终态为准",
                    )}
                </em>
            </form>
        </div>
    }
}

fn kill_switch_label(active: bool, known: bool) -> &'static str {
    if !known {
        "未知"
    } else if active {
        "开启"
    } else {
        "关闭"
    }
}

fn kill_switch_button_label(pending: bool, active: bool, known: bool) -> &'static str {
    if pending {
        "处理中"
    } else if !known {
        "等待快照"
    } else if active {
        "关闭总闸"
    } else {
        "开启总闸"
    }
}

fn kill_switch_effect_label(active: bool, known: bool) -> &'static str {
    if !known {
        "风险快照未确认，控制保持不可用"
    } else if active {
        "已阻止非 reduce-only 新订单；平仓与撤单仍可用"
    } else {
        "新开仓仍可提交；开启后不会自动撤销现有挂单"
    }
}

fn close_all_button_label(pending: bool, position_count: usize, confirmed: bool) -> String {
    if pending {
        "提交中".to_owned()
    } else if position_count == 0 {
        "无持仓".to_owned()
    } else if !confirmed {
        "输入确认短语".to_owned()
    } else {
        format!("关闭全部 {position_count} 个持仓")
    }
}

fn close_all_scope_label(rows: &[PositionRow]) -> String {
    if rows.is_empty() {
        return "当前没有可关闭持仓".to_owned();
    }
    let visible = rows
        .iter()
        .take(3)
        .map(|row| {
            format!(
                "{} {} {}",
                row.venue.to_uppercase(),
                row.symbol,
                position_side_label(row.side)
            )
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let remaining = rows.len().saturating_sub(3);
    let overflow = if remaining == 0 {
        String::new()
    } else {
        format!(" · 另 {remaining} 个")
    };
    format!("将关闭 {} 个持仓：{visible}{overflow}", rows.len())
}

const fn position_side_label(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "多",
        PositionSide::Short => "空",
    }
}

fn risk_action_message(state: &ActionState, idle: &str) -> String {
    state.message(idle)
}

fn kill_switch_request(
    risk: Option<RiskSnapshot>,
    source: &'static str,
) -> Option<KillSwitchRequest> {
    let risk = risk?;
    let current = risk.hard_limits.kill_switch_active;
    Some(KillSwitchRequest {
        active: !current,
        expected_active: Some(current),
        expected_open_order_count: Some(risk.hard_limits.open_orders_used as usize),
        reason: kill_switch_reason(source, !current).to_owned(),
    })
}

fn kill_switch_reason(source: &'static str, active: bool) -> &'static str {
    if active {
        match source {
            "positions" => "positions.kill_switch.enable",
            "settings" => "settings.kill_switch.enable",
            _ => "kill_switch.enable",
        }
    } else {
        match source {
            "positions" => "positions.kill_switch.disable",
            "settings" => "settings.kill_switch.disable",
            _ => "kill_switch.disable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::ApiProblem;

    #[test]
    fn risk_message_preserves_typed_failure() {
        let close_all = ActionState::failed(
            "全部平仓失败",
            ApiProblem::new("BAD_REQUEST", "确认短语错误"),
        );

        let message = risk_action_message(&close_all, "reduce-only 市价平仓");

        assert!(message.contains("全部平仓失败"));
        assert!(message.contains("确认短语错误"));
    }

    #[test]
    fn kill_switch_unknown_state_is_explicit() {
        assert_eq!(kill_switch_label(false, false), "未知");
        assert_eq!(kill_switch_button_label(false, false, false), "等待快照");
    }

    #[test]
    fn kill_switch_request_carries_snapshot_confirmation() {
        let request = kill_switch_request(Some(risk_snapshot(false, 3)), "positions");

        assert!(request.is_some());
        let request = request.unwrap_or_else(|| KillSwitchRequest {
            active: false,
            expected_active: None,
            expected_open_order_count: None,
            reason: String::new(),
        });

        assert!(request.active);
        assert_eq!(request.expected_active, Some(false));
        assert_eq!(request.expected_open_order_count, Some(3));
        assert_eq!(request.reason, "positions.kill_switch.enable");
    }

    fn risk_snapshot(active: bool, open_orders: u32) -> RiskSnapshot {
        RiskSnapshot {
            var_99_1d_usd: 0.0,
            var_pct_of_nav: 0.0,
            var_sample_size: 0,
            funding_clustering: Vec::new(),
            delta_concentration: Vec::new(),
            margin_utilization: Vec::new(),
            hard_limits: shared_types::HardLimitsUsage {
                open_orders_used: open_orders,
                open_orders_max: 10,
                max_symbol_notional_usd: 0.0,
                max_order_notional_usd: 0.0,
                kill_switch_active: active,
            },
            updated_at_ms: 1,
        }
    }
}
