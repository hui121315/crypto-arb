use crate::state::action_state::ActionState;
use leptos::prelude::*;

use super::{
    credential_selection_changed, settings_value, CredentialDraftValue,
    VenueCredentialMaintenanceAction, VenueCredentialSaveAction, VenueCredentialsResponse,
};

pub(super) fn install_initial_venue_selection(
    credentials: RwSignal<crate::state::load_state::LoadState<VenueCredentialsResponse>>,
    selected: RwSignal<String>,
) {
    Effect::new(move |_| {
        if !selected.get_untracked().is_empty() {
            return;
        }
        let Some(first) = settings_value(credentials)
            .and_then(|response| response.venues.into_iter().next())
            .map(|venue| venue.venue)
        else {
            return;
        };
        selected.set(first);
    });
}

pub(super) fn install_credential_selection_reset(
    selected: RwSignal<String>,
    previous_selected: RwSignal<Option<String>>,
    save_action: VenueCredentialSaveAction,
    maintenance_action: VenueCredentialMaintenanceAction,
    message: RwSignal<String>,
    drafts: RwSignal<Vec<CredentialDraftValue>>,
) {
    Effect::new(move |_| {
        let next = selected.get();
        let previous = previous_selected.get_untracked();
        previous_selected.set(Some(next.clone()));
        if !credential_selection_changed(previous.as_deref(), &next) {
            return;
        }
        save_action.state.set(ActionState::Idle);
        maintenance_action.state.set(ActionState::Idle);
        message.set("选择交易所后保存凭证字段；持久化位置以 Secret 存储状态为准。".into());
        drafts.set(Vec::new());
    });
}

pub(super) fn install_successful_credential_draft_clear(
    save_state: RwSignal<ActionState>,
    drafts: RwSignal<Vec<CredentialDraftValue>>,
) {
    Effect::new(move |_| {
        if matches!(save_state.get(), ActionState::Succeeded { .. })
            && !drafts.get_untracked().is_empty()
        {
            drafts.set(Vec::new());
        }
    });
}
