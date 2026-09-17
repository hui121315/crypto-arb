//! 平仓事故（CloseRun）面板组件：待处理事故表、补偿候选与确认提交。
//! 纯派生（筛选/排序/状态与成本文案/可提交判定）见 `derive.rs`，测试见 `testing.rs`。

#[path = "close_runs_panel/derive.rs"]
mod derive;
#[path = "close_runs_panel/summary.rs"]
mod summary;
#[cfg(test)]
#[path = "close_runs_panel/testing.rs"]
mod testing;

use leptos::prelude::*;
use shared_types::CloseRun;

use super::format::money;
use super::section_state::SectionData;
use crate::panels::modules::positions::data::{
    cancel_compensation_key, compensation_key, manual_terminal_key, CloseRunCompensationAction,
    CloseRunCompensationCancelInput, CloseRunCompensationInput, CloseRunManualTerminalInput,
};
use derive::{
    can_submit_compensation, can_submit_manual_terminal, cancellable_compensation_attempts,
    close_candidate_evidence, close_candidate_label, close_candidate_title, close_run_cost_detail,
    close_run_cost_label, close_run_cost_title, close_run_next_action_detail,
    close_run_remaining_positions_detail, close_run_row_class, close_run_status_detail,
    close_run_status_label, close_run_status_title, compensation_button_label,
    compensation_cancel_button_label, compensation_cancel_order_id, compensation_cancel_title,
    compensation_candidates, has_manual_terminal_action,
};
use summary::ManualTerminalDraft;

pub(in crate::panels::modules::positions) use summary::close_runs_panel;

fn render_close_run_body(
    rows: SectionData<Vec<CloseRun>>,
    action: CloseRunCompensationAction,
    phrase: &str,
    manual: &ManualTerminalDraft,
) -> AnyView {
    if rows.value.is_empty() {
        let text = rows
            .status
            .empty_text("暂无待处理平仓事故", "读取中", "读取失败");
        return view! { <tr><td colspan="6" class="empty-cell">{text}</td></tr> }.into_any();
    }
    view! {
        {rows.value
            .into_iter()
            .map(|run| view! {
                <CloseRunRow
                    run=run
                    phrase=phrase.to_owned()
                    manual=manual.clone()
                    action=action
                />
            })
            .collect_view()}
    }
    .into_any()
}

#[component]
fn CloseRunRow(
    run: CloseRun,
    phrase: String,
    manual: ManualTerminalDraft,
    action: CloseRunCompensationAction,
) -> impl IntoView {
    let candidates = compensation_candidates(&run);
    let cancel_attempts = cancellable_compensation_attempts(&run);
    let run_id = run.id.clone();
    let snapshot = run.snapshot_version.clone();
    let can_submit = can_submit_compensation(&run, &phrase);
    let can_manual_terminal = can_submit_manual_terminal(&run, &manual.phrase, &manual.reason);
    let show_manual_terminal = has_manual_terminal_action(&run);
    let cost = run.cost_reconciliation.clone();
    let cost_label = close_run_cost_label(cost.as_ref());
    let cost_detail = close_run_cost_detail(cost.as_ref());
    let cost_title = close_run_cost_title(cost.as_ref());
    let status_label = close_run_status_label(run.status);
    let status_detail = close_run_status_detail(&run);
    let status_title = close_run_status_title(&run);
    let next_action_detail = close_run_next_action_detail(&run);
    let remaining_positions_detail = close_run_remaining_positions_detail(&run);
    view! {
        <tr class=close_run_row_class(run.status)>
            <td title=status_title>
                <strong>{status_label}</strong>
                <small>{status_detail}</small>
                <small>{next_action_detail}</small>
            </td>
            <td>
                <strong>{run_id}</strong>
                <small>{snapshot}</small>
            </td>
            <td class="num">
                <strong>{money(run.naked_exposure_usd)}</strong>
                <small>{remaining_positions_detail}</small>
            </td>
            <td class="close-run-cost" title=cost_title>
                <strong>{cost_label}</strong>
                <small>{cost_detail}</small>
            </td>
            <td>
                <div class="close-run-candidates">
                    {candidates
                        .iter()
                        .map(|(index, candidate)| view! {
                            <span title=close_candidate_title(candidate)>
                                {close_candidate_label(*index, candidate)}
                                <small>{close_candidate_evidence(candidate)}</small>
                            </span>
                        })
                        .collect_view()}
                </div>
            </td>
            <td>
                <div class="close-run-actions">
                    {candidates
                        .into_iter()
                        .map(|(index, candidate)| {
                            let submit_run = run.clone();
                            let submit_phrase = phrase.clone();
                            let key = compensation_key(&submit_run, index);
                            let is_active = move || {
                                action
                                    .active_key
                                    .get()
                                    .as_deref()
                                    .is_some_and(|active| active == key)
                            };
                            view! {
                                <button
                                    class="danger-action"
                                    disabled=move || !can_submit || action.state.get().is_pending()
                                    on:click=move |_| {
                                        action.submit.run(CloseRunCompensationInput {
                                            run: submit_run.clone(),
                                            candidate_index: index,
                                            confirmation_phrase: submit_phrase.clone(),
                                        });
                                    }
                                >
                                    {move || if is_active() {
                                        "提交中"
                                    } else {
                                        compensation_button_label(&candidate)
                                    }}
                                </button>
                            }
                        })
                        .collect_view()}
                    {cancel_attempts
                        .into_iter()
                        .filter_map(|attempt| {
                            let order_id = compensation_cancel_order_id(&attempt)?;
                            let cancel_run = run.clone();
                            let key = cancel_compensation_key(&cancel_run, &order_id);
                            let label = compensation_cancel_button_label(&attempt).to_owned();
                            let title = compensation_cancel_title(&attempt);
                            let is_active = move || {
                                action
                                    .active_key
                                    .get()
                                    .as_deref()
                                    .is_some_and(|active| active == key)
                            };
                            Some(view! {
                                <button
                                    class="danger-action"
                                    title=title
                                    disabled=move || action.state.get().is_pending()
                                    on:click=move |_| {
                                        action.cancel.run(CloseRunCompensationCancelInput {
                                            run: cancel_run.clone(),
                                            order_id: order_id.clone(),
                                        });
                                    }
                                >
                                    {move || if is_active() {
                                        "撤单中".to_owned()
                                    } else {
                                        label.clone()
                                    }}
                                </button>
                            })
                        })
                        .collect_view()}
                    {if show_manual_terminal {
                        let manual_run = run.clone();
                        let manual_key = manual_terminal_key(&manual_run);
                        let manual_input = CloseRunManualTerminalInput {
                            run: manual_run,
                            confirmation_phrase: manual.phrase.clone(),
                            reason: manual.reason.clone(),
                            evidence: manual.evidence.clone(),
                            manual_handling_cost_usd: manual.cost_usd,
                        };
                        let is_active = move || {
                            action
                                .active_key
                                .get()
                                .as_deref()
                                .is_some_and(|active| active == manual_key)
                        };
                        view! {
                            <button
                                class="danger-action"
                                disabled=move || !can_manual_terminal || action.state.get().is_pending()
                                on:click=move |_| action.manual_terminal.run(manual_input.clone())
                            >
                                {move || if is_active() {
                                    "记录中"
                                } else {
                                    "人工终结"
                                }}
                            </button>
                        }
                        .into_any()
                    } else {
                        ().into_any()
                    }}
                </div>
            </td>
        </tr>
    }
}
