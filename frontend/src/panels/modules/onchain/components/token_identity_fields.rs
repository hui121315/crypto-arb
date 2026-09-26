use leptos::prelude::*;
use shared_types::OnchainTokenIdentity;

use super::super::data::{same_token_address, token_address_ready, OnchainData, TokenLeg, TokenResolution};
use super::super::draft::OnchainConfigDraft;

pub(super) fn token_identity_fields(draft: OnchainConfigDraft, data: OnchainData) -> impl IntoView {
    view! {
        <div class="onchain-token-grid">
            {token_field(draft, data, TokenLeg::Base)}
            {token_field(draft, data, TokenLeg::Quote)}
        </div>
    }
}

pub(super) fn identity_apply_problem(
    draft: OnchainConfigDraft,
    data: OnchainData,
) -> Option<String> {
    leg_problem(
        "Base",
        &draft.chain.get(),
        &draft.base_mint.get(),
        &draft.base_token.get(),
        &draft.base_decimals.get(),
        &data.form.base_identity.get(),
    )
    .or_else(|| {
        leg_problem(
            "Quote",
            &draft.chain.get(),
            &draft.quote_mint.get(),
            &draft.quote_token.get(),
            &draft.quote_decimals.get(),
            &data.form.quote_identity.get(),
        )
    })
}

fn token_field(draft: OnchainConfigDraft, data: OnchainData, leg: TokenLeg) -> impl IntoView {
    let (leg_label, address, symbol, decimals, identity_resolved) = match leg {
        TokenLeg::Base => (
            "Base",
            draft.base_mint,
            draft.base_token,
            draft.base_decimals,
            draft.base_identity_resolved,
        ),
        TokenLeg::Quote => (
            "Quote",
            draft.quote_mint,
            draft.quote_token,
            draft.quote_decimals,
            draft.quote_identity_resolved,
        ),
    };
    let input_label = format!("{leg_label} 合约 / Mint");
    let clear_label = format!("清除 {leg_label} 合约");
    let state = data.token_state(leg);
    let revision = data.token_revision(leg);
    view! {
        <div class="onchain-token-field">
            <div class="onchain-token-result" role="status" aria-live="polite">
                <span class="onchain-token-leg">{leg_label}</span>
                <strong>{move || symbol_label(&symbol.get(), &state.get())}</strong>
                <span>{move || decimals_label(&decimals.get())}</span>
                {move || resolution_note(
                    &state.get(),
                    &draft.chain.get(),
                    &address.get(),
                    identity_resolved.get(),
                )}
            </div>
            <div class="onchain-token-address-row">
                <label class="workbench-field">
                    <span>{input_label.clone()}</span>
                    <input
                        aria-label=input_label
                        autocomplete="off"
                        spellcheck="false"
                        class="num"
                        placeholder="粘贴完整地址后自动识别"
                        prop:value=move || address.get()
                        on:input=move |event| {
                            address.set(event_target_value(&event));
                            match leg {
                                TokenLeg::Base => draft.invalidate_base_identity(),
                                TokenLeg::Quote => draft.invalidate_quote_identity(),
                            }
                            revision.update(|revision| *revision = revision.wrapping_add(1));
                            state.set(TokenResolution::Dirty);
                        }
                    />
                </label>
                <button
                    class="onchain-token-resolve"
                    type="button"
                    aria-label=clear_label.clone()
                    title=clear_label
                    disabled=move || address.get().trim().is_empty()
                    on:click=move |_| {
                        revision.update(|revision| *revision = revision.wrapping_add(1));
                        match leg {
                            TokenLeg::Base => draft.clear_base_identity(),
                            TokenLeg::Quote => draft.clear_quote_identity(),
                        }
                        state.set(TokenResolution::Idle);
                    }
                >"×"</button>
            </div>
        </div>
    }
}

fn decimals_label(value: &str) -> String {
    if value.trim().is_empty() {
        "精度未知".to_owned()
    } else {
        format!("{value} 位精度")
    }
}

fn resolution_note(
    state: &TokenResolution,
    chain: &str,
    address: &str,
    identity_resolved: bool,
) -> AnyView {
    match state {
        TokenResolution::Idle if address.trim().is_empty() => {
            view! { <small>"粘贴合约后自动识别"</small> }.into_any()
        }
        TokenResolution::Idle if identity_resolved => {
            view! { <small class="is-positive">"身份已核对"</small> }.into_any()
        }
        TokenResolution::Idle => {
            view! { <small class="is-warning">"当前配置 · 仅原始观察"</small> }.into_any()
        }
        TokenResolution::Dirty if !token_address_ready(chain, address) => view! {
            <small class="is-warning">{incomplete_address_note(chain)}</small>
        }
        .into_any(),
        TokenResolution::Dirty => view! { <small>"即将自动读取币种与精度"</small> }.into_any(),
        TokenResolution::Loading => view! { <small>"正在读取币种与精度…"</small> }.into_any(),
        TokenResolution::Ready(identity) => view! {
            <small class=if identity.verified { "is-positive" } else { "is-warning" }>
                {identity_note(identity)}
            </small>
        }
        .into_any(),
        TokenResolution::PrecisionOnly(resolution) => view! {
            <details class="onchain-token-problem is-warning">
                <summary>{format!("精度 {} 已读取 · 仅原始观察", resolution.decimals)}</summary>
                <span>{resolution.identity_problem.clone().unwrap_or_else(|| {
                    "链上精度已有官方 RPC 数据依据；币种符号尚无可信基础资料，因此不会判断净收益或允许执行。".to_owned()
                })}</span>
            </details>
        }
        .into_any(),
        TokenResolution::Error(problem) => view! {
            <details class="onchain-token-problem">
                <summary>{token_problem_summary(problem)}</summary>
                <span>{problem.clone()}</span>
            </details>
        }
        .into_any(),
    }
}

fn incomplete_address_note(chain: &str) -> &'static str {
    if chain.eq_ignore_ascii_case("solana") {
        "请输入完整 Solana Mint（32–44 位 Base58）"
    } else {
        "请输入完整 EVM 合约地址（0x + 40 位十六进制）"
    }
}

fn symbol_label(value: &str, state: &TokenResolution) -> String {
    if value.trim().is_empty() {
        "符号待核对".to_owned()
    } else if matches!(state, TokenResolution::PrecisionOnly(_)) {
        format!("{}（未核对）", value.trim())
    } else {
        value.trim().to_owned()
    }
}

fn identity_evidence(identity: &OnchainTokenIdentity) -> &'static str {
    if identity.native {
        "原生资产预设"
    } else if identity.source == "jupiter_tokens_v2" && identity.verified {
        "registry 已验证"
    } else if identity.source == "jupiter_tokens_v2" {
        "registry 未验证"
    } else if identity.verified {
        "registry 已验证"
    } else {
        "合约自报基础资料"
    }
}

fn identity_note(identity: &OnchainTokenIdentity) -> String {
    let evidence = identity_evidence(identity);
    let label = identity.name.as_deref().map_or_else(
        || evidence.to_string(),
        |name| format!("{name} · {evidence}"),
    );
    if identity.verified {
        label
    } else {
        format!("{label} · 仅原始观察")
    }
}

fn leg_problem(
    label: &str,
    chain: &str,
    address: &str,
    symbol: &str,
    decimals: &str,
    state: &TokenResolution,
) -> Option<String> {
    if address.trim().is_empty() {
        return Some(format!("请填写 {label} 合约或 Mint"));
    }
    match state {
        TokenResolution::Idle => None,
        TokenResolution::Ready(identity)
            if identity.chain.eq_ignore_ascii_case(chain.trim())
                && same_token_address(chain, &identity.address, address) =>
        {
            let metadata_matches = identity.symbol.eq_ignore_ascii_case(symbol.trim())
                && decimals.trim().parse::<u8>().ok() == Some(identity.decimals);
            (!metadata_matches).then(|| format!("{label} 币种或精度尚未同步"))
        }
        TokenResolution::Ready(_) => Some(format!("{label} 身份与当前合约不一致")),
        TokenResolution::PrecisionOnly(resolution) => {
            let precision_matches = resolution.chain.eq_ignore_ascii_case(chain.trim())
                && same_token_address(chain, &resolution.address, address)
                && decimals.trim().parse::<u8>().ok() == Some(resolution.decimals);
            (!precision_matches).then(|| format!("{label} 精度或合约映射尚未同步"))
        }
        TokenResolution::Dirty => Some(format!("{label} 合约等待自动识别")),
        TokenResolution::Loading => Some(format!("{label} 合约正在自动识别")),
        TokenResolution::Error(_) => Some(format!("{label} 身份未通过")),
    }
}

fn token_problem_summary(problem: &str) -> &'static str {
    let normalized = problem.to_ascii_lowercase();
    if normalized.contains("invalid format") {
        "地址格式无效"
    } else if normalized.contains("onchain_token_identity_mismatch")
        || problem.contains("识别结果与当前链、合约或精度不一致")
    {
        "识别结果不一致 · 未应用"
    } else if problem.contains("超过")
        || normalized.contains("timed out")
        || normalized.contains("timeout")
    {
        "RPC 连接超时 · 自动重试"
    } else if normalized.contains("rpc failed")
        || normalized.contains("http 5")
        || normalized.contains("dns resolution failed")
        || normalized.contains("network")
    {
        "RPC 暂不可用 · 自动重试"
    } else {
        "身份识别失败"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(source: &str, verified: bool) -> OnchainTokenIdentity {
        OnchainTokenIdentity {
            chain: "solana".to_string(),
            address: "mint".to_string(),
            symbol: "TOKEN".to_string(),
            name: None,
            decimals: 6,
            source: source.to_string(),
            evidence_url: "https://dev.jup.ag/docs/token-api/v2".to_string(),
            verified,
            native: false,
            observed_at_ms: 1,
        }
    }

    #[test]
    fn distinguishes_unverified_registry_metadata_from_contract_metadata() {
        assert_eq!(
            identity_evidence(&identity("jupiter_tokens_v2", false)),
            "registry 未验证"
        );
        assert_eq!(
            identity_evidence(&identity("base_public_rpc", false)),
            "合约自报基础资料"
        );
        assert_eq!(
            identity_note(&identity("jupiter_tokens_v2", true)),
            "registry 已验证"
        );
        assert_eq!(
            identity_note(&identity("base_public_rpc", false)),
            "合约自报基础资料 · 仅原始观察"
        );
    }

    #[test]
    fn token_problem_summary_keeps_raw_transport_details_out_of_the_first_layer() {
        assert_eq!(
            token_problem_summary("invalid request: address has an invalid format · HTTP 400"),
            "地址格式无效"
        );
        assert_eq!(
            token_problem_summary("provider timed out"),
            "RPC 连接超时 · 自动重试"
        );
        assert_eq!(
            token_problem_summary("token metadata RPC failed · HTTP 502"),
            "RPC 暂不可用 · 自动重试"
        );
    }

    #[test]
    fn resolved_identity_requires_the_auto_read_decimals_to_match() {
        let identity = identity("base_public_rpc", false);
        assert_eq!(
            leg_problem(
                "Base",
                "solana",
                "mint",
                "TOKEN",
                "6",
                &TokenResolution::Ready(identity.clone()),
            ),
            None
        );
        assert!(leg_problem(
            "Base",
            "solana",
            "mint",
            "TOKEN",
            "18",
            &TokenResolution::Ready(identity),
        )
        .is_some_and(|problem| problem.contains("精度")));
    }

    #[test]
    fn precision_only_state_allows_raw_monitoring_but_keeps_identity_unverified() {
        let resolution = shared_types::OnchainTokenResolution {
            chain: "solana".to_owned(),
            address: "mint".to_owned(),
            decimals: 8,
            precision_source: "solana_mainnet_rpc".to_owned(),
            precision_evidence_url: "https://solana.com/docs/rpc/http/gettokensupply".to_owned(),
            identity: None,
            identity_problem: Some("Jupiter metadata unavailable".to_owned()),
            observed_at_ms: 1,
        };
        let state = TokenResolution::PrecisionOnly(resolution.clone());
        assert_eq!(symbol_label("PUPS", &state), "PUPS（未核对）");
        assert_eq!(leg_problem("Base", "solana", "mint", "PUPS", "8", &state), None);
    }

    #[test]
    fn incomplete_address_note_matches_the_selected_chain() {
        assert_eq!(
            incomplete_address_note("solana"),
            "请输入完整 Solana Mint（32–44 位 Base58）"
        );
        assert_eq!(
            incomplete_address_note("base"),
            "请输入完整 EVM 合约地址（0x + 40 位十六进制）"
        );
    }
}
