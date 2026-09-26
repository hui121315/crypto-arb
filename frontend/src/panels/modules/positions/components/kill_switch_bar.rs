use leptos::prelude::*;
use shared_types::{
    KillSwitchRequest, PositionOrigin, PositionRow, PositionSide,
    CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE,
};

use super::super::data::{CloseAllPositionsAction, CloseExecutionGate, PositionsKillSwitchAction};
use super::section_state::SectionData;
use crate::panels::shared::KILL_SWITCH_POLICY_LABEL;
use crate::panels::shared::operation_journal::settings_recovery_panel;
use crate::state::action_state::ActionState;
use crate::state::{load_state::LoadState, trading_status::TradingStatusState};

pub(in crate::panels::modules::positions) fn kill_switch_bar(
    position_count: Memo<usize>,
    rows: Memo<SectionData<Vec<PositionRow>>>,
    kill_switch_action: PositionsKillSwitchAction,
    close_all_action: CloseAllPositionsAction,
    visible: Memo<bool>,
) -> impl IntoView {
    let phrase = RwSignal::new(String::new());
    let status = expect_context::<TradingStatusState>().state;
    let execution_gate = Memo::new(move |_| status.with(CloseExecutionGate::from_status));
    let requires_live = Memo::new(move |_| {
        rows.with(|section| section.value.iter().any(|row| row.origin == PositionOrigin::AccountPrivate))
    });
    let close_blocked_label = Memo::new(move |_| {
        if !rows.get().has_fresh_value() {
            Some("持仓待确认")
        } else {
            execution_gate.get().blocked_label(requires_live.get())
        }
    });
    Effect::new(move |_| {
        execution_gate.track();
        phrase.set(String::new());
    });
    let kill_active = Memo::new(move |_| {
        status.get()
            .value()
            .map(|value| value.risk.kill_switch_active)
            .unwrap_or(false)
    });
    let kill_status_known = Memo::new(move |_| matches!(status.get(), LoadState::Ready(_)));
    let kill_busy = Memo::new(move |_| kill_switch_action.journal.busy.get());
    let kill_locked = Memo::new(move |_| kill_switch_action.journal.locked());
    let close_all_busy = Memo::new(move |_| close_all_action.recovery.journal.locked());
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
        if kill_busy.get_untracked() || kill_locked.get_untracked() {
            return;
        }
        let LoadState::Ready(current) = status.get_untracked() else {
            return;
        };
        kill_switch_action.submit.run(kill_switch_request(
            current.risk.kill_switch_active,
            current.open_order_count,
            "positions",
        ));
    };

    let close_all = move || {
        let confirmation = phrase.get_untracked();
        if close_all_busy.get_untracked()
            || close_blocked_label.get_untracked().is_some()
            || position_count.get_untracked() == 0
            || confirmation.trim() != CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE
        {
            return;
        }
        close_all_action.submit.run(confirmation.trim().to_owned());
    };

    view! {
        <div class="kill-switch-bar">
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
                        {move || {
                            let section = rows.get();
                            if section.has_fresh_value() {
                                format!("当前 {} 个持仓", section.value.len())
                            } else if !section.value.is_empty() {
                                format!("上次 {} 个持仓 · 待刷新", section.value.len())
                            } else {
                                "持仓待确认".to_owned()
                            }
                        }}
                    </p>
                    <p class="positions-control-description">
                        {move || kill_switch_effect_label(kill_active.get(), kill_status_known.get())}
                    </p>
                </div>
                <button
                    class="row-action positions-control-primary-action"
                    disabled=move || kill_busy.get() || kill_locked.get() || !kill_status_known.get()
                    on:click=toggle_kill
                >
                    {move || kill_switch_button_label(
                        kill_busy.get(),
                        kill_active.get(),
                        kill_status_known.get(),
                    )}
                </button>
                <em class="positions-action-message" aria-live="polite">
                    {move || match kill_switch_action.state.get() {
                        ActionState::Idle => "总闸以当前后台状态为准",
                        ActionState::Pending { .. } => "正在提交总闸操作",
                        ActionState::Accepted { .. } => "原操作已受理，结果尚待确认",
                        ActionState::Succeeded { .. } => "最近操作已完成，当前开关以上方状态为准",
                        ActionState::Failed { .. } if kill_locked.get() => "原操作结果未确认，请核对原处理结果",
                        ActionState::Failed { .. } => "最近操作未完成，请查看详情",
                    }}
                </em>
                {settings_recovery_panel(kill_switch_action.journal, kill_switch_action.recheck)}
                <Show when=move || !matches!(kill_switch_action.state.get(), ActionState::Idle)>
                    <details class="positions-action-evidence">
                        <summary>"最近操作详情"</summary>
                        <span>{move || risk_action_message(&kill_switch_action.state.get(), "")}</span>
                    </details>
                </Show>
            </section>
            <Show when=move || visible.get()>
            <form
                class="positions-control-card close-all-control"
                on:submit=move |event| {
                    event.prevent_default();
                    close_all();
                }
            >
                <header class="positions-control-card-header">
                    <span>"全部平仓"</span>
                    <strong class="positions-control-danger-state">{move || match execution_gate.get() {
                        CloseExecutionGate::Unknown => "环境待确认".to_owned(),
                        gate => format!("{}平仓", gate.environment_label()),
                    }}</strong>
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
                        prop:value=move || phrase.get()
                        on:input=move |ev| phrase.set(event_target_value(&ev))
                    />
                    <button
                        type="submit"
                        class="danger-action"
                        disabled=move || close_all_busy.get()
                            || close_blocked_label.get().is_some()
                            || position_count.get() == 0
                            || !close_all_confirmed.get()
                    >
                        {move || {
                            if !close_all_action.state.get().is_pending() {
                                if let Some(label) = close_blocked_label.get() {
                                    return label.to_owned();
                                }
                            }
                            close_all_button_label(
                                close_all_action.state.get().is_pending(),
                                position_count.get(),
                                close_all_confirmed.get(),
                            )
                        }}
                    </button>
                </div>
                <em class="positions-action-message" aria-live="polite">
                    {move || {
                        let idle = if !rows.get().has_fresh_value() {
                            "持仓数据未就绪或已过期，等待刷新后再确认全部平仓"
                        } else {
                            execution_gate.get().blocked_reason(requires_live.get())
                                .unwrap_or("reduce-only 市价平仓 · 以交易所最终结果为准")
                        };
                        risk_action_message(&close_all_action.state.get(), idle)
                    }}
                </em>
            </form>
            </Show>
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
        "后台总闸状态未确认，控制保持不可用"
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
    current: bool,
    open_order_count: usize,
    source: &'static str,
) -> KillSwitchRequest {
    KillSwitchRequest {
        active: !current,
        expected_active: Some(current),
        expected_open_order_count: Some(open_order_count),
        reason: kill_switch_reason(source, !current).to_owned(),
    }
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
        let request = kill_switch_request(false, 3, "positions");

        assert!(request.active);
        assert_eq!(request.expected_active, Some(false));
        assert_eq!(request.expected_open_order_count, Some(3));
        assert_eq!(request.reason, "positions.kill_switch.enable");
    }

}
