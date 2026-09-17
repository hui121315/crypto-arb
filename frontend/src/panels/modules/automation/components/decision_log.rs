use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{AutomationDecision, AutomationDecisionKind, AutomationRuntimeStatus};

use super::super::format::{date_time_label, decision_label, decision_reason_label, decision_tone};

pub(in crate::panels::modules::automation) fn decision_log(
    state: RwSignal<LoadState<AutomationRuntimeStatus>>,
) -> impl IntoView {
    view! {
        <section class="automation-decision-log">
            <header>
                <div><strong>"决策与生命周期"</strong><span>"只读历史 · 候选、阻断、提交与控制"</span></div>
                <small>{move || decision_count_label(&state.get())}</small>
            </header>
            <div class="automation-log-body">{move || decision_state(&state.get())}</div>
        </section>
    }
}

fn decision_count_label(state: &LoadState<AutomationRuntimeStatus>) -> String {
    state.value().map_or_else(
        || "等待决策流".into(),
        |status| {
            format!(
                "显示 {} 条 · 同类已合并",
                display_decisions(status.recent_decisions.clone()).len()
            )
        },
    )
}

fn decision_state(state: &LoadState<AutomationRuntimeStatus>) -> AnyView {
    let Some(status) = state.value().cloned() else {
        return view! { <p class="workbench-table-empty">"等待自动化决策流…"</p> }.into_any();
    };
    if status.recent_decisions.is_empty() {
        return view! {
            <div class="automation-log-empty"><strong>"暂无决策记录"</strong><span>"策略保持关闭时不会制造候选或提交事件。"</span></div>
        }
        .into_any();
    }
    view! {
        <div class="workbench-table-wrap">
            <table class="workbench-table automation-log-table" data-table-budget="row-cap">
                <thead><tr><th>"时间"</th><th>"结果"</th><th>"标的"</th><th>"原因"</th><th>"工件"</th><th>"ExecutionRun"</th><th>"问题"</th></tr></thead>
                <tbody>
                    {display_decisions(status.recent_decisions).into_iter().map(|decision| {
                        let class = format!("automation-log-kind {}", decision_tone(decision.kind));
                        let result = decision_result_label(&decision);
                        let problem = decision.problem.map_or_else(
                            || "—".to_owned(),
                            |problem| format!("{} · {}", problem.code, problem.message),
                        );
                        let problem_title = problem.clone();
                        let artifact = decision.execution_artifact.as_ref().map_or_else(
                            || "—".to_owned(),
                            |artifact| format!("{} · ${:+.4}", artifact.artifact_id, artifact.expected_net_edge_usd),
                        );
                        let artifact_title = artifact.clone();
                        view! {
                            <tr>
                                <td class="num">{date_time_label(decision.occurred_at_ms)}</td>
                                <td><span class=class>{result}</span></td>
                                <td>{decision.symbol.unwrap_or_else(|| "系统".to_owned())}</td>
                                <td title=decision.reason.clone()>{decision_reason_label(&decision.reason)}</td>
                                <td title=artifact_title>{artifact}</td>
                                <td>{decision.execution_run_id.unwrap_or_else(|| "—".to_owned())}</td>
                                <td title=problem_title>{problem}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
    .into_any()
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
            let duplicate = previous.as_ref().is_some_and(
                |(kind, reason): &(shared_types::AutomationDecisionKind, String)| {
                    *kind == decision.kind && reason == &decision.reason
                },
            );
            if !duplicate {
                previous = Some((decision.kind, decision.reason.clone()));
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
