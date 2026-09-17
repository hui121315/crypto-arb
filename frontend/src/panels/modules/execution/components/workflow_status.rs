use leptos::prelude::*;
use shared_types::{
    ExecutionRunPhase, HedgeLegRole, HedgeTicketLegView, HedgeTicketView, ResourceStatus,
    WorkflowEvidenceHealth,
};

use crate::panels::modules::execution::data::WorkflowViewSource;

pub(in crate::panels::modules::execution) fn workflow_status(
    workflow: RwSignal<Option<HedgeTicketView>>,
    provenance: RwSignal<WorkflowViewSource>,
) -> impl IntoView {
    view! {
        <Show when=move || workflow.get().is_some()>
            <section class="execution-workflow-status" data-testid="hedge-workflow-status">
                <div class="execution-section-head">
                    <div>
                        <span>"票据工作流"</span>
                        <strong>{move || workflow_identity(workflow.get().as_ref())}</strong>
                    </div>
                    <em data-workflow-source=move || provenance.get().label()>
                        {move || provenance.get().label()}
                    </em>
                </div>
                <div class="execution-workflow-legs">
                    <For
                        each=move || workflow_legs(workflow.get().as_ref())
                        key=|leg| leg_key(leg)
                        children=workflow_leg
                    />
                </div>
                <Show when=move || workflow.get().is_some_and(|view| !view.blockers.is_empty())>
                    <p class="execution-workflow-blockers">
                        {move || blocker_text(workflow.get().as_ref())}
                    </p>
                </Show>
            </section>
        </Show>
    }
}

fn workflow_leg(leg: HedgeTicketLegView) -> impl IntoView {
    let role = role_token(leg.role);
    let heading = format!("{} · {}", role_label(leg.role), leg.venue);
    view! {
        <article class="execution-workflow-leg" data-leg-role=role>
            <header>
                <strong>{heading}</strong>
                <span>{leg.symbol}</span>
            </header>
            <div class="execution-workflow-health">
                <HealthCell label="行情" kind="market" health=leg.market/>
                <HealthCell label="费率" kind="fee" health=leg.fee/>
                <HealthCell label="余额" kind="balance" health=leg.balance/>
                <HealthCell label="能力" kind="capability" health=leg.capability/>
            </div>
        </article>
    }
}

#[component]
fn HealthCell(
    label: &'static str,
    kind: &'static str,
    health: WorkflowEvidenceHealth,
) -> impl IntoView {
    let status = status_token(health.status);
    let class = format!("execution-workflow-health-cell {status}");
    let title = health_title(&health);
    view! {
        <span class=class data-health=kind data-health-status=status title=title>
            <b>{label}</b>
            <em>{status_label(health.status)}</em>
        </span>
    }
}

fn workflow_legs(view: Option<&HedgeTicketView>) -> Vec<HedgeTicketLegView> {
    let Some(view) = view else {
        return Vec::new();
    };
    [view.long_leg.clone(), view.short_leg.clone()]
        .into_iter()
        .flatten()
        .collect()
}

fn workflow_identity(view: Option<&HedgeTicketView>) -> String {
    let Some(view) = view else {
        return String::new();
    };
    let ticket = view.ticket().unwrap_or("-");
    match view.execution_run.as_ref() {
        Some(run) => format!(
            "{} · {} · {}",
            ticket,
            run.key.run().unwrap_or("run -"),
            phase_label(run.phase)
        ),
        None => format!("{ticket} · {}", phase_label(ExecutionRunPhase::Preview)),
    }
}

fn blocker_text(view: Option<&HedgeTicketView>) -> String {
    view.map(|view| format!("票据阻断 {} 条", view.blockers.len()))
        .unwrap_or_default()
}

fn health_title(health: &WorkflowEvidenceHealth) -> String {
    let mut parts = vec![format!("状态 {}", status_label(health.status))];
    if let Some(source) = health.source.as_deref() {
        parts.push(format!("source {source}"));
    }
    if let Some(evidence_id) = health.evidence_id.as_deref() {
        parts.push(format!("evidence {evidence_id}"));
    }
    if let Some(request_id) = health.request_id.as_deref() {
        parts.push(format!("request {request_id}"));
    }
    if let Some(problem) = health.problem.as_ref() {
        parts.push(format!("{}: {}", problem.code, problem.message));
    }
    parts.join(" · ")
}

fn leg_key(leg: &HedgeTicketLegView) -> String {
    format!("{}:{}:{}", role_token(leg.role), leg.venue, leg.symbol)
}

fn role_token(role: HedgeLegRole) -> &'static str {
    match role {
        HedgeLegRole::Long => "long",
        HedgeLegRole::Short => "short",
    }
}

fn role_label(role: HedgeLegRole) -> &'static str {
    match role {
        HedgeLegRole::Long => "多腿",
        HedgeLegRole::Short => "空腿",
    }
}

fn status_token(status: ResourceStatus) -> &'static str {
    match status {
        ResourceStatus::Ready => "ready",
        ResourceStatus::Warming => "warming",
        ResourceStatus::Degraded => "degraded",
        ResourceStatus::Partial => "partial",
        ResourceStatus::Error => "error",
    }
}

fn status_label(status: ResourceStatus) -> &'static str {
    match status {
        ResourceStatus::Ready => "就绪",
        ResourceStatus::Warming => "预热",
        ResourceStatus::Degraded => "降级",
        ResourceStatus::Partial => "部分",
        ResourceStatus::Error => "阻断",
    }
}

fn phase_label(phase: ExecutionRunPhase) -> &'static str {
    match phase {
        ExecutionRunPhase::Preview => "预览",
        ExecutionRunPhase::Confirming => "确认中",
        ExecutionRunPhase::Submitting => "提交中",
        ExecutionRunPhase::Working => "工作中",
        ExecutionRunPhase::Closing => "收口中",
        ExecutionRunPhase::Settled => "已结算",
        ExecutionRunPhase::Failed => "失败",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_title_retains_evidence_and_problem_identity() {
        let health = WorkflowEvidenceHealth {
            status: ResourceStatus::Error,
            source: Some("margin_balance".into()),
            evidence_id: Some("balance:okx:42".into()),
            problem: Some(shared_types::ApiProblem::new(
                "MARGIN_BALANCE_MISSING",
                "missing balance",
            )),
            ..WorkflowEvidenceHealth::default()
        };

        let title = health_title(&health);

        assert!(title.contains("balance:okx:42"));
        assert!(title.contains("MARGIN_BALANCE_MISSING"));
    }
}
