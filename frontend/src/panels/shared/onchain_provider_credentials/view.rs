use leptos::prelude::*;

use super::components::{provider_form, provider_panel_id, provider_selector, provider_tab_id};
use super::data::{
    use_provider_credentials_data, ProviderCredentialDraft, ProviderCredentialsData,
};
use super::status::{
    is_configurable_provider, load_problem, selected_readiness_class, selected_readiness_label,
    storage_note,
};

pub(crate) fn onchain_provider_credentials_editor(
    selected_provider: RwSignal<String>,
    selectable: bool,
    compact: bool,
) -> impl IntoView {
    let data = use_provider_credentials_data();
    let draft = ProviderCredentialDraft::new();
    let clear_armed = RwSignal::new(false);
    Effect::new(move |_| {
        selected_provider.get();
        draft.clear("jupiter_swap_v2_keyed");
        draft.clear("zeroex_swap_v2");
        draft.clear("okx_dex_v6");
        draft.clear("lifi");
        draft.clear("solana_wallet_signer");
        draft.clear("evm_wallet_signer");
        draft.clear("backpack_stocks");
        clear_armed.set(false);
        data.feedback.set(None);
        data.problem.set(None);
    });
    Effect::new(move |_| {
        if let (_, Some(provider)) = data.completed.get() {
            draft.clear(&provider);
        }
    });

    if compact {
        view! {
            <details class="provider-credentials provider-credentials-compact">
                <summary>
                    <span>{move || credential_group_label(&selected_provider.get())}</span>
                    <strong class=move || selected_readiness_class(data, selected_provider)>
                        {move || selected_readiness_label(data, selected_provider)}
                    </strong>
                </summary>
                {credential_editor_body(selected_provider, selectable, data, draft, clear_armed)}
            </details>
        }
        .into_any()
    } else {
        view! {
            <section class="provider-credentials" aria-label="链上 API 与钱包签名凭证">
                <header class="provider-credentials-header">
                    <div>
                        <strong>"链上 API、股票 RFQ 与钱包签名器"</strong>
                        <span>"Provider Key 与 Solana/EVM 私钥仅保存在后端安全存储；页面不回填密钥。"</span>
                    </div>
                    <strong class=move || selected_readiness_class(data, selected_provider)>
                        {move || selected_readiness_label(data, selected_provider)}
                    </strong>
                </header>
                {credential_editor_body(selected_provider, selectable, data, draft, clear_armed)}
            </section>
        }
        .into_any()
    }
}

pub(crate) fn onchain_access_credentials_editor(
    signer_provider: RwSignal<String>,
    quote_provider: RwSignal<String>,
    bridge_provider: RwSignal<String>,
    bridge_enabled: RwSignal<bool>,
) -> impl IntoView {
    let data = use_provider_credentials_data();
    Effect::new(move |_| {
        signer_provider.get();
        quote_provider.get();
        bridge_provider.get();
        bridge_enabled.get();
        data.feedback.set(None);
        data.problem.set(None);
    });

    view! {
        <div class="onchain-access-credentials">
            {access_credential_group("02", "当前链签名器", signer_provider, data)}
            {access_credential_group("03", "报价 API", quote_provider, data)}
            <div hidden=move || !bridge_enabled.get()>
                {access_credential_group("04", "跨链路由", bridge_provider, data)}
            </div>
            {move || storage_note(data)}
            {move || data.feedback.get().map(|message| view! {
                <div class="provider-credentials-feedback is-positive" role="status">{message}</div>
            })}
            {move || data.problem.get().map(|message| view! {
                <div class="provider-credentials-feedback is-danger" role="alert">{message}</div>
            })}
            {move || load_problem(data)}
        </div>
    }
}

fn access_credential_group(
    step: &'static str,
    role: &'static str,
    selected_provider: RwSignal<String>,
    data: ProviderCredentialsData,
) -> impl IntoView {
    let draft = ProviderCredentialDraft::new();
    let clear_armed = RwSignal::new(false);
    Effect::new(move |_| {
        let provider = selected_provider.get();
        draft.clear(&provider);
        clear_armed.set(false);
    });
    Effect::new(move |_| {
        if let (_, Some(provider)) = data.completed.get() {
            if selected_provider.get_untracked() == provider {
                draft.clear(&provider);
            }
        }
    });

    view! {
        <details class="provider-credentials provider-credentials-compact onchain-access-credential">
            <summary>
                <span>
                    <small>{format!("{step} · {role}")}</small>
                    <strong>{move || credential_instance_label(&selected_provider.get())}</strong>
                </span>
                <strong class=move || selected_readiness_class(data, selected_provider)>
                    {move || selected_readiness_label(data, selected_provider)}
                </strong>
            </summary>
            {credential_editor_panel(selected_provider, false, data, draft, clear_armed)}
        </details>
    }
}

fn credential_instance_label(provider: &str) -> &'static str {
    match provider {
        "jupiter_swap_v2_keyed" => "Jupiter API Key",
        "jupiter_swap_v2" => "Jupiter Keyless",
        "zeroex_swap_v2" => "0x API Key",
        "okx_dex_v6" => "OKX DEX 凭证",
        "lifi" => "LI.FI API Key（可选）",
        "solana_wallet_signer" => "Solana 私钥签名",
        "evm_wallet_signer" => "EVM 私钥签名",
        "backpack_stocks" => "Backpack 股票 RFQ",
        _ => "当前 Provider",
    }
}

fn credential_group_label(provider: &str) -> &'static str {
    match provider {
        "solana_wallet_signer" => "Solana 钱包签名器",
        "evm_wallet_signer" => "EVM 钱包签名器",
        "backpack_stocks" => "Backpack 股票 RFQ 凭证",
        _ => "报价 Provider 凭证",
    }
}

fn credential_editor_body(
    selected_provider: RwSignal<String>,
    selectable: bool,
    data: ProviderCredentialsData,
    draft: ProviderCredentialDraft,
    clear_armed: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div class="provider-credentials-body">
            {credential_editor_panel(selected_provider, selectable, data, draft, clear_armed)}
            {move || is_configurable_provider(&selected_provider.get()).then(|| storage_note(data))}
            {move || data.feedback.get().map(|message| view! {
                <div class="provider-credentials-feedback is-positive" role="status">{message}</div>
            })}
            {move || data.problem.get().map(|message| view! {
                <div class="provider-credentials-feedback is-danger" role="alert">{message}</div>
            })}
            {move || {
                if is_configurable_provider(&selected_provider.get()) {
                    load_problem(data)
                } else {
                    None
                }
            }}
        </div>
    }
}

fn credential_editor_panel(
    selected_provider: RwSignal<String>,
    selectable: bool,
    data: ProviderCredentialsData,
    draft: ProviderCredentialDraft,
    clear_armed: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div class="provider-credentials-editor-panel">
            {selectable.then(|| provider_selector(selected_provider, data.busy))}
            <div
                class="provider-credentials-panel"
                role=selectable.then_some("tabpanel")
                id=move || selectable.then(|| provider_panel_id(&selected_provider.get()))
                aria-labelledby=move || selectable.then(|| provider_tab_id(&selected_provider.get()))
                tabindex=selectable.then_some(0_i32)
            >
                {move || {
                    let provider = selected_provider.get();
                    match provider.as_str() {
                        "jupiter_swap_v2_keyed"
                        | "zeroex_swap_v2"
                        | "okx_dex_v6"
                        | "lifi"
                        | "solana_wallet_signer"
                        | "backpack_stocks"
                        | "evm_wallet_signer" => provider_form(
                            &provider,
                            data,
                            draft,
                            clear_armed,
                        ).into_any(),
                        _ => view! {
                            <div class="provider-credentials-keyless">
                                <strong>"当前 Provider 无必填凭证"</strong>
                                <span>{keyless_provider_note(&provider)}</span>
                            </div>
                        }.into_any(),
                    }
                }}
            </div>
            <div class="provider-credentials-refresh">
                <button type="button" class="row-action" disabled=move || data.busy.get() || data.reading.get()
                    on:click=move |_| data.reload.run(())>
                    {move || if data.reading.get() { "读取中…" } else { "刷新凭证状态" }}
                </button>
            </div>
        </div>
    }
}

fn keyless_provider_note(provider: &str) -> &'static str {
    match provider {
        "cow_protocol" => {
            "CoW 公共 API 按官方 SDK 的每 IP 5 RPS 上限有界读取；Fast Quote 不签名、不下单。"
        }
        _ => "Jupiter Keyless 路由不会发送已保存的 API Key，双向询价约每 4.5 秒一轮。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_editor_names_signers_separately_from_quote_credentials() {
        assert_eq!(
            credential_group_label("solana_wallet_signer"),
            "Solana 钱包签名器"
        );
        assert_eq!(
            credential_group_label("evm_wallet_signer"),
            "EVM 钱包签名器"
        );
        assert_eq!(
            credential_group_label("jupiter_swap_v2_keyed"),
            "报价 Provider 凭证"
        );
        assert_eq!(
            credential_instance_label("solana_wallet_signer"),
            "Solana 私钥签名"
        );
        assert_eq!(
            credential_instance_label("jupiter_swap_v2_keyed"),
            "Jupiter API Key"
        );
    }
}
