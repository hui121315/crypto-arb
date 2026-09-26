use crate::api::rest::{with_mutation_timeout, ApiClient};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{OnchainProviderCredentialMutationResponse, OnchainProviderCredentialsResponse, VenueCredentialValue};

use super::recovery::{
    outcome_unknown, read_with_timeout, recover_attempt, CredentialAttempt,
    CredentialOperation, CredentialRecovery,
};
use super::storage;

#[derive(Clone, Copy)]
pub(super) struct ProviderCredentialDraft {
    jupiter_api_key: RwSignal<String>,
    zeroex_api_key: RwSignal<String>,
    okx_api_key: RwSignal<String>,
    okx_secret_key: RwSignal<String>,
    okx_passphrase: RwSignal<String>,
    lifi_api_key: RwSignal<String>,
    solana_private_key: RwSignal<String>,
    evm_private_key: RwSignal<String>,
    backpack_api_key: RwSignal<String>,
    backpack_secret_key: RwSignal<String>,
}

impl ProviderCredentialDraft {
    pub(super) fn new() -> Self {
        Self {
            jupiter_api_key: RwSignal::new(String::new()),
            zeroex_api_key: RwSignal::new(String::new()),
            okx_api_key: RwSignal::new(String::new()),
            okx_secret_key: RwSignal::new(String::new()),
            okx_passphrase: RwSignal::new(String::new()),
            lifi_api_key: RwSignal::new(String::new()),
            solana_private_key: RwSignal::new(String::new()),
            evm_private_key: RwSignal::new(String::new()),
            backpack_api_key: RwSignal::new(String::new()),
            backpack_secret_key: RwSignal::new(String::new()),
        }
    }

    pub(super) fn signal(self, provider: &str, key: &str) -> Option<RwSignal<String>> {
        match (provider, key) {
            ("jupiter_swap_v2_keyed", "api_key") => Some(self.jupiter_api_key),
            ("zeroex_swap_v2", "api_key") => Some(self.zeroex_api_key),
            ("okx_dex_v6", "api_key") => Some(self.okx_api_key),
            ("okx_dex_v6", "secret_key") => Some(self.okx_secret_key),
            ("okx_dex_v6", "passphrase") => Some(self.okx_passphrase),
            ("lifi", "api_key") => Some(self.lifi_api_key),
            ("solana_wallet_signer", "private_key") => Some(self.solana_private_key),
            ("evm_wallet_signer", "private_key") => Some(self.evm_private_key),
            ("backpack_stocks", "api_key") => Some(self.backpack_api_key),
            ("backpack_stocks", "secret_key") => Some(self.backpack_secret_key),
            _ => None,
        }
    }

    pub(super) fn values(self, provider: &str) -> Vec<VenueCredentialValue> {
        field_keys(provider)
            .iter()
            .filter_map(|key| {
                let value = self.signal(provider, key)?.get_untracked();
                (!value.trim().is_empty()).then(|| VenueCredentialValue {
                    key: (*key).to_owned(),
                    value,
                })
            })
            .collect()
    }

    pub(super) fn has_values(self, provider: &str) -> bool {
        field_keys(provider).iter().any(|key| {
            self.signal(provider, key)
                .is_some_and(|signal| !signal.get().trim().is_empty())
        })
    }

    pub(super) fn clear(self, provider: &str) {
        for key in field_keys(provider) {
            if let Some(signal) = self.signal(provider, key) {
                signal.set(String::new());
            }
        }
    }
}

fn field_keys(provider: &str) -> &'static [&'static str] {
    match provider {
        "jupiter_swap_v2_keyed" => &["api_key"],
        "zeroex_swap_v2" => &["api_key"],
        "okx_dex_v6" => &["api_key", "secret_key", "passphrase"],
        "backpack_stocks" => &["api_key", "secret_key"],
        "lifi" => &["api_key"],
        "solana_wallet_signer" | "evm_wallet_signer" => &["private_key"],
        _ => &[],
    }
}

#[derive(Clone, Copy)]
pub(super) struct ProviderCredentialsData {
    pub state: RwSignal<LoadState<OnchainProviderCredentialsResponse>>,
    pub busy: RwSignal<bool>,
    pub reading: RwSignal<bool>,
    pub completed: RwSignal<(u64, Option<String>)>,
    pub feedback: RwSignal<Option<String>>,
    pub problem: RwSignal<Option<String>>,
    pub target: RwSignal<Option<String>>,
    pub cleared_drafts: RwSignal<Vec<String>>,
    pub pending: RwSignal<Option<CredentialAttempt>>,
    pub storage_problem: RwSignal<Option<String>>,
    storage_ready: RwSignal<bool>,
    active: RwSignal<usize>,
    version: RwSignal<u64>,
    pub reload: Callback<()>,
    pub save: Callback<(String, Vec<VenueCredentialValue>)>,
    pub clear: Callback<String>,
    pub recheck: Callback<()>,
}

impl ProviderCredentialsData {
    pub(super) fn locked(self) -> bool {
        self.busy.get() || self.pending.with(Option::is_some)
            || !self.storage_ready.get() || self.storage_problem.with(Option::is_some)
    }
}

pub(super) fn use_provider_credentials_data() -> ProviderCredentialsData {
    let data = expect_context::<ProviderCredentialsData>();
    data.active.update_untracked(|count| *count += 1);
    // Let the previous page unmount before notifying shared-state subscribers.
    spawn_local(async move { data.reload.run(()); });
    on_cleanup(move || {
        data.active.update_untracked(|count| *count = count.saturating_sub(1));
        if data.active.get_untracked() == 0 {
            data.version.update_untracked(|value| *value = value.wrapping_add(1));
            // Do not schedule renders of controls whose owner is being disposed.
            data.reading.update_untracked(|reading| *reading = false);
        }
    });
    data
}

pub(crate) fn provide_provider_credentials() {
    let app = expect_context::<crate::state::AppContext>();
    let api_base = app.api_base;
    let api_token = app.api_auth_token;
    let client = move || ApiClient::with_base_and_auth(&api_base.get_untracked(), &api_token.get_untracked());
    let state = RwSignal::new(LoadState::Loading);
    let busy = RwSignal::new(false);
    let reading = RwSignal::new(false);
    let version = RwSignal::new(0_u64);
    let completed = RwSignal::new((0_u64, None));
    let feedback = RwSignal::new(None);
    let problem = RwSignal::new(None);
    let target = RwSignal::new(None);
    let cleared_drafts = RwSignal::new(Vec::<String>::new());
    let pending = RwSignal::new(None::<CredentialAttempt>);
    let storage_problem = RwSignal::new(None::<String>);
    let storage_ready = RwSignal::new(false);
    let current_scope = move || storage::scope(&api_base.get_untracked(), &api_token.get_untracked());
    let scope = StoredValue::new(current_scope());
    let active = RwSignal::new(0_usize);
    let backend_revision = RwSignal::new(0_u64);

    let reload = Callback::new({
        move |()| {
            if active.get_untracked() == 0 || busy.get_untracked() || reading.get_untracked() {
                return;
            }
            fetch_status(client(), state, reading, version);
        }
    });
    let restore = move || {
        match storage::load(&scope.get_value()) {
            Ok(attempt) => {
                target.set(attempt.as_ref().map(|attempt| attempt.provider.clone()));
                pending.set(attempt);
                storage_problem.set(None);
            }
            Err(error) => storage_problem.set(Some(error)),
        }
        storage_ready.set(true);
    };
    let resolve = move |attempt: &CredentialAttempt| {
        match storage::resolve(&scope.get_value(), attempt) {
            Ok(()) => {
                storage_problem.set(None);
                pending.set(None);
                true
            }
            Err(error) => {
                storage_problem.set(Some(error));
                false
            }
        }
    };
    let succeeded = move |response: OnchainProviderCredentialMutationResponse, attempt: &CredentialAttempt| {
        if !resolve(attempt) { return; }
        feedback.set(Some(format!("{}：{}", response.label, response.message)));
        problem.set(None);
        cleared_drafts.update(|rows| rows.retain(|row| row != &response.provider));
        completed.update(|value| *value = (value.0.wrapping_add(1), Some(response.provider)));
        if active.get_untracked() > 0 {
            fetch_status(client(), state, reading, version);
        }
    };
    let submit = Callback::new({
        move |(provider, fields, operation): (String, Vec<VenueCredentialValue>, CredentialOperation)| {
            if busy.get_untracked()
                || pending.with_untracked(Option::is_some)
                || reading.get_untracked()
                || !storage_ready.get_untracked()
                || storage_problem.with_untracked(Option::is_some)
                || scope.get_value() != current_scope()
                || operation == CredentialOperation::Save && fields.is_empty()
                || !matches!(state.get_untracked(), LoadState::Ready(_))
            {
                return;
            }
            let client = client();
            let attempt = CredentialAttempt::new(provider.clone(), operation);
            target.set(Some(provider.clone()));
            feedback.set(None);
            problem.set(None);
            if let Err(error) = storage::persist(&scope.get_value(), &attempt, true) {
                restore();
                storage_problem.set(Some(error));
                return;
            }
            // Persist identity before dispatch, including a reload before timeout.
            pending.set(Some(attempt.clone()));
            busy.set(true);
            let revision = backend_revision.get_untracked();
            spawn_local(async move {
                let result = with_mutation_timeout("凭证操作", async {
                    let response = match operation {
                        CredentialOperation::Save => client
                            .save_onchain_provider_credentials(&provider, fields, &attempt.context).await?,
                        CredentialOperation::Clear => client
                            .clear_onchain_provider_credentials(&provider, Vec::new(), &attempt.context).await?,
                    };
                    attempt.validate_response(&response)?;
                    Ok(response)
                }).await;
                if backend_revision.try_get_untracked() != Some(revision) {
                    return;
                }
                match result {
                    Ok(response) => succeeded(response, &attempt),
                    Err(error) => {
                        if !outcome_unknown(&error) { resolve(&attempt); }
                        problem.set(Some(format!("{} · code {}", error, error.problem.code)));
                    }
                }
                busy.set(false);
            });
        }
    });
    let save = Callback::new(move |(provider, fields)| submit.run((provider, fields, CredentialOperation::Save)));
    let clear = Callback::new(move |provider| submit.run((provider, Vec::new(), CredentialOperation::Clear)));
    let recheck = Callback::new(move |()| {
        if busy.get_untracked() || reading.get_untracked() || scope.get_value() != current_scope() {
            return;
        }
        let Some(attempt) = pending.get_untracked() else { restore(); return; };
        let client = client();
        busy.set(true);
        problem.set(None);
        let revision = backend_revision.get_untracked();
        spawn_local(async move {
            let result = read_with_timeout("读取原凭证处理结果", recover_attempt(&client, attempt.clone())).await;
            if backend_revision.try_get_untracked() != Some(revision) {
                return;
            }
            match result {
                Ok(CredentialRecovery::Succeeded(response)) => succeeded(response, &attempt),
                Ok(CredentialRecovery::Waiting(attempt)) => {
                    storage_problem.set(storage::persist(&scope.get_value(), &attempt, false).err());
                    pending.set(Some(attempt));
                    problem.set(Some("后端已受理，尚未完成；请稍后核对，不会重新提交凭证。".into()));
                }
                Ok(CredentialRecovery::Failed(failure)) => {
                    resolve(&attempt);
                    problem.set(Some(format!("原操作已确认失败：{} · code {}", failure.message, failure.code)));
                    if active.get_untracked() > 0 {
                        fetch_status(client, state, reading, version);
                    }
                }
                Err(error) => problem.set(Some(format!("{} · code {}", error, error.problem.code))),
            }
            busy.set(false);
        });
    });

    let data = ProviderCredentialsData {
        state,
        busy,
        reading,
        completed,
        feedback,
        problem,
        target,
        cleared_drafts,
        pending,
        storage_problem,
        storage_ready,
        active,
        version,
        reload,
        save,
        clear,
        recheck,
    };
    // Shared receipts belong to one backend/auth context, never to its replacement.
    Effect::new(move |_| {
        app.api_base.track();
        app.api_auth_token.track();
        backend_revision.update(|value| *value = value.wrapping_add(1));
        version.update(|value| *value = value.wrapping_add(1));
        state.set(LoadState::Loading);
        reading.set(false);
        busy.set(false);
        feedback.set(None);
        problem.set(None);
        target.set(None);
        pending.set(None);
        storage_ready.set(false);
        scope.set_value(current_scope());
        restore();
        data.reload.run(());
    });
    provide_context(data);
}

pub(super) fn install_draft_lifecycle(data: ProviderCredentialsData, draft: ProviderCredentialDraft) {
    let app = expect_context::<crate::state::AppContext>();
    Effect::new(move |_| {
        app.api_base.track();
        app.api_auth_token.track();
        for provider in ["jupiter_swap_v2_keyed", "zeroex_swap_v2", "okx_dex_v6", "lifi",
            "solana_wallet_signer", "evm_wallet_signer", "backpack_stocks"] {
            draft.clear(provider);
        }
    });
    let initial = data.completed.get_untracked().0;
    Effect::new(move |previous: Option<u64>| {
        let (revision, provider) = data.completed.get();
        if revision != previous.unwrap_or(initial) {
            if let Some(provider) = provider { draft.clear(&provider); }
        }
        revision
    });
    on_cleanup(move || {
        let filled = ["jupiter_swap_v2_keyed", "zeroex_swap_v2", "okx_dex_v6", "lifi",
            "solana_wallet_signer", "evm_wallet_signer", "backpack_stocks"]
            .into_iter().filter(|provider| field_keys(provider).iter().any(|key| {
                draft.signal(provider, key).and_then(|value| value.try_get_untracked())
                    .is_some_and(|value| !value.trim().is_empty())
            })).map(str::to_owned).collect::<Vec<_>>();
        data.cleared_drafts.update_untracked(|rows| {
            for provider in filled { if !rows.contains(&provider) { rows.push(provider); } }
        });
    });
}

fn fetch_status(
    client: crate::api::rest::ApiClient,
    state: RwSignal<LoadState<OnchainProviderCredentialsResponse>>,
    reading: RwSignal<bool>,
    version: RwSignal<u64>,
) {
    if reading.get_untracked() {
        return;
    }
    reading.set(true);
    version.update(|value| *value = value.wrapping_add(1));
    let requested = version.get_untracked();
    spawn_local(async move {
        let result = read_with_timeout("读取凭证状态", client.onchain_provider_credentials())
            .await
            .map_err(|error| error.problem);
        if state.is_disposed() || version.get_untracked() != requested {
            return;
        }
        state.update(|current| current.apply_result(result));
        reading.set(false);
    });
}
