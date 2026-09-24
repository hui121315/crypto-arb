use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{OnchainProviderCredentialsResponse, VenueCredentialValue};

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
    pub reload: Callback<()>,
    pub save: Callback<(String, Vec<VenueCredentialValue>)>,
    pub clear: Callback<String>,
}

pub(super) fn use_provider_credentials_data() -> ProviderCredentialsData {
    let client = use_global().client;
    let state = RwSignal::new(LoadState::Loading);
    let busy = RwSignal::new(false);
    let reading = RwSignal::new(false);
    let version = RwSignal::new(0_u64);
    let completed = RwSignal::new((0_u64, None));
    let feedback = RwSignal::new(None);
    let problem = RwSignal::new(None);
    fetch_status(client.clone(), state, reading, version);

    let reload = Callback::new({
        let client = client.clone();
        move |()| {
            if busy.get_untracked() || reading.get_untracked() {
                return;
            }
            fetch_status(client.clone(), state, reading, version);
        }
    });
    let save = Callback::new({
        let client = client.clone();
        move |(provider, fields): (String, Vec<VenueCredentialValue>)| {
            if busy.get_untracked()
                || reading.get_untracked()
                || fields.is_empty()
                || !matches!(state.get_untracked(), LoadState::Ready(_))
            {
                return;
            }
            let client = client.clone();
            busy.set(true);
            feedback.set(None);
            problem.set(None);
            spawn_local(async move {
                let result = client
                    .save_onchain_provider_credentials(&provider, fields)
                    .await;
                if state.is_disposed() {
                    return;
                }
                match result {
                    Ok(response) => {
                        feedback.set(Some(format!("{}：{}", response.label, response.message)));
                        completed
                            .update(|value| *value = (value.0.wrapping_add(1), Some(provider)));
                        fetch_status(client, state, reading, version);
                    }
                    Err(error) => problem.set(Some(error.to_string())),
                }
                busy.set(false);
            });
        }
    });
    let clear = Callback::new(move |provider: String| {
        if busy.get_untracked()
            || reading.get_untracked()
            || !matches!(state.get_untracked(), LoadState::Ready(_))
        {
            return;
        }
        let client = client.clone();
        busy.set(true);
        feedback.set(None);
        problem.set(None);
        spawn_local(async move {
            let result = client
                .clear_onchain_provider_credentials(&provider, Vec::new())
                .await;
            if state.is_disposed() {
                return;
            }
            match result {
                Ok(response) => {
                    feedback.set(Some(format!("{}：{}", response.label, response.message)));
                    completed.update(|value| *value = (value.0.wrapping_add(1), Some(provider)));
                    fetch_status(client, state, reading, version);
                }
                Err(error) => problem.set(Some(error.to_string())),
            }
            busy.set(false);
        });
    });

    ProviderCredentialsData {
        state,
        busy,
        reading,
        completed,
        feedback,
        problem,
        reload,
        save,
        clear,
    }
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
        let result = client
            .onchain_provider_credentials()
            .await
            .map_err(|error| error.problem);
        if state.is_disposed() || version.get_untracked() != requested {
            return;
        }
        state.update(|current| current.apply_result(result));
        reading.set(false);
    });
}
