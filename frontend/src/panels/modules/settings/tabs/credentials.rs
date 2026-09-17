use leptos::prelude::*;

use crate::panels::shared::onchain_provider_credentials_editor;
use crate::state::module_runtime::{normalize_choice, store_choice, stored_choice};

use super::venue_credentials_matrix;

const CREDENTIAL_TASK_STORAGE_KEY: &str = "crossline.settings.credentialTask";
const CREDENTIAL_PROVIDER_STORAGE_KEY: &str = "crossline.settings.credentialProvider";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CredentialTask {
    #[default]
    Exchange,
    OnchainProvider,
}

impl CredentialTask {
    const fn slug(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::OnchainProvider => "onchain-provider",
        }
    }

    fn from_slug(value: &str) -> Option<Self> {
        Some(match normalize_choice(value) {
            "exchange" => Self::Exchange,
            "onchain-provider" => Self::OnchainProvider,
            _ => return None,
        })
    }

    const fn tab_id(self) -> &'static str {
        match self {
            Self::Exchange => "settings-credentials-tab-exchange",
            Self::OnchainProvider => "settings-credentials-tab-onchain-provider",
        }
    }

    const fn panel_id(self) -> &'static str {
        match self {
            Self::Exchange => "settings-credentials-panel-exchange",
            Self::OnchainProvider => "settings-credentials-panel-onchain-provider",
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Exchange => Self::OnchainProvider,
            Self::OnchainProvider => Self::Exchange,
        }
    }
}

pub(in crate::panels::modules::settings) fn credentials_tab() -> impl IntoView {
    let refresh_nonce = RwSignal::new(0_u64);
    let onchain_provider = RwSignal::new(
        stored_choice(CREDENTIAL_PROVIDER_STORAGE_KEY, configurable_provider)
            .unwrap_or_else(|| "jupiter_swap_v2_keyed".to_owned()),
    );
    let active = RwSignal::new(
        stored_choice(CREDENTIAL_TASK_STORAGE_KEY, CredentialTask::from_slug).unwrap_or_default(),
    );
    let exchange_ref = NodeRef::<leptos::html::Button>::new();
    let onchain_provider_ref = NodeRef::<leptos::html::Button>::new();

    Effect::new(move |_| {
        store_choice(CREDENTIAL_TASK_STORAGE_KEY, active.get().slug());
    });
    Effect::new(move |_| {
        store_choice(CREDENTIAL_PROVIDER_STORAGE_KEY, &onchain_provider.get());
    });

    view! {
        <div class="settings-stack">
            <div
                class="settings-task-tabs"
                role="tablist"
                aria-label="凭证类型"
                aria-orientation="horizontal"
                on:keydown=move |event| {
                    let next = match event.key().as_str() {
                        "ArrowRight" | "ArrowLeft" => Some(active.get().next()),
                        "Home" => Some(CredentialTask::Exchange),
                        "End" => Some(CredentialTask::OnchainProvider),
                        _ => None,
                    };
                    let Some(next) = next else { return };
                    event.prevent_default();
                    active.set(next);
                    let target = match next {
                        CredentialTask::Exchange => exchange_ref,
                        CredentialTask::OnchainProvider => onchain_provider_ref,
                    };
                    if let Some(button) = target.get() {
                        let _ = button.focus();
                    }
                }
            >
                {credential_task_tab("交易所账户", CredentialTask::Exchange, active, exchange_ref)}
                {credential_task_tab(
                    "链上凭证",
                    CredentialTask::OnchainProvider,
                    active,
                    onchain_provider_ref,
                )}
            </div>
            <section
                class="settings-task-panel"
                role="tabpanel"
                id=CredentialTask::Exchange.panel_id()
                aria-labelledby=CredentialTask::Exchange.tab_id()
                tabindex="0"
                aria-label="交易所账户凭证"
                hidden=move || active.get() != CredentialTask::Exchange
            >
                {venue_credentials_matrix(refresh_nonce)}
            </section>
            <section
                class="settings-task-panel"
                role="tabpanel"
                id=CredentialTask::OnchainProvider.panel_id()
                aria-labelledby=CredentialTask::OnchainProvider.tab_id()
                tabindex="0"
                aria-label="链上 API 与钱包签名凭证"
                hidden=move || active.get() != CredentialTask::OnchainProvider
            >
                {onchain_provider_credentials_editor(onchain_provider, true, false)}
            </section>
        </div>
    }
}

fn configurable_provider(value: &str) -> Option<String> {
    let provider = normalize_choice(value);
    matches!(
        provider,
        "jupiter_swap_v2_keyed"
            | "zeroex_swap_v2"
            | "okx_dex_v6"
            | "solana_wallet_signer"
            | "evm_wallet_signer"
    )
    .then(|| provider.to_owned())
}

fn credential_task_tab(
    label: &'static str,
    task: CredentialTask,
    active: RwSignal<CredentialTask>,
    node_ref: NodeRef<leptos::html::Button>,
) -> impl IntoView {
    view! {
        <button
            node_ref=node_ref
            type="button"
            id=task.tab_id()
            role="tab"
            aria-controls=task.panel_id()
            aria-selected=move || (active.get() == task).to_string()
            tabindex=move || if active.get() == task { 0 } else { -1 }
            class=move || if active.get() == task { "active" } else { "" }
            on:click=move |_| active.set(task)
        >
            {label}
        </button>
    }
}
