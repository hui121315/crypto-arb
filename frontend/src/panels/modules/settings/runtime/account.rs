use super::super::data::{bump_refresh, SettingsJournal};
use super::super::data::{
    use_trading_adapter_select_action, use_venue_credential_maintenance_action,
    use_venue_credential_save_action, SettingsResource, TradingAdapterSelectAction,
    VenueCredentialMaintenanceAction, VenueCredentialSaveAction,
};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ActionRun, ActionRunKind, ActionRunStatus, TradingAdaptersResponse};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct CredentialRuntime {
    pub refresh: RwSignal<u64>,
    pub health_refresh: RwSignal<u64>,
    pub account_refresh: RwSignal<u64>,
    pub selected: RwSignal<String>,
    pub previous_selected: RwSignal<Option<String>>,
    pub save: VenueCredentialSaveAction,
    pub maintenance: VenueCredentialMaintenanceAction,
    pub draft_cleared: RwSignal<bool>,
    pub recheck: Callback<()>,
}

pub(super) fn create_credential_runtime() -> CredentialRuntime {
    let refresh = RwSignal::new(0);
    let health_refresh = RwSignal::new(0);
    let account_refresh = RwSignal::new(0);
    let journal = SettingsJournal::new("credentials");
    let save = use_venue_credential_save_action(refresh, health_refresh, account_refresh, journal);
    let maintenance =
        use_venue_credential_maintenance_action(refresh, health_refresh, account_refresh, journal);
    let selected = RwSignal::new(String::new());
    Effect::new(move |_| {
        if let Some(attempt) = journal.pending.get() {
            selected.set(attempt.target);
        }
    });
    let recheck = journal.recheck(Callback::new(move |run: ActionRun| {
        let state = crate::state::action_state::action_state_from_action_run(&run);
        if run.kind == ActionRunKind::VenueCredentialsUpdate {
            save.state.set(state);
            if run.status == ActionRunStatus::Succeeded {
                save.saved_revision
                    .update(|value| *value = value.wrapping_add(1));
            }
        } else {
            maintenance.state.set(state);
        }
        bump_refresh(refresh);
        bump_refresh(health_refresh);
        bump_refresh(account_refresh);
    }));
    CredentialRuntime {
        refresh,
        health_refresh,
        account_refresh,
        selected,
        previous_selected: RwSignal::new(None),
        save,
        maintenance,
        recheck,
        draft_cleared: RwSignal::new(false),
    }
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct EnvironmentRuntime {
    pub refresh: RwSignal<u64>,
    pub adapters: SettingsResource<TradingAdaptersResponse>,
    pub select: TradingAdapterSelectAction,
}

pub(super) fn create_environment_runtime() -> EnvironmentRuntime {
    let refresh = RwSignal::new(0);
    let adapters = RwSignal::new(LoadState::Loading);
    EnvironmentRuntime {
        refresh,
        adapters,
        select: use_trading_adapter_select_action(
            refresh,
            adapters,
            SettingsJournal::new("environment"),
        ),
    }
}
