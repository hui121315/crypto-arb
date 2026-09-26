use super::*;

pub(super) fn credential_editor(input: CredentialEditorInput) -> impl IntoView {
    let CredentialEditorInput {
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
    } = input;
    let locked = Memo::new(move |_| save_action.journal.locked());
    let save = move || {
        submit_selected_credentials(
            credentials,
            selected,
            drafts,
            save_action,
            maintenance_action.state,
            message,
        );
    };

    view! {
        <form class="credential-editor" on:submit=move |event| {
            event.prevent_default();
            save();
        }>
            <div class="credential-editor-head">
                <label class="credential-venue">
                    <span>"交易所"</span>
                    <select
                        prop:value=move || selected.get()
                        disabled=move || locked.get()
                        on:change=move |ev| selected.set(event_target_value(&ev))
                    >
                        {move || venue_options(settings_state(credentials), selected)}
                    </select>
                </label>
                <div class="credential-summary" aria-live="polite">
                    <span>"当前能力"</span>
                    {move || {
                        let row = selected_credential.get();
                        credential_summary_facts_view(row.as_ref())
                    }}
                </div>
            </div>
            <fieldset class="credential-fields" disabled=move || locked.get()>
                {credential_inputs(credentials, selected, drafts)}
            </fieldset>
            <div class="credential-editor-actions">
                <em class="credential-draft-status" aria-live="polite">
                    {move || match credential_draft_count.get() {
                        0 => "仅填写需要更新的字段；留空保持现有值".to_owned(),
                        count => format!("将更新 {count} 项；其它字段保持不变"),
                    }}
                </em>
                <button
                    type="submit"
                    class=move || {
                        if credential_draft_count.get() > 0
                            || save_action.state.get().is_pending()
                        {
                            "primary-blue"
                        } else {
                            "row-action"
                        }
                    }
                    disabled=move || {
                        locked.get()
                            || !credential_spec_ready.get()
                            || credential_draft_count.get() == 0
                    }
                >
                    {move || {
                        if save_action.state.get().is_pending() {
                            "保存中".to_owned()
                        } else {
                            match credential_draft_count.get() {
                                0 => "填写后保存".to_owned(),
                                count => format!("保存 {count} 项"),
                            }
                        }
                    }}
                </button>
                {refresh_evidence_button(
                    [
                        refresh_nonce,
                        credentials_refresh_nonce,
                        runtime_health_refresh_nonce,
                        account_state_refresh_nonce,
                    ],
                    [save_action.state, maintenance_action.state],
                    message,
                )}
            </div>
            {credential_save_result(
                selected,
                selected_credential,
                save_action.state,
                message,
            )}
        </form>
    }
}
