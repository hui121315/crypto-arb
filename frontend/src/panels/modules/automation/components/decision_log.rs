use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{AutomationDecision, AutomationDecisionKind, AutomationRuntimeStatus};

use super::super::format::{date_time_label, decision_label, decision_reason_label, decision_tone};

pub(in crate::panels::modules::automation) fn decision_log(
    state: RwSignal<LoadState<AutomationRuntimeStatus>>,
    inspect_run: Callback<String>,
) -> impl IntoView {
    let rows = Memo::new(move |_| {
        state.with(|state| {
            state.value().map_or_else(Vec::new, |status| {
                display_decisions(status.recent_decisions.clone())
            })
        })
    });
    view! {
        <section class="automation-decision-log">
            <header>
                <div><strong>"决策与生命周期"</strong><span>"只读历史 · 候选、阻断、提交与控制"</span></div>
                <small>{move || decision_count_label(&state.get())}</small>
            </header>
            <div class="automation-log-body">
                <Show when=move || !rows.with(Vec::is_empty) fallback=move || view! {
                    <p class="workbench-table-empty">{move || state.with(|state| match state {
                        LoadState::Loading => "正在读取决策记录", LoadState::Error(_) => "决策记录读取失败",
                        _ => "暂无决策记录",
                    })}</p>
                }>
                    <div class="workbench-table-wrap"><table class="workbench-table automation-log-table" aria-label="自动化决策历史">
                        <thead><tr><th>"时间"</th><th>"结果 / 标的"</th><th>"原因 / 详情"</th></tr></thead>
                        <tbody><For each=move || rows.get() key=|decision| decision.id.clone() children=move |initial| {
                            let id = initial.id.clone();
                            let decision = Memo::new(move |_| rows.with(|rows| rows.iter().find(|row| row.id == id).cloned()).unwrap_or_else(|| initial.clone()));
                            decision_row(decision, inspect_run)
                        } /></tbody>
                    </table></div>
                </Show>
            </div>
        </section>
    }
}

fn decision_count_label(state: &LoadState<AutomationRuntimeStatus>) -> String {
    state.value().map_or_else(
        || "等待决策流".into(),
        |status| {
            format!(
                "显示 {} 条 · 重复系统心跳已合并",
                display_decisions(status.recent_decisions.clone()).len()
            )
        },
    )
}

fn decision_row(
    decision: Memo<AutomationDecision>,
    inspect_run: Callback<String>,
) -> impl IntoView {
    view! {
        <tr data-decision-id=move || decision.with(|decision| decision.id.clone())>
            <td class="num">{move || decision.with(|decision| date_time_label(decision.occurred_at_ms))}</td>
            <td><span class=move || decision.with(|decision| format!("automation-log-kind {}", decision_tone(decision.kind)))>
                {move || decision.with(decision_result_label)}</span><strong>{move || decision.with(|decision| decision.symbol.clone().unwrap_or_else(|| "系统".into()))}</strong></td>
            <td><div>{move || decision.with(|decision| decision_reason_label(&decision.reason))}</div>
                <details><summary>"查看记录"</summary><dl>
                <div><dt>"运行编号"</dt><dd>{move || decision.with(|decision| decision.execution_run_id.clone().unwrap_or_else(|| "尚无运行单".into()))}</dd></div>
                <div><dt>"工件"</dt><dd>{move || decision.with(|decision| decision.execution_artifact.as_ref().map_or_else(|| "尚无工件".into(), |artifact| artifact.artifact_id.clone()))}</dd></div>
                <div><dt>"技术原因"</dt><dd>{move || decision.with(|decision| decision.reason.clone())}</dd></div>
                {move || decision.with(|decision| decision.problem.clone()).map(|problem| view! { <div><dt>"问题"</dt><dd>{format!("{} · {}", problem.code, problem.message)}</dd></div> })}
            </dl></details>
            <Show when=move || decision.with(|decision| decision.execution_run_id.is_some())>
                <button type="button" class="row-action" on:click=move |_| {
                    if let Some(id) = decision.with(|decision| decision.execution_run_id.clone()) { inspect_run.run(id); }
                }>"查看回执"</button>
            </Show></td>
        </tr>
    }
}

fn decision_result_label(decision: &AutomationDecision) -> &'static str {
    if decision.kind == AutomationDecisionKind::NoEligibleCandidate
        && decision.reason == "automation is disabled"
    {
        "已关闭"
    } else {
        decision_label(decision.kind)
    }
}

fn display_decisions(decisions: Vec<AutomationDecision>) -> Vec<AutomationDecision> {
    let mut previous = None;
    decisions
        .into_iter()
        .filter(|decision| {
            let duplicate = previous
                .as_ref()
                .is_some_and(|previous: &AutomationDecision| {
                    decision.kind == AutomationDecisionKind::NoEligibleCandidate
                        && previous.kind == decision.kind
                        && previous.reason == decision.reason
                        && decision.symbol.is_none()
                        && previous.symbol.is_none()
                        && decision.opportunity_id.is_none()
                        && previous.opportunity_id.is_none()
                        && decision.execution_run_id.is_none()
                        && previous.execution_run_id.is_none()
                        && decision.execution_artifact.is_none()
                        && previous.execution_artifact.is_none()
                        && decision.problem == previous.problem
                });
            if !duplicate {
                previous = Some(decision.clone());
            }
            !duplicate
        })
        .take(50)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_duplicate_heartbeats_collapse_without_dropping_later_events() {
        let rows = display_decisions(vec![
            decision(AutomationDecisionKind::NoEligibleCandidate, "disabled", 3),
            decision(AutomationDecisionKind::NoEligibleCandidate, "disabled", 2),
            decision(AutomationDecisionKind::Paused, "paused", 1),
        ]);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].occurred_at_ms, 3);
        assert_eq!(rows[1].kind, AutomationDecisionKind::Paused);
    }

    #[test]
    fn disabled_lifecycle_event_is_not_presented_as_candidate_failure() {
        let row = decision(
            AutomationDecisionKind::NoEligibleCandidate,
            "automation is disabled",
            1,
        );

        assert_eq!(decision_result_label(&row), "已关闭");
    }

    #[test]
    fn same_reason_does_not_hide_distinct_orders_or_symbols() {
        let mut first = decision(AutomationDecisionKind::Submitted, "submitted", 3);
        first.execution_run_id = Some("run-one".into());
        let mut second = decision(AutomationDecisionKind::Submitted, "submitted", 2);
        second.execution_run_id = Some("run-two".into());
        assert_eq!(display_decisions(vec![first, second]).len(), 2);
        let mut first = decision(
            AutomationDecisionKind::NoEligibleCandidate,
            "insufficient balance",
            3,
        );
        first.symbol = Some("SOL".into());
        let mut second = first.clone();
        second.symbol = Some("ETH".into());
        assert_eq!(display_decisions(vec![first, second]).len(), 2);
    }

    fn decision(
        kind: AutomationDecisionKind,
        reason: &str,
        occurred_at_ms: i64,
    ) -> AutomationDecision {
        AutomationDecision {
            id: format!("decision-{occurred_at_ms}"),
            kind,
            opportunity_id: None,
            symbol: None,
            reason: reason.to_owned(),
            execution_run_id: None,
            problem: None,
            execution_artifact: None,
            occurred_at_ms,
        }
    }
}
