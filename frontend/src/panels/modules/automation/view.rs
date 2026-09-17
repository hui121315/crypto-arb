use crate::panels::shared::{webhook_monitor_disclosure, ModuleHeader};
use leptos::prelude::*;
use shared_types::WebhookEventKind;

use super::components::{automation_workspace, control_rail};
use super::data::{use_automation_data, AutomationRuntime};
use super::draft::{AutomationConfigDraft, AutomationProtectionDraft};

pub(in crate::panels) fn automation_module(runtime: AutomationRuntime) -> impl IntoView {
    let data = use_automation_data(runtime);
    let draft = AutomationConfigDraft::new(data.status);
    let protection_draft = AutomationProtectionDraft::new(data.protection);
    view! {
        <section class="module-page automation-page">
            <ModuleHeader title="自动化"/>
            <div class="automation-workbench">
                {control_rail(draft, protection_draft, data)}
                <div class="automation-main-column">
                    {automation_workspace(data.status, data.protection, data.webhook)}
                    <section class="automation-delivery-rail" aria-label="自动化提醒投递">
                        {webhook_monitor_disclosure(
                            "自动化实时 Webhook",
                            WebhookEventKind::AutomationDecision,
                            data.webhook,
                            data.webhook_problem,
                            data.test_webhook,
                        )}
                    </section>
                </div>
            </div>
        </section>
    }
}
