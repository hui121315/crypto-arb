use crate::panels::shared::{webhook_monitor_disclosure, ModuleHeader};
use leptos::prelude::*;
use shared_types::WebhookEventKind;

use super::components::{automation_workspace, control_rail};
use super::data::{use_automation_data, AutomationRuntime};

pub(in crate::panels) fn automation_module(runtime: AutomationRuntime) -> impl IntoView {
    let data = use_automation_data(runtime);
    let receipts = super::receipts::use_receipts(data.status);
    let draft = runtime.draft;
    let protection_draft = runtime.protection_draft;
    let mutations = data.mutations.get_value();
    let risk = data.risk.get_value();
    view! {
        <section class="module-page automation-page">
            <div class="automation-heading">
            <ModuleHeader title="自动化"/>
            {crate::panels::shared::operation_journal::settings_recovery_panel(mutations.journal, mutations.recheck).into_any()}
            {crate::panels::shared::operation_journal::settings_recovery_panel(risk.kill.journal, risk.recheck).into_any()}
            <Show when=move || mutations.needs_current.get()>
                <div class="provider-credentials-feedback provider-credentials-recovery has-action" role="alert" aria-label="自动化当前配置待同步">
                    <span>"原操作已核对，当前配置仍待读取；暂不允许重复修改或启动。"</span>
                    <button type="button" class="row-action" disabled=move || data.reading.get()
                        on:click=move |_| mutations.read_current.run(())>"重新读取当前配置"</button>
                </div>
            </Show>
            <Show when=move || mutations.journal.locked() || mutations.needs_current.get()>
                <div class="automation-sync-status">
                    <span>"结果未确认不代表后台已暂停；风控总闸仍可独立核对。"</span>
                    <a class="row-action" href="#settings" on:click=move |_| crate::panels::modules::settings::select_risk_tab()>"查看风控总闸"</a>
                </div>
            </Show>
            </div>
            <div class="automation-workbench">
                {control_rail(draft, protection_draft, data).into_any()}
                <div class="automation-main-column">
                    <div class="automation-sync-status" role="status">
                        <span>{move || data.status.with(|state| match state {
                            crate::state::load_state::LoadState::Loading => "正在读取自动化状态".into(),
                            crate::state::load_state::LoadState::Ready(_) => format!("状态已确认 · {}", data.source.get()),
                            crate::state::load_state::LoadState::Stale { problem, .. } => format!("状态待确认，显示上次快照：{}", problem.message),
                            crate::state::load_state::LoadState::Error(problem) if problem.code == "AUTOMATION_RESULT_UNKNOWN" => problem.message.clone(),
                            crate::state::load_state::LoadState::Error(problem) => format!("运行状态读取失败：{}", problem.message),
                        })}</span>
                        <button type="button" class="row-action" disabled=move || data.reading.get() || data.busy.get()
                            on:click=move |_| data.refresh.run(())>{move || if data.reading.get() { "读取中…" } else { "刷新运行状态" }}</button>
                    </div>
                    {automation_workspace(data.status, data.protection, data.webhook, receipts)}
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
