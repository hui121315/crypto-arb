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
    draft_cleared: RwSignal<bool>,
) {
    Effect::new(move |_| {
        let next = selected.get();
        let previous = previous_selected.get_untracked();
        previous_selected.set(Some(next.clone()));
        if !credential_selection_changed(previous.as_deref(), &next) {
            return;
        }
        if save_action.journal.locked()
        {
            return;
        }
        save_action.state.set(ActionState::Idle);
        maintenance_action.state.set(ActionState::Idle);
        draft_cleared.set(false);
        message.set("选择交易所后保存凭证字段；持久化位置以 Secret 存储状态为准。".into());
        drafts.set(Vec::new());
    });
}

pub(super) fn install_successful_credential_draft_clear(
    saved_revision: RwSignal<u64>,
    drafts: RwSignal<Vec<CredentialDraftValue>>,
) {
    let initial_revision = saved_revision.get_untracked();
    Effect::new(move |previous: Option<u64>| {
        let revision = saved_revision.get();
        if revision != previous.unwrap_or(initial_revision) && !drafts.get_untracked().is_empty() {
            drafts.set(Vec::new());
        }
        revision
    });
}
