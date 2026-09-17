use leptos::prelude::*;
use shared_types::{VenueCredentialField, VenueCredentialFieldSource};

use super::data::{ProviderCredentialDraft, ProviderCredentialsData};
use super::status::{current_status_for, status_line};

const CONFIGURABLE_PROVIDERS: [(&str, &str); 7] = [
    ("jupiter_swap_v2_keyed", "Jupiter"),
    ("zeroex_swap_v2", "0x"),
    ("okx_dex_v6", "OKX DEX"),
    ("lifi", "LI.FI"),
    ("solana_wallet_signer", "Solana 钱包"),
    ("evm_wallet_signer", "EVM 钱包"),
    ("backpack_stocks", "Backpack"),
];

pub(super) fn provider_selector(selected_provider: RwSignal<String>) -> impl IntoView {
    let jupiter_ref = NodeRef::<leptos::html::Button>::new();
    let zeroex_ref = NodeRef::<leptos::html::Button>::new();
    let okx_ref = NodeRef::<leptos::html::Button>::new();
    let lifi_ref = NodeRef::<leptos::html::Button>::new();
    let solana_ref = NodeRef::<leptos::html::Button>::new();
    let evm_ref = NodeRef::<leptos::html::Button>::new();
    let backpack_ref = NodeRef::<leptos::html::Button>::new();

    view! {
        <div
            class="provider-credentials-segment"
            role="tablist"
            aria-label="链上报价 Provider"
            aria-orientation="horizontal"
            on:keydown=move |event| {
                let current = selected_provider.get();
                let next = match event.key().as_str() {
                    "ArrowRight" => Some(next_provider(&current)),
                    "ArrowLeft" => Some(previous_provider(&current)),
                    "Home" => Some(CONFIGURABLE_PROVIDERS[0].0),
                    "End" => Some(CONFIGURABLE_PROVIDERS[6].0),
                    _ => None,
                };
                let Some(next) = next else { return };
                event.prevent_default();
                selected_provider.set(next.to_owned());
                let target = match next {
                    "jupiter_swap_v2_keyed" => jupiter_ref,
                    "zeroex_swap_v2" => zeroex_ref,
                    "okx_dex_v6" => okx_ref,
                    "lifi" => lifi_ref,
                    "solana_wallet_signer" => solana_ref,
                    "backpack_stocks" => backpack_ref,
                    _ => evm_ref,
                };
                if let Some(button) = target.get() {
                    let _ = button.focus();
                }
            }
        >
            {provider_tab(
                "jupiter_swap_v2_keyed",
                "Jupiter",
                selected_provider,
                jupiter_ref,
            )}
            {provider_tab(
                "zeroex_swap_v2",
                "0x",
                selected_provider,
                zeroex_ref,
            )}
            {provider_tab(
                "okx_dex_v6",
                "OKX DEX",
                selected_provider,
                okx_ref,
            )}
            {provider_tab(
                "lifi",
                "LI.FI",
                selected_provider,
                lifi_ref,
            )}
            {provider_tab(
                "solana_wallet_signer",
                "Solana 钱包",
                selected_provider,
                solana_ref,
            )}
            {provider_tab(
                "evm_wallet_signer",
                "EVM 钱包",
                selected_provider,
                evm_ref,
            )}
            {provider_tab("backpack_stocks","Backpack",selected_provider,backpack_ref)}
        </div>
    }
}

fn provider_tab(
    provider: &'static str,
    label: &'static str,
    selected_provider: RwSignal<String>,
    node_ref: NodeRef<leptos::html::Button>,
) -> impl IntoView {
    view! {
        <button
            node_ref=node_ref
            type="button"
            id=provider_tab_id(provider)
            role="tab"
            aria-controls=provider_panel_id(provider)
            aria-selected=move || (selected_provider.get() == provider).to_string()
            tabindex=move || if selected_provider.get() == provider { 0 } else { -1 }
            class:active=move || selected_provider.get() == provider
            on:click=move |_| selected_provider.set(provider.to_owned())
        >
            {label}
        </button>
    }
}

fn next_provider(provider: &str) -> &'static str {
    match provider {
        "jupiter_swap_v2_keyed" => "zeroex_swap_v2",
        "zeroex_swap_v2" => "okx_dex_v6",
        "okx_dex_v6" => "lifi",
        "lifi" => "solana_wallet_signer",
        "solana_wallet_signer" => "evm_wallet_signer",
        "evm_wallet_signer" => "backpack_stocks",
        _ => "jupiter_swap_v2_keyed",
    }
}

fn previous_provider(provider: &str) -> &'static str {
    match provider {
        "jupiter_swap_v2_keyed" => "backpack_stocks",
        "backpack_stocks" => "evm_wallet_signer",
        "zeroex_swap_v2" => "jupiter_swap_v2_keyed",
        "okx_dex_v6" => "zeroex_swap_v2",
        "lifi" => "okx_dex_v6",
        "solana_wallet_signer" => "lifi",
        _ => "solana_wallet_signer",
    }
}

pub(super) fn provider_tab_id(provider: &str) -> &'static str {
    match provider {
        "jupiter_swap_v2_keyed" => "provider-credentials-tab-jupiter",
        "zeroex_swap_v2" => "provider-credentials-tab-zeroex",
        "okx_dex_v6" => "provider-credentials-tab-okx",
        "lifi" => "provider-credentials-tab-lifi",
        "solana_wallet_signer" => "provider-credentials-tab-solana-signer",
        "backpack_stocks" => "provider-credentials-tab-backpack",
        _ => "provider-credentials-tab-evm-signer",
    }
}

pub(super) fn provider_panel_id(provider: &str) -> &'static str {
    match provider {
        "jupiter_swap_v2_keyed" => "provider-credentials-panel-jupiter",
        "zeroex_swap_v2" => "provider-credentials-panel-zeroex",
        "okx_dex_v6" => "provider-credentials-panel-okx",
        "lifi" => "provider-credentials-panel-lifi",
        "solana_wallet_signer" => "provider-credentials-panel-solana-signer",
        "backpack_stocks" => "provider-credentials-panel-backpack",
        _ => "provider-credentials-panel-evm-signer",
    }
}

pub(super) fn provider_form(
    provider: &str,
    data: ProviderCredentialsData,
    draft: ProviderCredentialDraft,
    clear_armed: RwSignal<bool>,
) -> impl IntoView {
    let status = current_status_for(&data.state.get(), provider);
    let fields = status
        .as_ref()
        .map(|status| status.fields.clone())
        .unwrap_or_default();
    let provider_for_save_gate = provider.to_owned();
    let provider_for_save = provider.to_owned();
    let provider_for_clear = provider.to_owned();
    let provider_for_values = provider.to_owned();
    let docs_url = status
        .as_ref()
        .map(|status| status.official_docs_url.clone());
    let configured = status
        .as_ref()
        .is_some_and(|status| status.configured_count > 0);
    view! {
        <div class="provider-credentials-form">
            <div class="provider-credentials-fields">
                {fields.into_iter().filter_map(|field| {
                    let signal = draft.signal(provider, &field.key)?;
                    Some(credential_field(field, signal))
                }).collect_view()}
            </div>
            <div class="provider-credentials-statusline">
                {move || status_line(&data.state.get(), &provider_for_values)}
                {docs_url.map(|url| view! {
                    <a href=url target="_blank" rel="noreferrer">"官方凭证文档 ↗"</a>
                })}
            </div>
            <div class="provider-credentials-actions">
                <button
                    type="button"
                    class="workbench-primary"
                    disabled=move || data.busy.get() || !draft.has_values(&provider_for_save_gate)
                    on:click=move |_| {
                        let values = draft.values(&provider_for_save);
                        data.save.run((provider_for_save.clone(), values));
                        clear_armed.set(false);
                    }
                >
                    {move || if data.busy.get() { "保存中…" } else { "保存新凭证" }}
                </button>
                <button
                    type="button"
                    class="provider-credentials-clear"
                    disabled=move || data.busy.get() || !configured
                    on:click=move |_| {
                        if clear_armed.get_untracked() {
                            data.clear.run(provider_for_clear.clone());
                            clear_armed.set(false);
                        } else {
                            clear_armed.set(true);
                        }
                    }
                >
                    {move || if clear_armed.get() { "再次点击确认清除" } else { "清除当前凭证" }}
                </button>
            </div>
        </div>
    }
}

fn credential_field(field: VenueCredentialField, value: RwSignal<String>) -> impl IntoView {
    let source = source_label(field.source);
    view! {
        <label class="provider-credential-field">
            <span>
                <strong>{field.label}</strong>
                <small>{if field.configured { format!("已配置 · {source}") } else { "未配置".to_owned() }}</small>
            </span>
            <input
                type="password"
                autocomplete="new-password"
                spellcheck="false"
                placeholder=field.env_key
                bind:value=value
            />
        </label>
    }
}

const fn source_label(source: VenueCredentialFieldSource) -> &'static str {
    match source {
        VenueCredentialFieldSource::Missing => "未配置",
        VenueCredentialFieldSource::Environment => "环境变量",
        VenueCredentialFieldSource::EnvFile => ".env",
        VenueCredentialFieldSource::Keychain => "Keychain",
        VenueCredentialFieldSource::Runtime => "运行态",
    }
}
