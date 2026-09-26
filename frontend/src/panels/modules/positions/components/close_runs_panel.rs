//! Stable per-incident details and confirmation drafts.
mod derive;
mod editor;
mod summary;
#[cfg(test)]
mod testing;

use leptos::prelude::*;
use shared_types::CloseRun;

use super::{format::money, section_state::SectionData};
use crate::panels::modules::positions::data::CloseRunCompensationAction;
use derive::{
    close_candidate_evidence, close_candidate_label, close_candidate_title, close_run_cost_detail,
    close_run_cost_label, close_run_cost_title, close_run_next_action_detail,
    close_run_remaining_positions_detail, close_run_row_class, close_run_status_detail,
    close_run_status_title, compensation_candidates,
};

pub(in crate::panels::modules::positions) use derive::close_run_status_label;
pub(in crate::panels::modules::positions) use summary::close_runs_panel;

fn close_run_record(
    record: Memo<Option<CloseRun>>,
    fresh: Memo<bool>,
    action: CloseRunCompensationAction,
) -> impl IntoView {
    view! {
        <details class=move || format!("close-incident {}", record.get().map_or("", |run| close_run_row_class(run.status))) data-run-id=move || record.get().map(|run| run.id)>
            <summary>
                {move || record.get().map(|run| {
                    let markets = run.legs.iter().map(|leg| format!("{} · {}", leg.symbol, leg.venue.to_uppercase())).collect::<Vec<_>>().join(" / ");
                    view! {
                        <span><strong class="warning" title=close_run_status_title(&run)>{close_run_status_label(run.status)}</strong><small>{run.id.clone()}</small></span>
                        <span><strong>{markets}</strong><small>{close_run_status_detail(&run)}</small></span>
                        <span class="num"><strong>{money(run.naked_exposure_usd)}</strong><small>{if run.unwind_plan.as_ref().is_some_and(|plan| !plan.compensation_attempts.is_empty()) { "原事故敞口" } else { "剩余裸露" }}</small></span>
                        <span class="close-incident-open">"处理详情"</span>
                    }
                })}
            </summary>
            <div class="close-incident-detail">
                {move || record.get().map(|run| view! {
                    <div class="close-incident-context">
                        <a class="row-action" href=crate::panels::routing::close_run_review_href(&run.id)>"关联复盘"</a>
                        <p>{close_run_next_action_detail(&run)}</p>
                        <p>{close_run_remaining_positions_detail(&run)}</p>
                        <p title=close_run_cost_title(run.cost_reconciliation.as_ref())>
                            <strong>{close_run_cost_label(run.cost_reconciliation.as_ref())}</strong>
                            " · " {close_run_cost_detail(run.cost_reconciliation.as_ref())}
                        </p>
                    </div>
                    <div class="close-incident-candidates">
                        {compensation_candidates(&run).into_iter().map(|(index, candidate)| view! {
                            <div title=close_candidate_title(&candidate)>
                                <strong>{close_candidate_label(index, &candidate)}</strong>
                                <small>{close_candidate_evidence(&candidate)}</small>
                            </div>
                        }).collect_view()}
                    </div>
                    {run.unwind_plan.as_ref().filter(|plan| !plan.compensation_attempts.is_empty()).map(|plan| view! {
                      <div class="close-incident-candidates" aria-label="补偿订单进度">
                        {plan.compensation_attempts.iter().map(|attempt| {
                            let number = |value: Option<f64>| value.map(super::format::quantity).unwrap_or_else(|| "待确认".to_owned());
                            let state = match attempt.status {
                                shared_types::CloseLegStatus::PartiallyFilled => "部分成交",
                                shared_types::CloseLegStatus::CancelRequested => "撤单待确认",
                                shared_types::CloseLegStatus::Cancelled => "已取消未成交部分",
                                shared_types::CloseLegStatus::Filled => "成交结果",
                                shared_types::CloseLegStatus::Rejected | shared_types::CloseLegStatus::Failed | shared_types::CloseLegStatus::Skipped => "补偿未完成",
                                _ => "等待成交",
                            };
                            view! {
                                <div data-order-id=attempt.order.as_ref().map(|order| order.intent.id.clone())>
                                    <strong>{format!("{} · {} · {state}", attempt.venue.to_uppercase(), attempt.symbol)}</strong>
                                    <small>{format!("已成交 {} / 目标 {} · 未完成 {}", number(attempt.confirmed_filled_quantity()), number(Some(attempt.target_quantity)), number(attempt.unfilled_quantity()))}</small>
                                </div>
                            }
                        }).collect_view()}
                      </div>
                    })}
                })}
                {editor::incident_editor(record, fresh, action)}
                <p class="positions-action-message" role="status">{move || {
                    let state = action.state.get();
                    if record.get().is_some_and(|run| state.evidence().and_then(|e| e.run_id.as_deref()) == Some(run.id.as_str())) {
                        state.label().unwrap_or("").to_owned()
                    } else { String::new() }
                }}</p>
                <p class="positions-action-message" role="status">{move || {
                    let state = action.cancel_state.get();
                    if record.get().is_some_and(|run| run.unwind_plan.as_ref().is_some_and(|plan|
                        plan.compensation_attempts.iter().filter_map(|attempt| attempt.order.as_ref())
                            .any(|order| state.evidence().is_some_and(|e| e.order_ids.contains(&order.intent.id))))) {
                        state.label().unwrap_or("").to_owned()
                    } else { String::new() }
                }}</p>
            </div>
        </details>
    }
}
