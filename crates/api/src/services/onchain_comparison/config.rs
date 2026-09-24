use super::projection::{apply_runtime_telemetry, publish_snapshot};
use super::provider_runtime::provider_runtime;
use super::token_registry::resolve_jupiter_token_identities;
use crate::services::spot;
use crate::state::AppState;
use common::AppError;
use shared_types::{
    onchain_known_token, OnchainComparisonConfig, OnchainComparisonConfigPatch,
    OnchainComparisonSnapshot, OnchainRpcMode, OnchainSourceConfigPatch,
    OnchainTokenIdentityRequest, OnchainTokenResolution,
};
use std::sync::Arc;

pub(crate) async fn update_config(
    state: &AppState,
    patch: &OnchainComparisonConfigPatch,
    now_ms: i64,
) -> Result<Arc<OnchainComparisonSnapshot>, AppError> {
    let _mutation = state.onchain_config_mutation_lock().lock().await;
    let current = state.onchain_monitor().snapshot();
    let submitted_rpc_url = patch
        .source
        .as_ref()
        .and_then(|source| source.custom_rpc_url.as_deref())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned);
    let resolved = resolve_patch(state, &current.config, patch).await?;
    let next_config = state
        .onchain_monitor()
        .preview_config(&resolved)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let verified_rpc_status = verify_custom_rpc(&next_config, &resolved, now_ms).await?;
    if next_config.rpc.mode == OnchainRpcMode::Custom {
        if let Some(url) = submitted_rpc_url.as_deref() {
            crate::services::onchain_rpc_registry::persist(&next_config.chain, url)
                .await
                .map_err(AppError::Config)?;
        }
    }
    let batch_configs = state
        .onchain_monitor()
        .batch()
        .configs()
        .into_iter()
        .map(|(_, config)| config)
        .collect::<Vec<_>>();
    let store = Arc::clone(state.onchain_config_store());
    tokio::task::spawn_blocking(move || store.persist(&next_config, &batch_configs))
        .await
        .map_err(anyhow::Error::new)?
        .map_err(anyhow::Error::new)?;
    let updated = state
        .onchain_monitor()
        .update_config(&resolved, now_ms)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let mut next = (*updated).clone();
    let context = state.onchain_monitor().read_context();
    let runtime = provider_runtime(&next.config.provider);
    apply_runtime_telemetry(&mut next, &runtime);
    if let Some(status) = verified_rpc_status {
        state
            .onchain_monitor()
            .publish_rpc_status(&context, status);
    }
    next.rpc_status = (*state.onchain_monitor().rpc_status()).clone();
    publish_snapshot(state, &context, &next, true);
    if next.config.rpc.mode != OnchainRpcMode::Custom {
        super::refresh_rpc_status(state, &context, now_ms).await;
    }
    Ok(state.onchain_monitor().snapshot())
}

pub(super) async fn resolve_patch(
    state: &AppState,
    current: &OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
) -> Result<OnchainComparisonConfigPatch, AppError> {
    let base_token = patch
        .base_token
        .as_deref()
        .unwrap_or(&current.base_token)
        .trim()
        .to_ascii_uppercase();
    let quote_token = patch
        .quote_token
        .as_deref()
        .unwrap_or(&current.quote_token)
        .trim()
        .to_ascii_uppercase();
    let chain = patch
        .chain
        .as_deref()
        .unwrap_or(&current.chain)
        .trim()
        .to_ascii_lowercase();
    let mut resolved = patch.clone();
    resolved.chain = Some(chain.clone());
    resolved.base_token = Some(base_token.clone());
    resolved.quote_token = Some(quote_token.clone());
    resolved.cex_venue = Some(
        patch
            .cex_venue
            .as_deref()
            .unwrap_or(&current.cex_venue)
            .trim()
            .to_ascii_lowercase(),
    );
    if let Some(wallet_address) = resolved.wallet_address.as_mut() {
        *wallet_address = wallet_address.trim().to_owned();
    }
    resolved.cex_symbol = Some(resolve_cex_symbol(current, patch)?);
    if let Some(source) = resolved.source.as_mut() {
        if let Some(provider) = source.provider.as_mut() {
            *provider = provider.trim().to_ascii_lowercase();
        }
        if let Some(rpc_url) = source.custom_rpc_url.as_mut() {
            *rpc_url = rpc_url.trim().to_owned();
        }
    }
    hydrate_custom_rpc(state, current, &mut resolved, &chain);
    resolve_changed_token_identity(current, &mut resolved, &base_token, &quote_token).await?;
    verify_changed_contract_evidence(state, current, &mut resolved).await?;
    Ok(resolved)
}

fn hydrate_custom_rpc(
    state: &AppState,
    current: &OnchainComparisonConfig,
    patch: &mut OnchainComparisonConfigPatch,
    chain: &str,
) {
    let rpc_mode = patch
        .source
        .as_ref()
        .and_then(|source| source.rpc_mode)
        .unwrap_or(current.rpc.mode);
    if rpc_mode != OnchainRpcMode::Custom {
        return;
    }
    let submitted = patch
        .source
        .as_ref()
        .and_then(|source| source.custom_rpc_url.as_deref())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned);
    let active = (current.chain.eq_ignore_ascii_case(chain)
        && current.rpc.mode == OnchainRpcMode::Custom)
        .then(|| state.onchain_monitor().custom_rpc_url())
        .flatten()
        .map(|url| url.as_str().to_owned());
    let url = submitted
        .or(active)
        .or_else(|| crate::services::onchain_rpc_registry::configured_url(chain))
        .unwrap_or_default();
    let source = patch
        .source
        .get_or_insert_with(OnchainSourceConfigPatch::default);
    source.custom_rpc_url = Some(url);
}

async fn verify_custom_rpc(
    config: &OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
    now_ms: i64,
) -> Result<Option<shared_types::OnchainRpcStatus>, AppError> {
    if config.rpc.mode != OnchainRpcMode::Custom {
        return Ok(None);
    }
    let url = patch
        .source
        .as_ref()
        .and_then(|source| source.custom_rpc_url.as_deref())
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("自定义 RPC URL 未配置".to_owned()))?;
    let status = super::rpc::probe(config, Some(url), now_ms).await;
    if status.ready {
        Ok(Some(status))
    } else {
        Err(AppError::BadRequest(status.problem.unwrap_or_else(|| {
            "自定义 RPC 尚未通过链身份与最新区块核验".to_owned()
        })))
    }
}

async fn verify_changed_contract_evidence(
    state: &AppState,
    current: &OnchainComparisonConfig,
    patch: &mut OnchainComparisonConfigPatch,
) -> Result<(), AppError> {
    let chain = patch.chain.as_deref().unwrap_or(&current.chain);
    let custom_rpc_url = patch
        .source
        .as_ref()
        .and_then(|source| source.custom_rpc_url.clone());
    let base_request =
        changed_contract_request(current, patch, chain, true, custom_rpc_url.clone());
    let quote_request = changed_contract_request(current, patch, chain, false, custom_rpc_url);
    let (base_resolution, quote_resolution) = tokio::join!(
        resolve_contract_evidence(state, base_request),
        resolve_contract_evidence(state, quote_request),
    );
    if let Some(resolution) = base_resolution? {
        apply_contract_evidence(patch, &resolution, true)?;
    }
    if let Some(resolution) = quote_resolution? {
        apply_contract_evidence(patch, &resolution, false)?;
    }
    Ok(())
}

fn changed_contract_request(
    current: &OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
    chain: &str,
    base: bool,
    custom_rpc_url: Option<String>,
) -> Option<OnchainTokenIdentityRequest> {
    let (current_address, current_token, current_decimals, current_resolved) = if base {
        (
            &current.base_mint,
            &current.base_token,
            current.base_decimals,
            current.base_identity_resolved,
        )
    } else {
        (
            &current.quote_mint,
            &current.quote_token,
            current.quote_decimals,
            current.quote_identity_resolved,
        )
    };
    let (requested_address, requested_token, requested_decimals, requested_resolved) = if base {
        (
            patch.base_mint.as_deref(),
            patch.base_token.as_deref(),
            patch.base_decimals,
            patch.base_identity_resolved,
        )
    } else {
        (
            patch.quote_mint.as_deref(),
            patch.quote_token.as_deref(),
            patch.quote_decimals,
            patch.quote_identity_resolved,
        )
    };
    let address = requested_address.unwrap_or(current_address).trim();
    let chain_changed = !chain.eq_ignore_ascii_case(&current.chain);
    let address_changed = !token_address_matches(chain, address, current_address);
    let identity_changed = requested_token
        .is_some_and(|token| !token.eq_ignore_ascii_case(current_token))
        || requested_decimals.is_some_and(|decimals| decimals != current_decimals)
        || requested_resolved.is_some_and(|resolved| resolved != current_resolved);
    (!address.is_empty() && (chain_changed || address_changed || identity_changed)).then(|| {
        OnchainTokenIdentityRequest {
            chain: chain.to_owned(),
            address: address.to_owned(),
            custom_rpc_url,
        }
    })
}

async fn resolve_contract_evidence(
    state: &AppState,
    request: Option<OnchainTokenIdentityRequest>,
) -> Result<Option<OnchainTokenResolution>, AppError> {
    match request {
        Some(request) => super::token_identity_service::resolve(state, request)
            .await
            .map(Some),
        None => Ok(None),
    }
}

fn apply_contract_evidence(
    patch: &mut OnchainComparisonConfigPatch,
    resolution: &OnchainTokenResolution,
    base: bool,
) -> Result<(), AppError> {
    let (label, submitted_token, submitted_decimals) = if base {
        ("Base", patch.base_token.as_deref(), patch.base_decimals)
    } else {
        ("Quote", patch.quote_token.as_deref(), patch.quote_decimals)
    };
    if submitted_decimals.is_some_and(|decimals| decimals != resolution.decimals) {
        return Err(AppError::BadRequest(format!(
            "{label} 精度与链上 decimals() / getTokenSupply 结果不一致；等待自动识别完成后再保存"
        )));
    }
    if let Some(identity) = resolution.identity.as_ref() {
        if submitted_token.is_some_and(|token| !token.eq_ignore_ascii_case(&identity.symbol)) {
            return Err(AppError::BadRequest(format!(
                "{label} 币种符号与合约返回的 {} 不一致；等待自动识别完成后再保存",
                identity.symbol
            )));
        }
        if base {
            patch.base_token = Some(identity.symbol.clone());
            patch.base_decimals = Some(identity.decimals);
            patch.base_identity_resolved = Some(identity.verified);
        } else {
            patch.quote_token = Some(identity.symbol.clone());
            patch.quote_decimals = Some(identity.decimals);
            patch.quote_identity_resolved = Some(identity.verified);
        }
    } else if base {
        patch.base_decimals = Some(resolution.decimals);
        patch.base_identity_resolved = Some(false);
    } else {
        patch.quote_decimals = Some(resolution.decimals);
        patch.quote_identity_resolved = Some(false);
    }
    Ok(())
}

fn token_address_matches(chain: &str, left: &str, right: &str) -> bool {
    if chain.eq_ignore_ascii_case("solana") {
        left == right
    } else {
        left.eq_ignore_ascii_case(right)
    }
}

fn resolve_cex_symbol(
    current: &OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
) -> Result<String, AppError> {
    let requested = match patch.cex_symbol.as_deref() {
        Some(requested) => requested,
        None if patch.base_token.is_none() && patch.quote_token.is_none() => &current.cex_symbol,
        None => {
            return Err(AppError::BadRequest(
                "链上资产发生变化后必须明确选择 CEX 交易对，系统不会自动配对".to_owned(),
            ));
        }
    };
    let Some((requested_base, requested_quote)) = spot::split_spot_pair(requested) else {
        return Err(AppError::BadRequest(
            "请选择一个明确包含 Base 和 Quote 的 CEX 现货交易对".to_owned(),
        ));
    };
    if requested_base.eq_ignore_ascii_case(&requested_quote) {
        return Err(AppError::BadRequest(
            "CEX 交易对的 Base 与 Quote 不能相同".to_owned(),
        ));
    }
    Ok(format!("{requested_base}/{requested_quote}"))
}

pub(super) async fn resolve_changed_token_identity(
    current: &OnchainComparisonConfig,
    patch: &mut OnchainComparisonConfigPatch,
    base_token: &str,
    quote_token: &str,
) -> Result<(), AppError> {
    let chain = patch.chain.clone().unwrap_or_else(|| current.chain.clone());
    let base_changed = !base_token.eq_ignore_ascii_case(&current.base_token)
        || !chain.eq_ignore_ascii_case(&current.chain);
    let quote_changed = !quote_token.eq_ignore_ascii_case(&current.quote_token)
        || !chain.eq_ignore_ascii_case(&current.chain);
    let base_identity_resolved = patch
        .base_identity_resolved
        .unwrap_or(current.base_identity_resolved);
    let quote_identity_resolved = patch
        .quote_identity_resolved
        .unwrap_or(current.quote_identity_resolved);
    validate_unresolved_identity(patch, base_identity_resolved, true)?;
    validate_unresolved_identity(patch, quote_identity_resolved, false)?;
    let base_needs_resolution = base_identity_resolved
        && base_changed
        && patch
            .base_mint
            .as_deref()
            .is_none_or(|mint| mint == current.base_mint);
    let quote_needs_resolution = quote_identity_resolved
        && quote_changed
        && patch
            .quote_mint
            .as_deref()
            .is_none_or(|mint| mint == current.quote_mint);
    if base_needs_resolution {
        apply_known_identity(patch, &chain, base_token, true)?;
    }
    if quote_needs_resolution {
        apply_known_identity(patch, &chain, quote_token, false)?;
    }
    let base_missing =
        base_needs_resolution && patch.base_mint.as_deref().is_none_or(str::is_empty);
    let quote_missing =
        quote_needs_resolution && patch.quote_mint.as_deref().is_none_or(str::is_empty);
    if !base_missing && !quote_missing {
        return Ok(());
    }
    if !chain.eq_ignore_ascii_case("solana") {
        let tokens = [
            base_missing.then_some(base_token),
            quote_missing.then_some(quote_token),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        return Err(AppError::BadRequest(format!(
            "{chain} 上的 {tokens} 缺少官方合约地址；请在高级设置填写地址与精度"
        )));
    }
    let symbols = [
        base_missing.then_some(base_token),
        quote_missing.then_some(quote_token),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let identities = resolve_jupiter_token_identities(&symbols)
        .await
        .map_err(AppError::BadRequest)?;
    for identity in identities {
        if identity.symbol.eq_ignore_ascii_case(base_token) && base_missing {
            patch.base_mint = Some(identity.address);
            patch.base_decimals = Some(identity.decimals);
            patch.base_amount_raw = Some(unit_amount(identity.decimals, 1)?);
            patch.base_identity_resolved = Some(true);
        } else if identity.symbol.eq_ignore_ascii_case(quote_token) && quote_missing {
            patch.quote_mint = Some(identity.address);
            patch.quote_decimals = Some(identity.decimals);
            patch.quote_amount_raw = Some(unit_amount(identity.decimals, 100)?);
            patch.quote_identity_resolved = Some(true);
        }
    }
    Ok(())
}

fn validate_unresolved_identity(
    patch: &OnchainComparisonConfigPatch,
    identity_resolved: bool,
    base: bool,
) -> Result<(), AppError> {
    if identity_resolved {
        return Ok(());
    }
    let (mint, decimals) = if base {
        (patch.base_mint.as_deref(), patch.base_decimals)
    } else {
        (patch.quote_mint.as_deref(), patch.quote_decimals)
    };
    if mint.is_some_and(|mint| !mint.trim().is_empty()) && decimals.is_some() {
        return Ok(());
    }
    let leg = if base { "Base" } else { "Quote" };
    Err(AppError::BadRequest(format!(
        "{leg} 身份未完整解析时只能用于原始观察，并且必须携带由 RPC 读取的合约地址与精度"
    )))
}

fn apply_known_identity(
    patch: &mut OnchainComparisonConfigPatch,
    chain: &str,
    token: &str,
    base: bool,
) -> Result<(), AppError> {
    let Some(identity) = onchain_known_token(chain, token) else {
        if base {
            patch.base_mint = Some(String::new());
        } else {
            patch.quote_mint = Some(String::new());
        }
        return Ok(());
    };
    if base {
        patch.base_mint = Some(identity.address.to_owned());
        patch.base_decimals = Some(identity.decimals);
        patch.base_amount_raw = Some(unit_amount(identity.decimals, 1)?);
        patch.base_identity_resolved = Some(true);
    } else {
        patch.quote_mint = Some(identity.address.to_owned());
        patch.quote_decimals = Some(identity.decimals);
        patch.quote_amount_raw = Some(unit_amount(identity.decimals, 100)?);
        patch.quote_identity_resolved = Some(true);
    }
    Ok(())
}

fn unit_amount(decimals: u8, units: u128) -> Result<String, AppError> {
    10_u128
        .checked_pow(u32::from(decimals))
        .and_then(|scale| scale.checked_mul(units))
        .map(|amount| amount.to_string())
        .ok_or_else(|| {
            AppError::BadRequest("token precision exceeds u128 amount capacity".to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_cex_pair_allows_custom_markets_but_rejects_invalid_pairs() {
        let current = OnchainComparisonConfig::default();
        let matching = OnchainComparisonConfigPatch {
            cex_symbol: Some("WIF-USDC".to_owned()),
            ..OnchainComparisonConfigPatch::default()
        };
        let cross_quote = OnchainComparisonConfigPatch {
            cex_symbol: Some("WIF/USDT".to_owned()),
            ..OnchainComparisonConfigPatch::default()
        };
        let custom_base = OnchainComparisonConfigPatch {
            cex_symbol: Some("SOL/USDT".to_owned()),
            ..OnchainComparisonConfigPatch::default()
        };
        let same_asset = OnchainComparisonConfigPatch {
            cex_symbol: Some("SOL/SOL".to_owned()),
            ..OnchainComparisonConfigPatch::default()
        };

        assert_eq!(
            resolve_cex_symbol(&current, &matching).ok().as_deref(),
            Some("WIF/USDC")
        );
        assert_eq!(
            resolve_cex_symbol(&current, &cross_quote).ok().as_deref(),
            Some("WIF/USDT")
        );
        assert_eq!(
            resolve_cex_symbol(&current, &custom_base).ok().as_deref(),
            Some("SOL/USDT")
        );
        assert!(resolve_cex_symbol(&current, &same_asset).is_err());

        let missing = OnchainComparisonConfigPatch {
            base_token: Some("WIF".to_owned()),
            cex_symbol: None,
            ..OnchainComparisonConfigPatch::default()
        };
        assert!(resolve_cex_symbol(&current, &missing).is_err());
    }

    #[test]
    fn unresolved_identity_requires_explicit_rpc_evidence() {
        let valid = OnchainComparisonConfigPatch {
            base_mint: Some("mint".to_owned()),
            base_decimals: Some(9),
            ..OnchainComparisonConfigPatch::default()
        };
        assert!(validate_unresolved_identity(&valid, false, true).is_ok());

        let missing_precision = OnchainComparisonConfigPatch {
            base_mint: Some("mint".to_owned()),
            ..OnchainComparisonConfigPatch::default()
        };
        assert!(validate_unresolved_identity(&missing_precision, false, true).is_err());
        assert!(validate_unresolved_identity(&valid, true, true).is_ok());
    }

    #[test]
    fn unverified_contract_metadata_cannot_enable_fee_adjusted_execution() {
        let mut patch = OnchainComparisonConfigPatch {
            base_token: Some("USDT".to_owned()),
            base_decimals: Some(6),
            base_identity_resolved: Some(true),
            ..OnchainComparisonConfigPatch::default()
        };
        let resolution = OnchainTokenResolution::complete(shared_types::OnchainTokenIdentity {
            chain: "ethereum".to_owned(),
            address: "0xtoken".to_owned(),
            symbol: "USDT".to_owned(),
            name: Some("Tether USD".to_owned()),
            decimals: 6,
            source: "publicnode_ethereum_rpc".to_owned(),
            evidence_url: "https://eips.ethereum.org/EIPS/eip-20".to_owned(),
            verified: false,
            native: false,
            observed_at_ms: 1,
        });

        apply_contract_evidence(&mut patch, &resolution, true)
            .expect("contract metadata should remain usable for raw observation");

        assert_eq!(patch.base_decimals, Some(6));
        assert_eq!(patch.base_identity_resolved, Some(false));
    }

    #[test]
    fn contract_precision_mismatch_is_rejected_before_config_save() {
        let mut patch = OnchainComparisonConfigPatch {
            base_token: Some("USDT".to_owned()),
            base_decimals: Some(18),
            ..OnchainComparisonConfigPatch::default()
        };
        let resolution = OnchainTokenResolution {
            chain: "ethereum".to_owned(),
            address: "0xtoken".to_owned(),
            decimals: 6,
            precision_source: "publicnode_ethereum_rpc".to_owned(),
            precision_evidence_url: "https://eips.ethereum.org/EIPS/eip-20".to_owned(),
            identity: None,
            identity_problem: Some("symbol unavailable".to_owned()),
            observed_at_ms: 1,
        };

        let error = apply_contract_evidence(&mut patch, &resolution, true)
            .expect_err("wrong decimals must not reach quote calculation");

        assert!(error.to_string().contains("精度与链上"));
    }

    #[test]
    fn changing_precision_for_the_same_contract_still_requires_chain_evidence() {
        let current = OnchainComparisonConfig::default();
        let precision_patch = OnchainComparisonConfigPatch {
            base_decimals: Some(8),
            ..OnchainComparisonConfigPatch::default()
        };
        let fee_patch = OnchainComparisonConfigPatch {
            cex_taker_fee_bps: Some(5.0),
            ..OnchainComparisonConfigPatch::default()
        };

        assert!(
            changed_contract_request(&current, &precision_patch, "solana", true, None,).is_some()
        );
        assert!(changed_contract_request(&current, &fee_patch, "solana", true, None).is_none());
    }

    #[tokio::test]
    async fn unresolved_identity_preserves_the_explicit_contract_without_symbol_lookup() {
        let current = OnchainComparisonConfig::default();
        let mut patch = OnchainComparisonConfigPatch {
            base_token: Some("PUPS".to_owned()),
            base_identity_resolved: Some(false),
            base_mint: Some("pups-mint".to_owned()),
            base_decimals: Some(9),
            base_amount_raw: Some("1000000000".to_owned()),
            ..OnchainComparisonConfigPatch::default()
        };

        resolve_changed_token_identity(&current, &mut patch, "PUPS", "USDC")
            .await
            .expect("provisional raw observation should not need symbol lookup");

        assert_eq!(patch.base_mint.as_deref(), Some("pups-mint"));
        assert_eq!(patch.base_decimals, Some(9));
    }
}
