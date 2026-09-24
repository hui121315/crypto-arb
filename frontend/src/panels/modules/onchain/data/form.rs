use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::use_debounced_value;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ApiProblem, OnchainCexPairCatalog, OnchainRpcMode, OnchainTokenIdentityRequest,
    OnchainTokenResolution,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::super::draft::{explicit_pair_assets, normalized_asset, OnchainConfigDraft};
use super::{OnchainData, TokenLeg, TokenResolution, TokenResolveCommand};

const CEX_PAIR_REQUEST_DEDUPE_MS: u64 = 1_000;
const CEX_PAIR_RETRY_BASE_MS: u32 = 1_500;
const CEX_PAIR_RETRY_MAX_MS: u32 = 12_000;
const TOKEN_IDENTITY_REFRESH_MS: u32 = 30_000;
const TOKEN_IDENTITY_ERROR_RETRY_MS: u32 = 15_000;

pub(in crate::panels::modules::onchain) fn use_onchain_form_data(
    draft: OnchainConfigDraft,
    data: OnchainData,
) {
    use_auto_token_resolution(draft, data, TokenLeg::Base);
    use_auto_token_resolution(draft, data, TokenLeg::Quote);
    use_cex_pair_catalog(draft, data);
}

fn use_auto_token_resolution(draft: OnchainConfigDraft, data: OnchainData, leg: TokenLeg) {
    let address = match leg {
        TokenLeg::Base => draft.base_mint,
        TokenLeg::Quote => draft.quote_mint,
    };
    let identity_resolved = match leg {
        TokenLeg::Base => draft.base_identity_resolved,
        TokenLeg::Quote => draft.quote_identity_resolved,
    };
    let state = data.token_state(leg);
    let revision = data.token_revision(leg);
    let debounced = use_debounced_value(
        move || (revision.get(), address.get()),
        Duration::from_millis(450),
    );
    let active = Arc::new(AtomicBool::new(true));
    let cleanup_active = Arc::clone(&active);
    let resolver_active = Arc::clone(&active);
    on_cleanup(move || cleanup_active.store(false, Ordering::Release));
    let on_resolved = Callback::new(move |resolution: OnchainTokenResolution| match leg {
        TokenLeg::Base => match resolution.identity.as_ref() {
            Some(identity) => draft.apply_base_identity(identity),
            None => draft.apply_base_precision(&resolution.address, resolution.decimals),
        },
        TokenLeg::Quote => match resolution.identity.as_ref() {
            Some(identity) => draft.apply_quote_identity(identity),
            None => draft.apply_quote_precision(&resolution.address, resolution.decimals),
        },
    });
    Effect::new(move |_| {
        let Some((_, address_value)) = debounced.get() else {
            return;
        };
        let chain = draft.chain.get();
        if address_value.trim().is_empty() {
            state.set(TokenResolution::Idle);
            return;
        }
        if matches!(state.get_untracked(), TokenResolution::Idle)
            && !identity_resolved.get_untracked()
            && token_address_ready(&chain, &address_value)
        {
            state.set(TokenResolution::Dirty);
        }
        if !should_resolve_token(&state.get_untracked(), &chain, &address_value) {
            return;
        }
        data.form.resolve_token.run(TokenResolveCommand {
            leg,
            request: token_identity_request(draft, chain, address_value),
            current_chain: draft.chain,
            current_address: address,
            on_resolved,
            active: Arc::clone(&resolver_active),
        });
    });
    use_partial_identity_refresh(address, state, revision, Arc::clone(&active));
    use_identity_error_refresh(address, state, revision, active);
}

fn use_identity_error_refresh(
    address: RwSignal<String>,
    state: RwSignal<TokenResolution>,
    revision: RwSignal<u64>,
    active: Arc<AtomicBool>,
) {
    let scheduled = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let TokenResolution::Error(problem) = state.get() else {
            return;
        };
        let address_key = address.get_untracked().trim().to_owned();
        let key = format!("{address_key}\n{problem}");
        if scheduled.get_untracked().as_deref() == Some(key.as_str()) {
            return;
        }
        scheduled.set(Some(key.clone()));
        let active = Arc::clone(&active);
        spawn_local(async move {
            TimeoutFuture::new(TOKEN_IDENTITY_ERROR_RETRY_MS).await;
            if !active.load(Ordering::Acquire) { return; }
            let still_failed = matches!(
                state.get_untracked(),
                TokenResolution::Error(current) if current == problem
            );
            if active.load(Ordering::Acquire)
                && address.get_untracked().trim() == address_key
                && still_failed
            {
                state.set(TokenResolution::Dirty);
                revision.update(|revision| *revision = revision.wrapping_add(1));
            }
            if scheduled.get_untracked().as_deref() == Some(key.as_str()) {
                scheduled.set(None);
            }
        });
    });
}

fn use_partial_identity_refresh(
    address: RwSignal<String>,
    state: RwSignal<TokenResolution>,
    revision: RwSignal<u64>,
    active: Arc<AtomicBool>,
) {
    let scheduled = RwSignal::new(None::<(String, i64)>);
    Effect::new(move |_| {
        let TokenResolution::PrecisionOnly(resolution) = state.get() else {
            return;
        };
        let key = (resolution.address.clone(), resolution.observed_at_ms);
        if scheduled.get_untracked().as_ref() == Some(&key) {
            return;
        }
        scheduled.set(Some(key.clone()));
        let active = Arc::clone(&active);
        spawn_local(async move {
            TimeoutFuture::new(TOKEN_IDENTITY_REFRESH_MS).await;
            if !active.load(Ordering::Acquire) { return; }
            let still_partial = matches!(
                state.get_untracked(),
                TokenResolution::PrecisionOnly(current)
                    if current.address == key.0 && current.observed_at_ms == key.1
            );
            if active.load(Ordering::Acquire)
                && address.get_untracked().trim() == key.0
                && still_partial
            {
                state.set(TokenResolution::Dirty);
                revision.update(|revision| *revision = revision.wrapping_add(1));
            }
            if scheduled.get_untracked().as_ref() == Some(&key) {
                scheduled.set(None);
            }
        });
    });
}

fn token_identity_request(
    draft: OnchainConfigDraft,
    chain: String,
    address: String,
) -> OnchainTokenIdentityRequest {
    OnchainTokenIdentityRequest {
        chain,
        address,
        custom_rpc_url: (draft.rpc_mode.get_untracked() == OnchainRpcMode::Custom)
            .then(|| draft.custom_rpc_url.get_untracked())
            .filter(|url| !url.trim().is_empty()),
    }
}

fn should_resolve_token(state: &TokenResolution, chain: &str, address: &str) -> bool {
    matches!(state, TokenResolution::Dirty) && token_address_ready(chain, address)
}

fn use_cex_pair_catalog(draft: OnchainConfigDraft, data: OnchainData) {
    let client = use_global().client;
    let active = Arc::new(AtomicBool::new(true));
    let requested_scope = RwSignal::new(None::<String>);
    let retry_state = RwSignal::new((String::new(), 0_u8));
    let cleanup_active = Arc::clone(&active);
    on_cleanup(move || cleanup_active.store(false, Ordering::Release));
    Effect::new(move |_| {
        // The request gate doubles as a retry wake-up signal after a transient failure.
        let _retry_wakeup = data.form.cex_pair_request_gate.get();
        let venue = draft.venue.get();
        let base_token = draft.base_token.get();
        let cex_symbol = draft.symbol.get();
        let base_identity = data.form.base_identity.get();
        match &base_identity {
            TokenResolution::Dirty | TokenResolution::Loading => {
                if !catalog_request_allowed(&base_identity, &cex_symbol) {
                    requested_scope.set(None);
                    data.form.cex_pair_scope.set(None);
                    data.form.cex_pairs.set(LoadState::Loading);
                    return;
                }
            }
            TokenResolution::PrecisionOnly(resolution) => {
                if !catalog_request_allowed(&base_identity, &cex_symbol) {
                    requested_scope.set(None);
                    data.form.cex_pair_scope.set(None);
                    data.form.cex_pairs.set(LoadState::Error(ApiProblem::new(
                        "ONCHAIN_TOKEN_IDENTITY_PARTIAL",
                        resolution.identity_problem.clone().unwrap_or_else(|| {
                            "链上精度已读取，但币种符号尚未通过身份核验".to_owned()
                        }),
                    )));
                    return;
                }
            }
            TokenResolution::Error(problem) => {
                if !catalog_request_allowed(&base_identity, &cex_symbol) {
                    requested_scope.set(None);
                    data.form.cex_pair_scope.set(None);
                    data.form.cex_pairs.set(LoadState::Error(ApiProblem::new(
                        "ONCHAIN_TOKEN_IDENTITY_UNAVAILABLE",
                        problem.clone(),
                    )));
                    return;
                }
            }
            TokenResolution::Idle | TokenResolution::Ready(_) => {}
        }
        let normalized_base = catalog_base_token(&base_token, &cex_symbol);
        if normalized_base.is_empty() {
            requested_scope.set(None);
            data.form.cex_pair_scope.set(None);
            data.form
                .cex_pairs
                .set(LoadState::Ready(OnchainCexPairCatalog {
                    venue,
                    base_token: String::new(),
                    ..OnchainCexPairCatalog::default()
                }));
            return;
        }
        let scope = cex_pair_scope_key(&venue, &normalized_base);
        if requested_scope.get_untracked().as_deref() == Some(scope.as_str()) {
            return;
        }
        requested_scope.set(Some(scope.clone()));
        let requested_at_ms = form_now_ms();
        if !acquire_cex_pair_request(data.form.cex_pair_request_gate, &scope, requested_at_ms) {
            return;
        }
        let retry_attempt =
            retry_state.with_untracked(
                |(retry_scope, attempt)| {
                    if retry_scope == &scope {
                        *attempt
                    } else {
                        0
                    }
                },
            );
        retry_state.set((scope.clone(), retry_attempt));
        let can_keep_catalog = data.form.cex_pair_scope.get_untracked().as_deref()
            == Some(scope.as_str())
            && data.form.cex_pairs.get_untracked().value().is_some();
        if !can_keep_catalog {
            data.form.cex_pairs.set(LoadState::Loading);
        }
        let client = client.clone();
        let active = Arc::clone(&active);
        spawn_local(async move {
            let result = client.onchain_cex_pairs(&venue, &normalized_base).await;
            if !active.load(Ordering::Acquire) {
                return;
            }
            let still_current = draft.venue.get_untracked().eq_ignore_ascii_case(&venue)
                && catalog_base_token(
                    &draft.base_token.get_untracked(),
                    &draft.symbol.get_untracked(),
                ) == normalized_base
                && catalog_request_allowed(
                    &data.form.base_identity.get_untracked(),
                    &draft.symbol.get_untracked(),
                );
            if !still_current {
                return;
            }
            match result {
                Ok(catalog) => {
                    let retry_required = cex_pair_catalog_needs_retry(&catalog);
                    data.form.cex_pair_scope.set(Some(scope.clone()));
                    data.form.cex_pairs.set(LoadState::Ready(catalog));
                    if !retry_required {
                        retry_state.set((scope.clone(), 0));
                        return;
                    }
                }
                Err(error) => {
                    data.form
                        .cex_pairs
                        .update(|state| state.apply_result(Err(error.problem)));
                }
            }
            retry_state.set((scope.clone(), retry_attempt.saturating_add(1)));
            TimeoutFuture::new(cex_pair_retry_delay_ms(retry_attempt)).await;
            if !active.load(Ordering::Acquire)
                || !cex_pair_request_is_current(draft, data, &venue, &normalized_base)
                || requested_scope.get_untracked().as_deref() != Some(scope.as_str())
                || data.form.cex_pair_request_gate.get_untracked().as_ref()
                    != Some(&(scope.clone(), requested_at_ms))
            {
                return;
            }
            requested_scope.set(None);
            data.form.cex_pair_request_gate.set(None);
        });
    });
}

fn cex_pair_catalog_needs_retry(catalog: &OnchainCexPairCatalog) -> bool {
    catalog.problem.as_ref().is_some_and(|problem| {
        matches!(
            problem.code.as_str(),
            "ONCHAIN_CEX_PAIR_REGISTRY_SYNCING"
                | "ONCHAIN_CEX_PAIR_REGISTRY_STALE"
                | "ONCHAIN_CEX_PAIR_REGISTRY_UNAVAILABLE"
        )
    })
}

fn catalog_base_token(chain_base: &str, cex_symbol: &str) -> String {
    explicit_pair_assets(cex_symbol)
        .map(|(cex_base, _)| cex_base)
        .unwrap_or_else(|| normalized_asset(chain_base))
}

fn cex_pair_scope_key(venue: &str, base_token: &str) -> String {
    format!(
        "{}|{}",
        venue.trim().to_ascii_lowercase(),
        normalized_asset(base_token),
    )
}

fn catalog_request_allowed(state: &TokenResolution, cex_symbol: &str) -> bool {
    explicit_pair_assets(cex_symbol).is_some()
        || matches!(state, TokenResolution::Idle | TokenResolution::Ready(_))
}

fn acquire_cex_pair_request(
    gate: RwSignal<Option<(String, u64)>>,
    scope: &str,
    now_ms: u64,
) -> bool {
    if gate
        .get_untracked()
        .as_ref()
        .is_some_and(|(current, at_ms)| {
            current == scope && now_ms.saturating_sub(*at_ms) < CEX_PAIR_REQUEST_DEDUPE_MS
        })
    {
        return false;
    }
    gate.set(Some((scope.to_owned(), now_ms)));
    true
}

fn cex_pair_retry_delay_ms(attempt: u8) -> u32 {
    CEX_PAIR_RETRY_BASE_MS
        .saturating_mul(1_u32 << attempt.min(3))
        .min(CEX_PAIR_RETRY_MAX_MS)
}

fn cex_pair_request_is_current(
    draft: OnchainConfigDraft,
    data: OnchainData,
    venue: &str,
    normalized_base: &str,
) -> bool {
    draft.venue.get_untracked().eq_ignore_ascii_case(venue)
        && catalog_base_token(
            &draft.base_token.get_untracked(),
            &draft.symbol.get_untracked(),
        ) == normalized_base
        && catalog_request_allowed(
            &data.form.base_identity.get_untracked(),
            &draft.symbol.get_untracked(),
        )
}

fn form_now_ms() -> u64 {
    js_sys::Date::now().max(0.0).round().min(u64::MAX as f64) as u64
}

pub(in crate::panels::modules::onchain) fn token_address_ready(chain: &str, address: &str) -> bool {
    let address = address.trim();
    if chain.eq_ignore_ascii_case("solana") {
        return (32..=44).contains(&address.len()) && address.bytes().all(is_base58_byte);
    }
    address.len() == 42
        && address.starts_with("0x")
        && address[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_base58_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_resolution_waits_for_complete_chain_addresses() {
        assert!(token_address_ready(
            "solana",
            "So11111111111111111111111111111111111111112"
        ));
        assert!(!token_address_ready("solana", "So111"));
        assert!(!token_address_ready("solana", &"0".repeat(32)));
        assert!(token_address_ready(
            "base",
            "0x4200000000000000000000000000000000000006"
        ));
        assert!(!token_address_ready("base", "0x4200"));
    }

    #[test]
    fn automatic_resolution_only_runs_after_operator_input() {
        let mint = "So11111111111111111111111111111111111111112";

        assert!(!should_resolve_token(
            &TokenResolution::Idle,
            "solana",
            mint
        ));
        assert!(should_resolve_token(
            &TokenResolution::Dirty,
            "solana",
            mint
        ));
        assert!(!should_resolve_token(
            &TokenResolution::Loading,
            "solana",
            mint
        ));
    }

    #[test]
    fn unsaved_custom_rpc_is_sent_with_read_only_identity_probe() {
        Owner::new().with(|| {
            let draft =
                OnchainConfigDraft::from_config(&shared_types::OnchainComparisonConfig::default());
            draft.rpc_mode.set(OnchainRpcMode::Custom);
            draft
                .custom_rpc_url
                .set("https://rpc.example.test".to_owned());

            let request =
                token_identity_request(draft, "solana".to_owned(), "mint-address".to_owned());
            assert_eq!(
                request.custom_rpc_url.as_deref(),
                Some("https://rpc.example.test")
            );

            draft.rpc_mode.set(OnchainRpcMode::ProviderManaged);
            assert_eq!(
                token_identity_request(draft, "solana".to_owned(), "mint".to_owned())
                    .custom_rpc_url,
                None
            );
        });
    }

    #[test]
    fn cex_catalog_follows_the_explicit_user_selected_base() {
        assert_eq!(catalog_base_token("PUPS", "SOL/USD"), "SOL");
        assert_eq!(catalog_base_token("PUPS", "PUPS/USDT"), "PUPS");
        assert_eq!(catalog_base_token("PUPS", ""), "PUPS");
    }

    #[test]
    fn explicit_cex_pair_catalog_does_not_wait_for_chain_identity() {
        let identity_error = TokenResolution::Error("RPC unavailable".to_owned());
        assert!(catalog_request_allowed(&identity_error, "SOL/USD"));
        assert!(!catalog_request_allowed(&identity_error, ""));
        assert!(catalog_request_allowed(&TokenResolution::Idle, ""));
    }

    #[test]
    fn cex_catalog_retry_backoff_is_bounded() {
        assert_eq!(cex_pair_retry_delay_ms(0), 1_500);
        assert_eq!(cex_pair_retry_delay_ms(1), 3_000);
        assert_eq!(cex_pair_retry_delay_ms(2), 6_000);
        assert_eq!(cex_pair_retry_delay_ms(3), 12_000);
        assert_eq!(cex_pair_retry_delay_ms(9), 12_000);
    }

    #[test]
    fn cex_catalog_retries_only_while_registry_is_bootstrapping() {
        let unavailable = OnchainCexPairCatalog {
            problem: Some(ApiProblem::new(
                "ONCHAIN_CEX_PAIR_REGISTRY_UNAVAILABLE",
                "registry loading",
            )),
            ..OnchainCexPairCatalog::default()
        };
        assert!(cex_pair_catalog_needs_retry(&unavailable));

        let stale_with_rows = OnchainCexPairCatalog {
            pairs: vec![shared_types::OnchainCexPairOption {
                venue: "binance".to_owned(),
                base_token: "SOL".to_owned(),
                quote_token: "USDC".to_owned(),
                cex_symbol: "SOL/USDC".to_owned(),
                native_symbol: "SOLUSDC".to_owned(),
                quality: shared_types::MarketDataQuality::Missing,
                source: shared_types::MarketDataSourceKind::LocalCache,
                freshness_ms: None,
                observed_at_ms: 1,
            }],
            problem: Some(ApiProblem::new(
                "ONCHAIN_CEX_PAIR_REGISTRY_STALE",
                "restored snapshot requires refresh",
            )),
            ..OnchainCexPairCatalog::default()
        };
        assert!(cex_pair_catalog_needs_retry(&stale_with_rows));

        let missing = OnchainCexPairCatalog {
            problem: Some(ApiProblem::new(
                "ONCHAIN_CEX_PAIR_MISSING",
                "pair is not listed",
            )),
            ..OnchainCexPairCatalog::default()
        };
        assert!(!cex_pair_catalog_needs_retry(&missing));
    }
}
