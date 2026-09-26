use super::*;

pub(super) fn refresh_evidence_button(
    refresh_nonces: [RwSignal<u64>; 4],
    action_states: [RwSignal<ActionState>; 2],
    message: RwSignal<String>,
) -> impl IntoView {
    let pending_states = action_states;
    view! {
        <button
            type="button"
            class="row-action"
            title="刷新当前交易所的凭证、账户和运行数据依据"
            disabled=move || pending_states.iter().any(|state| state.get().is_pending())
            on:click=move |_| {
                for nonce in refresh_nonces {
                    bump_refresh(nonce);
                }
                message.set("已请求刷新当前交易所数据依据".into());
            }
        >
            "刷新当前数据依据"
        </button>
    }
}

pub(super) fn credential_maintenance_controls(
    selected: Memo<Option<VenueCredentialStatus>>,
    action: VenueCredentialMaintenanceAction,
    save_state: RwSignal<ActionState>,
) -> impl IntoView {
    let confirmation = RwSignal::new(String::new());
    let locked = Memo::new(move |_| action.journal.locked());
    let clear = move |_| {
        if locked.get_untracked() {
            return;
        }
        let Some(status) = selected.get_untracked() else {
            return;
        };
        let fields = configured_field_keys(&status);
        if fields.is_empty()
            || !clear_confirmation_matches(&confirmation.get_untracked(), &status.venue)
        {
            return;
        }
        save_state.set(ActionState::Idle);
        confirmation.set(String::new());
        action.submit.run(VenueCredentialMaintenance::Clear {
            venue: status.venue,
            fields,
        });
    };
    let migrate = move |_| {
        if locked.get_untracked() {
            return;
        }
        let Some(status) = selected.get_untracked() else {
            return;
        };
        if configured_field_keys(&status).is_empty() {
            return;
        }
        save_state.set(ActionState::Idle);
        action.submit.run(VenueCredentialMaintenance::Migrate {
            venue: status.venue,
        });
    };

    view! {
        <details class="credential-maintenance">
            <summary>
                <strong>"凭证维护"</strong>
                <span>"迁移存储或清空当前交易所字段"</span>
            </summary>
            <div class="settings-actions credential-maintenance-actions">
                <label class="credential-clear-confirmation">
                    <span>{move || format!(
                        "输入 {} 确认清空",
                        clear_confirmation_phrase(&selected.get()),
                    )}</span>
                    <input
                        aria-label="清空确认"
                        disabled=move || locked.get()
                        placeholder=move || clear_confirmation_phrase(&selected.get())
                        prop:value=move || confirmation.get()
                        on:input=move |ev| confirmation.set(event_target_value(&ev))
                    />
                </label>
                <button
                    type="button"
                    class="row-action credential-clear-action"
                    disabled=move || {
                        locked.get()
                            || !can_clear(&selected.get(), &confirmation.get(), &action.state.get())
                    }
                    on:click=clear
                >
                    {move || if action.state.get().is_pending() { "处理中" } else { "清空已填字段" }}
                </button>
                <button
                    type="button"
                    class="row-action"
                    disabled=move || {
                        locked.get()
                            || !can_migrate(&selected.get(), &action.state.get())
                    }
                    on:click=migrate
                >
                    "迁移到当前存储"
                </button>
                <em>{move || action_message("", &action.state.get())}</em>
            </div>
        </details>
    }
}

pub(super) fn configured_field_keys(status: &VenueCredentialStatus) -> Vec<String> {
    status
        .fields
        .iter()
        .filter(|field| field.configured)
        .map(|field| field.key.clone())
        .collect()
}

pub(super) fn clear_confirmation_phrase(status: &Option<VenueCredentialStatus>) -> String {
    status.as_ref().map_or_else(
        || "CLEAR venue".to_owned(),
        |status| format!("CLEAR {}", status.venue),
    )
}

pub(super) fn clear_confirmation_matches(value: &str, venue: &str) -> bool {
    value.trim() == format!("CLEAR {venue}")
}

fn can_clear(
    status: &Option<VenueCredentialStatus>,
    confirmation: &str,
    action: &ActionState,
) -> bool {
    status.as_ref().is_some_and(|status| {
        !configured_field_keys(status).is_empty()
            && clear_confirmation_matches(confirmation, &status.venue)
            && !action.is_pending()
    })
}

fn can_migrate(status: &Option<VenueCredentialStatus>, action: &ActionState) -> bool {
    status
        .as_ref()
        .is_some_and(|status| !configured_field_keys(status).is_empty() && !action.is_pending())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_requires_the_selected_venue_phrase() {
        assert!(clear_confirmation_matches("CLEAR okx", "okx"));
        assert!(!clear_confirmation_matches("CLEAR binance", "okx"));
        assert!(!clear_confirmation_matches("clear okx", "okx"));
    }
}
