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

pub(super) fn provider_selector(
    selected_provider: RwSignal<String>,
    busy: RwSignal<bool>,
) -> impl IntoView {
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
            aria-label="链上报价 报价服务"
            aria-orientation="horizontal"
            on:keydown=move |event| {
                if busy.get_untracked() { return; }
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
                busy,
            )}
            {provider_tab(
                "zeroex_swap_v2",
                "0x",
                selected_provider,
                zeroex_ref,
                busy,
            )}
            {provider_tab(
                "okx_dex_v6",
                "OKX DEX",
                selected_provider,
                okx_ref,
                busy,
            )}
            {provider_tab(
                "lifi",
                "LI.FI",
                selected_provider,
                lifi_ref,
                busy,
            )}
            {provider_tab(
                "solana_wallet_signer",
                "Solana 钱包",
                selected_provider,
                solana_ref,
                busy,
            )}
            {provider_tab(
                "evm_wallet_signer",
                "EVM 钱包",
                selected_provider,
                evm_ref,
                busy,
            )}
            {provider_tab("backpack_stocks","Backpack",selected_provider,backpack_ref,busy)}
        </div>
    }
}

fn provider_tab(
    provider: &'static str,
    label: &'static str,
    selected_provider: RwSignal<String>,
    node_ref: NodeRef<leptos::html::Button>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <button
            node_ref=node_ref
            type="button"
            disabled=move || busy.get()
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
    let provider = provider.to_owned();
    let status_provider = provider.clone();
    let status = Memo::new(move |_| current_status_for(&data.state.get(), &status_provider));
    let fields = Memo::new(move |_| status.get().map(|s| s.fields).unwrap_or_default());
    let ready = Memo::new(move |_| {
        matches!(
            data.state.get(),
            crate::state::load_state::LoadState::Ready(_)
        ) && status.get().is_some()
    });
    let provider_for_save_gate = provider.to_owned();
    let provider_for_save = provider.to_owned();
    let provider_for_clear = provider.to_owned();
    let provider_for_values = provider.to_owned();
    let configured = Memo::new(move |_| {
        status
            .get()
            .is_some_and(|status| status.configured_count > 0)
    });
    view! {
        <div class="provider-credentials-form">
            <fieldset class="provider-credentials-fields" disabled=move || data.locked() || !ready.get()>
                <For each=move || fields.get() key=|field| field.key.clone() children=move |initial| {
                    let value = draft.signal(&provider, &initial.key);
                    let field = Memo::new(move |_| fields.with(|fields| fields.iter().find(|field| field.key == initial.key).cloned().unwrap_or_else(|| initial.clone())));
                    value.map(|value| credential_field(field, value))
                }/>
            </fieldset>
            <div class="provider-credentials-statusline">
                {move || status_line(&data.state.get(), &provider_for_values)}
                {move || status.get().map(|status| view! {
                    <a href=status.official_docs_url target="_blank" rel="noreferrer">"官方凭证文档 ↗"</a>
                })}
            </div>
            <div class="provider-credentials-actions">
                <button
                    type="button"
                    class="workbench-primary"
                    disabled=move || data.locked() || data.reading.get() || !ready.get() || !draft.has_values(&provider_for_save_gate)
                    on:click=move |_| {
                        let values = draft.values(&provider_for_save);
                        data.save.run((provider_for_save.clone(), values));
                        clear_armed.set(false);
                    }
                >
                    {move || if data.busy.get() { "处理中…" } else { "保存新凭证" }}
                </button>
                <button
                    type="button"
                    class="provider-credentials-clear"
                    disabled=move || data.locked() || data.reading.get() || !ready.get() || !configured.get()
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

fn credential_field(field: Memo<VenueCredentialField>, value: RwSignal<String>) -> impl IntoView {
    view! {
        <label class="provider-credential-field">
            <span>
                <strong>{move || field.get().label}</strong>
                <small>{move || { let field = field.get(); if field.configured { format!("已配置 · {}", source_label(field.source)) } else { "未配置".to_owned() } }}</small>
            </span>
            <input
                type="password"
                autocomplete="new-password"
                spellcheck="false"
                placeholder=move || field.get().env_key
                prop:value=move || value.get()
                on:input=move |event| value.set(event_target_value(&event))
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
        VenueCredentialFieldSource::Runtime => "运行状态",
    }
}
