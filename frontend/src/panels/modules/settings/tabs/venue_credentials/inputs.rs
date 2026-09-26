use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum CredentialSpecState {
    Loading,
    Error(ApiProblem),
    SelectVenue,
    MissingSpec { venue: String },
    Ready(VenueCredentialStatus),
}

impl CredentialSpecState {
    pub(super) fn can_save(&self) -> bool {
        matches!(self, Self::Ready(spec) if !spec.fields.is_empty())
    }
}

pub(super) fn credential_spec_state(
    state: &LoadState<VenueCredentialsResponse>,
    venue: &str,
) -> CredentialSpecState {
    let response = match state {
        LoadState::Loading => return CredentialSpecState::Loading,
        LoadState::Error(problem) => return CredentialSpecState::Error(problem.clone()),
        LoadState::Ready(response)
        | LoadState::Stale {
            value: response, ..
        } => response,
    };
    let venue = venue.trim();
    if venue.is_empty() {
        return CredentialSpecState::SelectVenue;
    }
    response
        .venues
        .iter()
        .find(|row| row.venue == venue)
        .cloned()
        .map(CredentialSpecState::Ready)
        .unwrap_or_else(|| CredentialSpecState::MissingSpec {
            venue: venue.to_owned(),
        })
}

pub(super) fn credential_inputs(
    state: RwSignal<LoadState<VenueCredentialsResponse>>,
    venue: RwSignal<String>,
    drafts: RwSignal<Vec<CredentialDraftValue>>,
) -> impl IntoView {
    let spec = Memo::new(move |_| credential_spec_state(&state.get(), &venue.get()));
    let fields = Memo::new(move |_| match spec.get() {
        CredentialSpecState::Ready(row) => row
            .fields
            .into_iter()
            .map(|field| (row.venue.clone(), field))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    });
    view! {
    {move || match spec.get() {
        CredentialSpecState::Loading => view! {
            <div class="empty-cell" data-credential-spec-state="loading">
                "交易所凭证规格加载中"
            </div>
        }
        .into_any(),
        CredentialSpecState::Error(problem) => {
            let message = problem_message("读取凭证规格失败", &problem);
            view! {
                <div class="empty-cell" data-credential-spec-state="error">{message}</div>
            }
            .into_any()
        }
        CredentialSpecState::SelectVenue => view! {
            <div class="empty-cell" data-credential-spec-state="select-venue">"请选择交易所"</div>
        }
        .into_any(),
        CredentialSpecState::MissingSpec { venue } => view! {
            <div class="empty-cell" data-credential-spec-state="missing">
                {format!("{venue} 缺少凭证规格，暂不可保存")}
            </div>
        }
        .into_any(),
        CredentialSpecState::Ready(_) => ().into_any(),
    }}
    <For each=move || fields.get() key=|(venue, field)| (venue.clone(), field.key.clone()) children=move |(venue, initial)| {
        let field = Memo::new(move |_| state.with(|state| state.value()
            .and_then(|response| response.venues.iter().find(|row| row.venue == venue))
            .and_then(|row| row.fields.iter().find(|field| field.key == initial.key))
            .cloned().unwrap_or_else(|| initial.clone())));
        credential_input(field, drafts)
    }/>
    }
}

pub(super) fn credential_input(
    field: Memo<VenueCredentialField>,
    drafts: RwSignal<Vec<CredentialDraftValue>>,
) -> impl IntoView {
    let key = field.get_untracked().key;
    let value_key = key.clone();
    let input_key = key.clone();
    let input_name = key.clone();
    let identity = credential_identity(&key);
    let input_type = move || {
        if field.get().secret && !identity {
            "password"
        } else {
            "text"
        }
    };
    let input_class = identity.then_some("credential-identity-input");
    let autocomplete = move || credential_autocomplete(&key, field.get().secret);
    let placeholder = move || {
        let field = field.get();
        if field.configured {
            format!(
                "{} · 已有保存值，留空保持原值；权限需运行状态验证",
                field.env_key
            )
        } else {
            field.env_key.clone()
        }
    };
    view! {
        <label class="credential-field">
            <span>{move || field.get().label}</span>
            <input
                type=input_type
                class=input_class
                name=input_name
                autocomplete=autocomplete
                placeholder=placeholder
                prop:value=move || draft_value(&drafts.get(), &value_key)
                on:input=move |ev| set_draft_value(drafts, &input_key, event_target_value(&ev))
            />
        </label>
    }
}

fn credential_identity(key: &str) -> bool {
    key == "api_key"
        || key.ends_with("_api_key")
        || matches!(key, "live_key" | "account_address" | "vault_address")
}

fn credential_autocomplete(key: &str, secret: bool) -> &'static str {
    if credential_identity(key) {
        "username"
    } else if secret {
        "new-password"
    } else {
        "off"
    }
}

pub(super) fn values_from(
    spec: &VenueCredentialStatus,
    drafts: &[CredentialDraftValue],
) -> Vec<(String, String)> {
    let mut fields = Vec::with_capacity(spec.fields.len());
    for field in &spec.fields {
        push_value(
            &mut fields,
            field.key.as_str(),
            &draft_value(drafts, field.key.as_str()),
        );
    }
    fields
}

pub(super) fn credential_draft_field_count(
    state: &LoadState<VenueCredentialsResponse>,
    venue: &str,
    drafts: &[CredentialDraftValue],
) -> usize {
    match credential_spec_state(state, venue) {
        CredentialSpecState::Ready(spec) => values_from(&spec, drafts).len(),
        CredentialSpecState::Loading
        | CredentialSpecState::Error(_)
        | CredentialSpecState::SelectVenue
        | CredentialSpecState::MissingSpec { .. } => 0,
    }
}

pub(super) fn draft_value(drafts: &[CredentialDraftValue], key: &str) -> String {
    drafts
        .iter()
        .find(|draft| draft.key == key)
        .map(|draft| draft.value.clone())
        .unwrap_or_default()
}

pub(super) fn set_draft_value(
    drafts: RwSignal<Vec<CredentialDraftValue>>,
    key: &str,
    value: String,
) {
    let key = key.to_owned();
    drafts.update(
        move |rows| match rows.iter_mut().find(|draft| draft.key == key) {
            Some(draft) => draft.value = value,
            None => rows.push(CredentialDraftValue { key, value }),
        },
    );
}

pub(super) fn push_value(fields: &mut Vec<(String, String)>, key: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        fields.push((key.to_string(), value.to_string()));
    }
}

#[cfg(test)]
mod autocomplete_tests {
    use super::{credential_autocomplete, credential_identity};

    #[test]
    fn credential_fields_expose_account_and_secret_semantics() {
        assert_eq!(credential_autocomplete("api_key", true), "username");
        assert_eq!(credential_autocomplete("spot_api_key", true), "username");
        assert_eq!(credential_autocomplete("futures_api_key", true), "username");
        assert_eq!(credential_autocomplete("live_key", true), "username");
        assert_eq!(credential_autocomplete("api_secret", true), "new-password");
        assert_eq!(credential_autocomplete("passphrase", true), "new-password");
        assert_eq!(credential_autocomplete("memo", false), "off");
        assert!(credential_identity("api_key"));
        assert!(credential_identity("spot_api_key"));
        assert!(credential_identity("futures_api_key"));
        assert!(credential_identity("account_address"));
        assert!(!credential_identity("private_key"));
    }
}
