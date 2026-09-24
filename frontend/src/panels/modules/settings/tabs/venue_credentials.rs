use crate::state::action_state::ActionState;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::credential_matrix::{CredentialProbeMatrix, CredentialReadiness};
use shared_types::{
    normalized_venue_name, venue_family, ApiProblem, ExchangeWsOperation, ExchangeWsReleaseStatus,
    ExchangeWsSupportStatus, ExchangeWsVenue, SecretStorageHealth, SecretStorageMode,
    SecretStorageStatus, VenueCredentialField, VenueCredentialStatus, VenueCredentialsResponse,
    VenueOperationEvidence, VenueOperationHealth, VenueOperationHealthSnapshot, VenueOperationKind,
    VenueOperationStatus,
};

use super::super::data::{
    bump_refresh, settings_state, settings_value, use_account_state_snapshot, use_action_runs,
    use_exchange_ws_venues, use_fee_schedule_registry, use_rest_endpoint_registry,
    use_scoped_venue_operation_health, use_venue_credential_maintenance_action,
    use_venue_credential_save_action, use_venue_credentials, VenueCredentialMaintenance,
    VenueCredentialMaintenanceAction, VenueCredentialSave, VenueCredentialSaveAction,
};
use super::{action_message, problem_cell, problem_message};
use crate::panels::modules::pagination::{page_controls, use_table_runtime, TableRuntimeHandle};

const CREDENTIAL_FIELDS_PAGE_SIZE: usize = 20;
const CREDENTIAL_FIELDS_PAGE_STORAGE_KEY: &str = "crossline.settings.credentials.fields.page";
const RUNTIME_HEALTH_PAGE_SIZE: usize = 20;
const RUNTIME_HEALTH_PAGE_STORAGE_KEY: &str = "crossline.settings.credentials.runtime.page";

mod action_result;
mod capability;
mod derive;
mod editor;
mod fees;
mod format;
mod inputs;
mod maintenance;
mod panels;
mod rest;
mod selection;
#[cfg(test)]
mod tests_credentials;
#[cfg(test)]
mod tests_runtime;
mod validation;
mod ws;

use action_result::*;
use capability::*;
use derive::*;
use editor::*;
use fees::*;
use format::*;
use inputs::*;
use maintenance::*;
use panels::*;
use rest::*;
use selection::*;
use validation::*;
use ws::*;

pub(in crate::panels::modules::settings) fn venue_credentials_matrix(
    refresh_nonce: RwSignal<u64>,
) -> impl IntoView {
    let credentials_refresh_nonce = RwSignal::new(0_u64);
    let runtime_health_refresh_nonce = RwSignal::new(0_u64);
    let account_state_refresh_nonce = RwSignal::new(0_u64);
    let credentials = use_venue_credentials(credentials_refresh_nonce);
    let account_state = use_account_state_snapshot(account_state_refresh_nonce);
    let ws_venues = use_exchange_ws_venues(refresh_nonce);
    let rest_endpoints = use_rest_endpoint_registry(refresh_nonce);
    let fee_schedules = use_fee_schedule_registry(refresh_nonce);
    let action_runs = use_action_runs(credentials_refresh_nonce);
    let save_action = use_venue_credential_save_action(
        credentials_refresh_nonce,
        runtime_health_refresh_nonce,
        account_state_refresh_nonce,
        action_runs,
    );
    let maintenance_action = use_venue_credential_maintenance_action(
        credentials_refresh_nonce,
        runtime_health_refresh_nonce,
        account_state_refresh_nonce,
        action_runs,
    );
    let selected = RwSignal::new(String::new());
    let operation_health =
        use_scoped_venue_operation_health(runtime_health_refresh_nonce, selected);
    let drafts = RwSignal::new(Vec::<CredentialDraftValue>::new());
    let selected_credential = Memo::new(move |_| {
        selected_credential_status(settings_value(credentials), &selected.get())
    });
    let credential_spec_ready = Memo::new(move |_| {
        credentials.with(|state| credential_spec_state(state, &selected.get()).can_save())
    });
    let credential_draft_count = Memo::new(move |_| {
        credentials
            .with(|state| credential_draft_field_count(state, &selected.get(), &drafts.get()))
    });
    let credential_fields = Memo::new(move |_| {
        selected_credential_fields(settings_value(credentials), &selected.get())
    });
    let credential_fields_for_key = credential_fields;
    let credential_fields_key = Memo::new(move |_| {
        credential_fields_for_key
            .with(|fields| credential_fields_dataset_key(&selected.get(), fields))
    });
    let credential_fields_table = use_table_runtime(
        CREDENTIAL_FIELDS_PAGE_STORAGE_KEY,
        credential_fields_key,
        credential_fields,
        CREDENTIAL_FIELDS_PAGE_SIZE,
    );
    let runtime_selection = Memo::new(move |_| {
        selected_runtime_health_selection(settings_value(operation_health), &selected.get())
    });
    let runtime_rows =
        Memo::new(move |_| runtime_selection.with(|selection| selection.rows.clone()));
    let runtime_selection_for_key = runtime_selection;
    let runtime_rows_key = Memo::new(move |_| {
        runtime_selection_for_key
            .with(|selection| runtime_selection_dataset_key(&selected.get(), selection))
    });
    let runtime_health_table = use_table_runtime(
        RUNTIME_HEALTH_PAGE_STORAGE_KEY,
        runtime_rows_key,
        runtime_rows,
        RUNTIME_HEALTH_PAGE_SIZE,
    );
    let message =
        RwSignal::new("选择交易所后保存凭证字段；持久化位置以 Secret 存储状态为准。".to_string());
    let previous_selected = RwSignal::new(None::<String>);

    install_initial_venue_selection(credentials, selected);
    install_credential_selection_reset(
        selected,
        previous_selected,
        save_action,
        maintenance_action,
        message,
        drafts,
    );
    install_successful_credential_draft_clear(save_action.saved_revision, drafts);

    view! {
        <div class="settings-stack">
            {credential_editor(CredentialEditorInput {
                refresh_nonce,
                credentials_refresh_nonce,
                runtime_health_refresh_nonce,
                account_state_refresh_nonce,
                credentials,
                selected,
                drafts,
                selected_credential,
                credential_spec_ready,
                credential_draft_count,
                save_action,
                maintenance_action,
                message,
            })}
            {credential_maintenance_controls(
                selected_credential,
                maintenance_action,
                save_action.state,
            )}
            <details class="credential-evidence-group">
                <summary>
                    <span><strong>"凭证与存储证据"</strong><small>"Secret、字段来源与保存期验证"</small></span>
                    <em>{move || credentials_evidence_summary(settings_state(credentials))}</em>
                </summary>
                <div class="credential-evidence-body">
                    {move || credentials_panel(
                        settings_state(credentials),
                        &selected.get(),
                        &credential_fields_table
                    )}
                </div>
            </details>
            <details class="credential-evidence-group">
                <summary>
                    <span><strong>"运行态与账户证据"</strong><small>"交易权限、私有流、终态与账户字段"</small></span>
                    <em>{move || runtime_selection.with(|selection| {
                        if selection.total == 0 {
                            "暂无运行证据".to_owned()
                        } else if selection.attention == 0 {
                            format!("{} 条正常", selection.total)
                        } else {
                            format!("{} / {} 条需处理", selection.attention, selection.total)
                        }
                    })}</em>
                </summary>
                <div class="credential-evidence-body settings-stack">
                    {move || {
                        runtime_health_panel(
                            settings_state(operation_health),
                            &selected.get(),
                            &runtime_health_table,
                        )
                    }}
                    {move || account_state_evidence_panel(settings_state(account_state), &selected.get())}
                </div>
            </details>
            <details class="credential-evidence-group">
                <summary>
                    <span><strong>"接口与官方证据"</strong><small>"WS、REST endpoint 与费率注册表"</small></span>
                    <em>"技术明细"</em>
                </summary>
                <div class="credential-evidence-body settings-stack">
                    {move || ws_panel(settings_state(ws_venues), &selected.get())}
                    {move || rest_panel(settings_state(rest_endpoints), &selected.get())}
                    {move || fee_schedule_panel(settings_state(fee_schedules), &selected.get())}
                </div>
            </details>
        </div>
    }
}

#[derive(Clone, Copy)]
struct CredentialEditorInput {
    refresh_nonce: RwSignal<u64>,
    credentials_refresh_nonce: RwSignal<u64>,
    runtime_health_refresh_nonce: RwSignal<u64>,
    account_state_refresh_nonce: RwSignal<u64>,
    credentials: RwSignal<LoadState<VenueCredentialsResponse>>,
    selected: RwSignal<String>,
    drafts: RwSignal<Vec<CredentialDraftValue>>,
    selected_credential: Memo<Option<VenueCredentialStatus>>,
    credential_spec_ready: Memo<bool>,
    credential_draft_count: Memo<usize>,
    save_action: VenueCredentialSaveAction,
    maintenance_action: VenueCredentialMaintenanceAction,
    message: RwSignal<String>,
}

fn credentials_evidence_summary(state: LoadState<VenueCredentialsResponse>) -> String {
    match state {
        LoadState::Loading => "读取中".to_owned(),
        LoadState::Error(_) => "读取失败".to_owned(),
        LoadState::Ready(response)
        | LoadState::Stale {
            value: response, ..
        } => {
            let storage = response.secret_storage;
            let encryption = if storage.encrypted {
                "已加密"
            } else {
                "未加密"
            };
            format!(
                "{} · {encryption}",
                secret_storage_health_label(storage.health)
            )
        }
    }
}

struct CredentialCapabilityFact {
    label: &'static str,
    value: String,
    detail: String,
    tone: &'static str,
}

fn credential_summary_facts_view(row: Option<&VenueCredentialStatus>) -> impl IntoView {
    let title = selected_summary(row);
    let facts = credential_summary_facts(row);
    view! {
        <div class="credential-capability-grid" title=title>
            {facts.into_iter().map(|fact| view! {
                <div class=format!("credential-capability-fact {}", fact.tone)>
                    <small>{fact.label}</small>
                    <strong>{fact.value}</strong>
                    <em>{fact.detail}</em>
                </div>
            }).collect_view()}
        </div>
    }
}

fn credential_summary_facts(row: Option<&VenueCredentialStatus>) -> Vec<CredentialCapabilityFact> {
    let Some(row) = row else {
        return vec![
            capability_fact("字段配置", "等待规格", "尚未取得字段定义", "is-pending"),
            capability_fact("保存期验证", "等待规格", "尚未取得验证证据", "is-pending"),
            capability_fact("实盘写侧", "等待规格", "尚未取得能力声明", "is-pending"),
        ];
    };
    let required_total = row.fields.iter().filter(|field| field.required).count();
    let required_configured = row
        .fields
        .iter()
        .filter(|field| field.required && field.configured)
        .count();
    let optional_total = row.fields.len().saturating_sub(required_total);
    let optional_configured = row
        .fields
        .iter()
        .filter(|field| !field.required && field.configured)
        .count();
    let fields_ready = row.missing_fields.is_empty();
    let field_value = if optional_total == 0 {
        format!("{required_configured}/{required_total} 已填写")
    } else {
        format!("{required_configured}/{required_total} 必填")
    };
    let field_detail = if fields_ready {
        if optional_total == 0 {
            "字段完整".to_owned()
        } else {
            format!("{optional_configured}/{optional_total} 可选字段已填写")
        }
    } else {
        format!("缺 {} 项字段", row.missing_fields.len())
    };
    let (validation_value, validation_detail, validation_tone) = credential_validation_fact(row);
    let write_value = if row.live_write {
        "仅静态声明"
    } else {
        "未声明"
    };
    let write_detail = if row.live_write {
        format!("{} · 仍需运行态证据", row.note)
    } else {
        row.note.clone()
    };
    vec![
        CredentialCapabilityFact {
            label: "字段配置",
            value: field_value,
            detail: field_detail,
            tone: if fields_ready {
                "is-ready"
            } else {
                "is-blocked"
            },
        },
        CredentialCapabilityFact {
            label: "保存期验证",
            value: validation_value,
            detail: validation_detail,
            tone: validation_tone,
        },
        CredentialCapabilityFact {
            label: "实盘写侧",
            value: write_value.to_owned(),
            detail: write_detail,
            tone: if row.live_write {
                "is-pending"
            } else {
                "is-neutral"
            },
        },
    ]
}

fn credential_validation_fact(row: &VenueCredentialStatus) -> (String, String, &'static str) {
    let Some(evidence) = row.validation_evidence.as_ref() else {
        return (
            "未验证".to_owned(),
            "保存凭证后生成验证证据".to_owned(),
            "is-pending",
        );
    };
    let probe_labels = evidence
        .blocking_links()
        .into_iter()
        .map(credential_link_label)
        .collect::<Vec<_>>()
        .join(" / ");
    match evidence.readiness() {
        CredentialReadiness::LiveReady => (
            "已验证".to_owned(),
            validation_status_label(evidence.status).to_owned(),
            "is-ready",
        ),
        CredentialReadiness::Blocked => (
            "权限阻断".to_owned(),
            if probe_labels.is_empty() {
                validation_status_label(evidence.status).to_owned()
            } else {
                format!(
                    "{} · {probe_labels}",
                    validation_status_label(evidence.status)
                )
            },
            "is-blocked",
        ),
        CredentialReadiness::Incomplete => (
            "待补证".to_owned(),
            if probe_labels.is_empty() {
                validation_status_label(evidence.status).to_owned()
            } else {
                format!("待补 {probe_labels}")
            },
            "is-pending",
        ),
    }
}

fn capability_fact(
    label: &'static str,
    value: &'static str,
    detail: &'static str,
    tone: &'static str,
) -> CredentialCapabilityFact {
    CredentialCapabilityFact {
        label,
        value: value.to_owned(),
        detail: detail.to_owned(),
        tone,
    }
}

fn credential_selection_changed(previous: Option<&str>, next: &str) -> bool {
    previous.is_some_and(|previous| !previous.is_empty() && previous != next)
}

fn submit_selected_credentials(
    credentials: RwSignal<LoadState<VenueCredentialsResponse>>,
    selected: RwSignal<String>,
    drafts: RwSignal<Vec<CredentialDraftValue>>,
    save_action: VenueCredentialSaveAction,
    maintenance_state: RwSignal<ActionState>,
    message: RwSignal<String>,
) {
    if save_action.state.get_untracked().is_pending()
        || maintenance_state.get_untracked().is_pending()
    {
        return;
    }
    let venue = selected.get_untracked();
    let spec = match credentials.with_untracked(|state| credential_spec_state(state, &venue)) {
        CredentialSpecState::Ready(spec) => spec,
        CredentialSpecState::Loading => {
            save_action.state.set(ActionState::Idle);
            message.set("凭证规格仍在加载，暂不可保存".into());
            return;
        }
        CredentialSpecState::SelectVenue => {
            save_action.state.set(ActionState::Idle);
            message.set("请先选择交易所".into());
            return;
        }
        CredentialSpecState::MissingSpec { venue } => {
            save_action.state.set(ActionState::Idle);
            message.set(format!("{venue} 缺少凭证规格，暂不可保存"));
            return;
        }
        CredentialSpecState::Error(problem) => {
            save_action
                .state
                .set(ActionState::failed("保存失败", problem));
            message.set("凭证规格加载失败".into());
            return;
        }
    };
    let fields = values_from(&spec, &drafts.get_untracked());
    if fields.is_empty() {
        save_action.state.set(ActionState::Idle);
        message.set("请至少填写一个字段".into());
        return;
    }
    save_action
        .submit
        .run(VenueCredentialSave { venue, fields });
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CredentialDraftValue {
    key: String,
    value: String,
}
