use super::evm_token_identity;
use super::solana_token_precision::{self, SolanaTokenPrecision, SOLANA_TOKEN_SUPPLY_DOCS};
use super::token_registry::{resolve_jupiter_token_identity, ResolvedTokenIdentity};
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    onchain_chain_preset, OnchainRpcMode, OnchainTokenIdentity, OnchainTokenIdentityRequest,
    OnchainTokenResolution, EVM_NATIVE_TOKEN_ADDRESS,
};
use std::sync::OnceLock;

const JUPITER_TOKEN_SEARCH_URL: &str = "https://api.jup.ag/tokens/v2/search";
const CIRCLE_USDC_CONTRACTS_URL: &str =
    "https://developers.circle.com/stablecoins/usdc-contract-addresses";
const JUPITER_IDENTITY_WAIT: std::time::Duration = std::time::Duration::from_millis(1_500);
const TOKEN_PRECISION_WAIT: std::time::Duration = std::time::Duration::from_secs(4);
const EVM_TOKEN_IDENTITY_WAIT: std::time::Duration = std::time::Duration::from_secs(8);
const COMPLETE_TOKEN_RESOLUTION_CACHE_MS: i64 = 60_000;
const PARTIAL_TOKEN_RESOLUTION_CACHE_MS: i64 = 5_000;

pub(crate) async fn resolve(
    state: &AppState,
    request: OnchainTokenIdentityRequest,
) -> Result<OnchainTokenResolution, AppError> {
    let OnchainTokenIdentityRequest {
        chain,
        address,
        custom_rpc_url,
    } = request;
    let chain = chain.trim().to_ascii_lowercase();
    let requested_rpc = custom_rpc_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());
    let preset = onchain_chain_preset(&chain)
        .ok_or_else(|| AppError::BadRequest("unsupported on-chain network".to_owned()))?;
    let address = address.trim();
    validate_address(&chain, address)?;
    if requested_rpc.is_none() {
        if let Some(resolution) = cached_resolution(&chain, address) {
            return Ok(resolution);
        }
    }
    if let Some(identity) = preset_identity(preset, address) {
        return Ok(cache_resolution(OnchainTokenResolution::complete(identity)));
    }
    if chain == "solana" {
        return resolve_solana(state, chain, address, requested_rpc)
            .await
            .map(|resolution| maybe_cache_resolution(resolution, requested_rpc.is_none()));
    }
    let snapshot = state.onchain_monitor().snapshot();
    let active_rpc = (snapshot.config.chain.eq_ignore_ascii_case(&chain)
        && snapshot.config.rpc.mode == OnchainRpcMode::Custom)
        .then(|| state.onchain_monitor().custom_rpc_url())
        .flatten();
    let saved_rpc = crate::services::onchain_rpc_registry::configured_url(&chain);
    let custom_rpc = requested_rpc
        .or(saved_rpc.as_deref())
        .or(active_rpc.as_deref().map(String::as_str));
    let resolution = match tokio::time::timeout(
        EVM_TOKEN_IDENTITY_WAIT,
        evm_token_identity::resolve(&chain, address, custom_rpc),
    )
    .await
    {
        Ok(result) => result.map_err(upstream_problem),
        Err(_) => Err(upstream_timeout(format!(
            "{chain} ERC-20 合约元数据读取超过 {}ms",
            EVM_TOKEN_IDENTITY_WAIT.as_millis()
        ))),
    }?;
    Ok(maybe_cache_resolution(resolution, requested_rpc.is_none()))
}

fn cached_resolution(chain: &str, address: &str) -> Option<OnchainTokenResolution> {
    let key = resolution_cache_key(chain, address);
    let resolution = token_resolution_cache().get(&key)?.clone();
    let freshness_ms = common::time::now_ms().saturating_sub(resolution.observed_at_ms);
    if freshness_ms <= resolution_cache_ttl_ms(&resolution) {
        Some(resolution)
    } else {
        token_resolution_cache().remove(&key);
        None
    }
}

fn resolution_cache_ttl_ms(resolution: &OnchainTokenResolution) -> i64 {
    if resolution.is_complete() {
        COMPLETE_TOKEN_RESOLUTION_CACHE_MS
    } else {
        PARTIAL_TOKEN_RESOLUTION_CACHE_MS
    }
}

fn cache_resolution(resolution: OnchainTokenResolution) -> OnchainTokenResolution {
    token_resolution_cache().insert(
        resolution_cache_key(&resolution.chain, &resolution.address),
        resolution.clone(),
    );
    resolution
}

fn maybe_cache_resolution(
    resolution: OnchainTokenResolution,
    cache_allowed: bool,
) -> OnchainTokenResolution {
    if cache_allowed {
        cache_resolution(resolution)
    } else {
        resolution
    }
}

fn resolution_cache_key(chain: &str, address: &str) -> String {
    if chain.eq_ignore_ascii_case("solana") {
        format!("solana:{address}")
    } else {
        format!(
            "{}:{}",
            chain.to_ascii_lowercase(),
            address.to_ascii_lowercase()
        )
    }
}

fn token_resolution_cache() -> &'static dashmap::DashMap<String, OnchainTokenResolution> {
    static CACHE: OnceLock<dashmap::DashMap<String, OnchainTokenResolution>> = OnceLock::new();
    CACHE.get_or_init(dashmap::DashMap::new)
}

async fn resolve_solana(
    state: &AppState,
    chain: String,
    address: &str,
    requested_rpc: Option<&str>,
) -> Result<OnchainTokenResolution, AppError> {
    let snapshot = state.onchain_monitor().snapshot();
    let active_rpc = (snapshot.config.chain.eq_ignore_ascii_case("solana")
        && snapshot.config.rpc.mode == OnchainRpcMode::Custom)
        .then(|| state.onchain_monitor().custom_rpc_url())
        .flatten();
    let saved_rpc = crate::services::onchain_rpc_registry::configured_url("solana");
    let custom_rpc = requested_rpc
        .or(saved_rpc.as_deref())
        .or(active_rpc.as_deref().map(String::as_str));
    let (precision, identity) = tokio::join!(
        async {
            tokio::time::timeout(
                TOKEN_PRECISION_WAIT,
                solana_token_precision::resolve(address, custom_rpc),
            )
            .await
            .map_err(|_| {
                format!(
                    "Solana getTokenSupply timed out after {}ms",
                    TOKEN_PRECISION_WAIT.as_millis()
                )
            })
            .and_then(|result| result)
        },
        async {
            tokio::time::timeout(
                JUPITER_IDENTITY_WAIT,
                resolve_jupiter_token_identity(address),
            )
            .await
            .map_err(|_| "Jupiter token metadata is still loading".to_owned())
            .and_then(|result| result)
        }
    );
    merge_solana_evidence(chain, address, identity, precision)
}

fn merge_solana_evidence(
    chain: String,
    address: &str,
    identity: Result<ResolvedTokenIdentity, String>,
    precision: Result<SolanaTokenPrecision, String>,
) -> Result<OnchainTokenResolution, AppError> {
    match (identity, precision) {
        (Ok(identity), Ok(precision)) if identity.decimals != precision.decimals => {
            let problem = format!(
                "Jupiter reports {} decimals but Solana getTokenSupply reports {}; symbol remains unverified",
                identity.decimals, precision.decimals
            );
            Ok(precision_only(chain, address, precision, problem))
        }
        (Ok(identity), precision) => Ok(complete_solana_identity(chain, identity, precision.ok())),
        (Err(identity_problem), Ok(precision)) => {
            Ok(precision_only(chain, address, precision, identity_problem))
        }
        (Err(identity_problem), Err(precision_problem)) => {
            let problem = format!(
                "Jupiter token metadata failed: {identity_problem}; Solana getTokenSupply failed: {precision_problem}"
            );
            if precision_problem.contains("timed out") {
                Err(upstream_timeout(problem))
            } else {
                Err(upstream_problem(problem))
            }
        }
    }
}

fn complete_solana_identity(
    chain: String,
    identity: ResolvedTokenIdentity,
    precision: Option<SolanaTokenPrecision>,
) -> OnchainTokenResolution {
    let mut resolution = OnchainTokenResolution::complete(OnchainTokenIdentity {
        chain,
        address: identity.address,
        symbol: identity.symbol,
        name: identity.name,
        decimals: identity.decimals,
        source: "jupiter_tokens_v2".to_owned(),
        evidence_url: JUPITER_TOKEN_SEARCH_URL.to_owned(),
        verified: identity.verified,
        native: false,
        observed_at_ms: common::time::now_ms(),
    });
    if let Some(precision) = precision {
        resolution.precision_source = precision.source;
        resolution.precision_evidence_url = SOLANA_TOKEN_SUPPLY_DOCS.to_owned();
    }
    resolution
}

fn precision_only(
    chain: String,
    address: &str,
    precision: SolanaTokenPrecision,
    identity_problem: String,
) -> OnchainTokenResolution {
    OnchainTokenResolution {
        chain,
        address: address.to_owned(),
        decimals: precision.decimals,
        precision_source: precision.source,
        precision_evidence_url: SOLANA_TOKEN_SUPPLY_DOCS.to_owned(),
        identity: None,
        identity_problem: Some(identity_problem),
        observed_at_ms: precision.observed_at_ms,
    }
}

fn preset_identity(
    preset: &shared_types::OnchainChainPreset,
    address: &str,
) -> Option<OnchainTokenIdentity> {
    let matches = |known: &str| {
        if preset.chain_id.is_some() {
            known.eq_ignore_ascii_case(address)
        } else {
            known == address
        }
    };
    let (symbol, decimals, native) = if matches(preset.base_address) {
        (
            preset.base_token,
            preset.base_decimals,
            preset
                .base_address
                .eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS),
        )
    } else if matches(preset.quote_address) {
        (preset.quote_token, preset.quote_decimals, false)
    } else {
        return None;
    };
    let evidence_url = if symbol.eq_ignore_ascii_case("USDC") {
        CIRCLE_USDC_CONTRACTS_URL
    } else {
        preset.provider_docs_url
    };
    Some(OnchainTokenIdentity {
        chain: preset.id.to_owned(),
        address: address.to_owned(),
        symbol: symbol.to_owned(),
        name: None,
        decimals,
        source: "crossline_chain_preset".to_owned(),
        evidence_url: evidence_url.to_owned(),
        verified: true,
        native,
        observed_at_ms: common::time::now_ms(),
    })
}

fn validate_address(chain: &str, address: &str) -> Result<(), AppError> {
    let valid = if chain == "solana" {
        bs58::decode(address)
            .into_vec()
            .is_ok_and(|decoded| decoded.len() == 32)
    } else {
        address.len() == 42
            && address.starts_with("0x")
            && address[2..]
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "{chain} token address has an invalid format"
        )))
    }
}

fn upstream_problem(problem: String) -> AppError {
    AppError::domain(
        StatusCode::BAD_GATEWAY,
        "ONCHAIN_TOKEN_IDENTITY_UNAVAILABLE",
        problem,
    )
}

fn upstream_timeout(problem: String) -> AppError {
    AppError::domain(
        StatusCode::GATEWAY_TIMEOUT,
        "ONCHAIN_TOKEN_IDENTITY_TIMEOUT",
        problem,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solana_precision(decimals: u8) -> SolanaTokenPrecision {
        SolanaTokenPrecision {
            decimals,
            source: "solana_mainnet_rpc".to_owned(),
            observed_at_ms: 123,
        }
    }

    fn jupiter_identity(decimals: u8) -> ResolvedTokenIdentity {
        ResolvedTokenIdentity {
            symbol: "PUPS".to_owned(),
            address: "mint".to_owned(),
            decimals,
            name: Some("PUPS".to_owned()),
            verified: true,
        }
    }

    #[test]
    fn native_base_identity_is_resolved_without_rpc() {
        let preset = onchain_chain_preset("base");
        assert!(preset.is_some(), "base preset should exist");
        let Some(preset) = preset else {
            return;
        };
        let identity = preset_identity(preset, EVM_NATIVE_TOKEN_ADDRESS);
        assert!(identity.is_some(), "native identity should resolve");
        let Some(identity) = identity else {
            return;
        };
        assert_eq!(identity.symbol, "ETH");
        assert_eq!(identity.decimals, 18);
        assert!(identity.native);
        assert!(identity.verified);
    }

    #[test]
    fn preset_usdc_identity_uses_circle_contract_evidence() {
        let preset = onchain_chain_preset("base");
        assert!(preset.is_some(), "base preset should exist");
        let Some(preset) = preset else {
            return;
        };
        let identity = preset_identity(preset, preset.quote_address);
        assert!(identity.is_some(), "base USDC identity should resolve");
        let Some(identity) = identity else {
            return;
        };
        assert_eq!(identity.symbol, "USDC");
        assert_eq!(identity.decimals, 6);
        assert_eq!(identity.evidence_url, CIRCLE_USDC_CONTRACTS_URL);
    }

    #[test]
    fn malformed_addresses_fail_before_network_access() {
        assert!(validate_address("base", "0x1234").is_err());
        assert!(validate_address("solana", "not-a-mint").is_err());
        assert!(validate_address("solana", &"0".repeat(32)).is_err());
    }

    #[test]
    fn solana_precision_survives_slow_identity_metadata() {
        let resolution = merge_solana_evidence(
            "solana".to_owned(),
            "mint",
            Err("Jupiter token metadata is still loading".to_owned()),
            Ok(solana_precision(9)),
        )
        .expect("Solana precision should remain usable");

        assert_eq!(resolution.decimals, 9);
        assert_eq!(resolution.precision_source, "solana_mainnet_rpc");
        assert!(resolution.identity.is_none());
    }

    #[test]
    fn matching_solana_evidence_keeps_identity_and_rpc_precision_source() {
        let resolution = merge_solana_evidence(
            "solana".to_owned(),
            "mint",
            Ok(jupiter_identity(9)),
            Ok(solana_precision(9)),
        )
        .expect("matching evidence should resolve");

        assert_eq!(resolution.decimals, 9);
        assert_eq!(resolution.precision_source, "solana_mainnet_rpc");
        assert_eq!(
            resolution
                .identity
                .as_ref()
                .map(|identity| identity.symbol.as_str()),
            Some("PUPS")
        );
    }

    #[test]
    fn precision_only_results_expire_before_the_frontend_identity_retry() {
        let partial = precision_only(
            "solana".to_owned(),
            "mint",
            solana_precision(9),
            "Jupiter token metadata is still loading".to_owned(),
        );
        let complete = complete_solana_identity(
            "solana".to_owned(),
            jupiter_identity(9),
            Some(solana_precision(9)),
        );

        assert_eq!(resolution_cache_ttl_ms(&partial), 5_000);
        assert_eq!(resolution_cache_ttl_ms(&complete), 60_000);
        assert!(
            resolution_cache_ttl_ms(&partial) < 30_000,
            "partial cache must expire before the UI retries identity metadata"
        );
    }
}
