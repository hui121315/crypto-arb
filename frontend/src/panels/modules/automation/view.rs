use crate::panels::shared::{webhook_monitor_disclosure, ModuleHeader};
use leptos::prelude::*;
use shared_types::WebhookEventKind;

use super::components::{automation_workspace, control_rail};
use super::data::{use_automation_data, AutomationRuntime};
use super::draft::{AutomationConfigDraft, AutomationProtectionDraft};

pub(in crate::panels) fn automation_module(runtime: AutomationRuntime) -> impl IntoView {
    let data = use_automation_data(runtime);
    let draft = AutomationConfigDraft::new(data.status, data.config_saved);
    let protection_draft = AutomationProtectionDraft::new(data.protection, data.protection_saved);
    view! {
        <section class="module-page automation-page">
            <ModuleHeader title="自动化"/>
            <div class="automation-workbench">
                {control_rail(draft, protection_draft, data)}
                <div class="automation-main-column">
                    <div class="automation-sync-status" role="status">
                        <span>{move || data.status.with(|state| match state {
                            crate::state::load_state::LoadState::Loading => "正在读取自动化状态".into(),
                            crate::state::load_state::LoadState::Ready(_) => format!("状态已确认 · {}", data.source.get()),
                            crate::state::load_state::LoadState::Stale { problem, .. } => format!("状态待确认，显示上次快照：{}", problem.message),
                            crate::state::load_state::LoadState::Error(problem) => format!("运行态读取失败：{}", problem.message),
                        })}</span>
                        <button type="button" class="row-action" disabled=move || data.reading.get() || data.busy.get()
                            on:click=move |_| data.refresh.run(())>{move || if data.reading.get() { "读取中…" } else { "刷新运行态" }}</button>
                    </div>
                    {automation_workspace(data.status, data.protection, data.webhook)}
                    <section class="automation-delivery-rail" aria-label="自动化提醒投递">
                        {webhook_monitor_disclosure(
                            "自动化实时 Webhook",
                            WebhookEventKind::AutomationDecision,
                            data.webhook,
                            data.webhook_problem,
                            data.test_webhook,
                            Some(data.webhook_feedback),
                        )}
                    </section>
                </div>
            </div>
        </section>
    }
}
