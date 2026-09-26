//! Settings 模块提交类 hooks：凭证保存 / 风控保存 / 交易急停，含幂等重放键。

use crate::state::action_state::ActionState;
use crate::state::trading_status::TradingStatusState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ActionRun, ActionRunKind, KillSwitchRequest, RiskConfigPatch};

use super::format::{credential_success_message, kill_switch_success_message};
use super::resources::{bump_refresh, SettingsResource};

#[path = "actions/transport.rs"]
mod transport;
use transport::{
    credential_response_evidence, kill_switch_evidence, save_venue_credentials_task,
    select_trading_adapter_task, set_kill_switch_task, trading_status_evidence,
    update_risk_config_task,
};

#[path = "api_base.rs"]
mod api_base;
pub(in crate::panels::modules::settings) use api_base::*;

pub(in crate::panels::modules::settings) struct VenueCredentialSave {
    pub venue: String,
    pub fields: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct VenueCredentialSaveAction {
    pub state: RwSignal<ActionState>,
    pub saved_revision: RwSignal<u64>,
    pub submit: Callback<VenueCredentialSave>,
    pub journal: super::SettingsJournal,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct RiskConfigSaveAction {
    pub state: RwSignal<ActionState>,
    pub receipt: RwSignal<Option<shared_types::TradingStatusResponse>>,
    pub journal: super::SettingsJournal,
    pub submit: Callback<RiskConfigPatch>,
}

#[derive(Clone, Copy)]
pub(in crate::panels) struct KillSwitchAction {
    pub state: RwSignal<ActionState>,
    pub journal: super::SettingsJournal,
    pub submit: Callback<KillSwitchRequest>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct TradingAdapterSelectAction {
    pub state: RwSignal<ActionState>,
    pub journal: super::SettingsJournal,
    pub recheck: Callback<()>,
    pub submit: Callback<String>,
}

pub(in crate::panels::modules::settings) fn use_trading_adapter_select_action(
    refresh_nonce: RwSignal<u64>,
    adapters: SettingsResource<shared_types::TradingAdaptersResponse>,
    journal: super::SettingsJournal,
) -> TradingAdapterSelectAction {
    let shared_status = use_context::<TradingStatusState>();
    let state = RwSignal::new(ActionState::Idle);
    Effect::new(move |_| {
        journal.connection.track();
        adapters.set(crate::state::load_state::LoadState::Loading);
        state.set(journal.restored_state(&[ActionRunKind::TradingAdapterSelect]));
    });
    let recheck = journal.recheck(Callback::new(move |run: ActionRun| {
        state.set(crate::state::action_state::action_state_from_action_run(
            &run,
        ));
        // A historical receipt proves the action, not today's execution environment.
        adapters.set(crate::state::load_state::LoadState::Loading);
        bump_refresh(refresh_nonce);
        let epoch = journal.epoch.get_untracked();
        let client = journal.client();
        if let Some(shared) = shared_status {
            let revision = shared.invalidate();
            spawn_local(async move {
                let result = crate::api::rest::with_mutation_timeout(
                    "读取当前执行环境",
                    client.trading_status(),
                )
                .await;
                if !journal.current(epoch) {
                    return;
                }
                shared.apply_read(revision, result.map_err(|error| error.problem));
            });
        }
    }));
    let submit = Callback::new(move |adapter_id: String| {
        let Some(attempt) = journal.begin(
            ActionRunKind::TradingAdapterSelect,
            adapter_id.trim().to_owned(),
        ) else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        let context = attempt.context.clone();
        let pending_evidence = context.evidence();
        let key = context.idempotency_key().unwrap_or_default().to_owned();
        state.set(ActionState::pending("正在切换执行环境").with_evidence(pending_evidence.clone()));
        let client = journal.client();
        spawn_local(async move {
            let result = select_trading_adapter_task(client, attempt.target.clone(), context)
                .await
                .and_then(|response| {
                    super::validate_setting_response(&attempt, &response)?;
                    Ok(response)
                });
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(response) => {
                    if let Some(shared) = shared_status {
                        shared.accept_receipt(response.clone());
                    }
                    let evidence = trading_status_evidence(&response, &key, pending_evidence);
                    if journal.resolve(&attempt) {
                        bump_refresh(refresh_nonce);
                        adapters.update(|state| match state {
                            crate::state::load_state::LoadState::Ready(value)
                            | crate::state::load_state::LoadState::Stale { value, .. } => {
                                value.current.clone_from(&response.adapter);
                                value.current_environment = response.environment;
                            }
                            _ => {}
                        });
                        let mode = crate::panels::shared::execution_environment_label(
                            response.environment,
                        );
                        state.set(
                            ActionState::succeeded(format!("执行环境已切换为{mode}"))
                                .with_evidence(evidence),
                        );
                    } else {
                        state.set(
                            ActionState::accepted("处理结果已返回，恢复记录待清理")
                                .with_evidence(evidence),
                        );
                    }
                    journal.busy.set(false);
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    bump_refresh(refresh_nonce);
                    state.set(
                        ActionState::failed("执行环境切换结果待确认", error.problem)
                            .with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    TradingAdapterSelectAction {
        state,
        journal,
        recheck,
        submit,
    }
}

pub(in crate::panels::modules::settings) fn use_venue_credential_save_action(
    credentials_refresh_nonce: RwSignal<u64>,
    runtime_health_refresh_nonce: RwSignal<u64>,
    account_state_refresh_nonce: RwSignal<u64>,
    journal: super::SettingsJournal,
) -> VenueCredentialSaveAction {
    let state = RwSignal::new(ActionState::Idle);
    let saved_revision = RwSignal::new(0_u64);
    Effect::new(move |_| {
        journal.connection.track();
        state.set(journal.restored_state(&[ActionRunKind::VenueCredentialsUpdate]));
    });
    let submit = Callback::new(move |request: VenueCredentialSave| {
        let Some(attempt) =
            journal.begin(ActionRunKind::VenueCredentialsUpdate, request.venue.clone())
        else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        let context = attempt.context.clone();
        let pending_evidence = context.evidence();
        let key = context.idempotency_key().unwrap_or_default().to_owned();
        state.set(
            ActionState::pending("正在校验凭证数据依据并保存字段")
                .with_evidence(pending_evidence.clone()),
        );
        let client = journal.client();
        spawn_local(async move {
            let result =
                save_venue_credentials_task(client, request.venue, request.fields, context)
                    .await
                    .and_then(|response| {
                        super::validate_setting_response(&attempt, &response)?;
                        Ok(response)
                    });
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(response) => {
                    let evidence = credential_response_evidence(&response, &key, pending_evidence);
                    if journal.resolve(&attempt) {
                        saved_revision.update(|revision| *revision = revision.wrapping_add(1));
                        bump_refresh(credentials_refresh_nonce);
                        bump_refresh(runtime_health_refresh_nonce);
                        bump_refresh(account_state_refresh_nonce);
                        state.set(
                            ActionState::succeeded(credential_success_message(&response))
                                .with_evidence(evidence),
                        );
                    } else {
                        state.set(
                            ActionState::accepted("处理结果已返回，恢复记录待清理")
                                .with_evidence(evidence),
                        );
                    }
                    journal.busy.set(false);
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    bump_refresh(credentials_refresh_nonce);
                    let label = if journal.pending.with_untracked(Option::is_some) {
                        "保存结果待确认"
                    } else {
                        "保存失败"
                    };
                    state.set(
                        ActionState::failed(label, error.problem).with_evidence(pending_evidence),
                    );
                }
            }
        });
    });
    VenueCredentialSaveAction {
        state,
        saved_revision,
        submit,
        journal,
    }
}

pub(in crate::panels::modules::settings) fn current_trading_refresh(
    journal: super::SettingsJournal,
) -> Callback<()> {
    let shared = expect_context::<TradingStatusState>();
    Callback::new(move |()| {
        let revision = shared.invalidate();
        let epoch = journal.epoch.get_untracked();
        let client = journal.client();
        spawn_local(async move {
            let result =
                crate::api::rest::with_mutation_timeout("读取当前风控", client.trading_status())
                    .await;
            if journal.current(epoch) {
                shared.apply_read(revision, result.map_err(|error| error.problem));
            }
        });
    })
}

pub(in crate::panels::modules::settings) fn use_risk_config_save_action(
    journal: super::SettingsJournal,
) -> RiskConfigSaveAction {
    let state = RwSignal::new(ActionState::Idle);
    let receipt = RwSignal::new(None);
    let shared = expect_context::<TradingStatusState>();
    Effect::new(move |_| {
        journal.connection.track();
        state.set(journal.restored_state(&[ActionRunKind::TradingRiskConfigUpdate]));
        receipt.set(None);
    });
    let submit = Callback::new(move |patch: RiskConfigPatch| {
        let Some(attempt) =
            journal.begin(ActionRunKind::TradingRiskConfigUpdate, "risk-config".into())
        else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        let evidence = attempt.context.evidence();
        let key = attempt
            .context
            .idempotency_key()
            .unwrap_or_default()
            .to_owned();
        state.set(ActionState::pending("正在保存风控参数").with_evidence(evidence.clone()));
        receipt.set(None);
        let client = journal.client();
        spawn_local(async move {
            let result = update_risk_config_task(client, patch, attempt.context.clone())
                .await
                .and_then(|response| {
                    super::validate_setting_response(&attempt, &response)?;
                    Ok(response)
                });
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(response) => {
                    shared.accept_receipt(response.clone());
                    if journal.resolve(&attempt) {
                        receipt.set(Some(response.clone()));
                        state.set(
                            ActionState::succeeded(super::format::risk_config_success_message(
                                &response, &key,
                            ))
                            .with_evidence(trading_status_evidence(&response, &key, evidence)),
                        );
                    } else {
                        state.set(
                            ActionState::accepted("处理结果已返回，恢复记录待清理")
                                .with_evidence(evidence),
                        );
                    }
                    journal.busy.set(false);
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    let label = if journal.pending.with_untracked(Option::is_some) {
                        "保存结果待核对"
                    } else {
                        "保存失败"
                    };
                    state.set(ActionState::failed(label, error.problem).with_evidence(evidence));
                }
            }
        });
    });
    RiskConfigSaveAction {
        state,
        receipt,
        journal,
        submit,
    }
}

pub(in crate::panels::modules::settings) fn use_kill_switch_action(
    journal: super::SettingsJournal,
) -> KillSwitchAction {
    let state = RwSignal::new(ActionState::Idle);
    let shared = expect_context::<TradingStatusState>();
    Effect::new(move |_| {
        journal.connection.track();
        state.set(journal.restored_state(&[ActionRunKind::TradingKillSwitch]));
    });
    let submit = Callback::new(move |request: KillSwitchRequest| {
        let target = if request.active {
            "kill-switch:on"
        } else {
            "kill-switch:off"
        };
        let Some(attempt) = journal.begin(ActionRunKind::TradingKillSwitch, target.into()) else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        let evidence = attempt.context.evidence();
        let key = attempt
            .context
            .idempotency_key()
            .unwrap_or_default()
            .to_owned();
        state.set(ActionState::pending("正在更新 交易急停").with_evidence(evidence.clone()));
        let client = journal.client();
        spawn_local(async move {
            let result = set_kill_switch_task(client, request, attempt.context.clone())
                .await
                .and_then(|response| {
                    super::validate_setting_response(&attempt, &response)?;
                    Ok(response)
                });
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(response) => {
                    shared.accept_receipt(response.status.clone());
                    if journal.resolve(&attempt) {
                        state.set(
                            ActionState::succeeded(kill_switch_success_message(&response))
                                .with_evidence(kill_switch_evidence(&response, &key, evidence)),
                        );
                    } else {
                        state.set(
                            ActionState::accepted("处理结果已返回，恢复记录待清理")
                                .with_evidence(evidence),
                        );
                    }
                    journal.busy.set(false);
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    let label = if journal.pending.with_untracked(Option::is_some) {
                        "总闸结果待核对"
                    } else {
                        "总闸更新失败"
                    };
                    state.set(ActionState::failed(label, error.problem).with_evidence(evidence));
                }
            }
        });
    });
    KillSwitchAction {
        state,
        journal,
        submit,
    }
}
