use crate::api::rest::{ApiClient, ApiError};
use gloo_timers::future::TimeoutFuture;
use shared_types::OnchainTokenResolution;

use super::TokenResolveCommand;

const TOKEN_RESOLVE_RETRY_MS: u32 = 400;

pub(super) async fn resolve_token_with_retry(
    client: &ApiClient,
    command: &TokenResolveCommand,
) -> Result<OnchainTokenResolution, ApiError> {
    let first = client.resolve_onchain_token(&command.request).await;
    let Err(problem) = &first else {
        return first;
    };
    if !retryable(problem) || !command.is_active() || !command.is_current() {
        return first;
    }

    TimeoutFuture::new(TOKEN_RESOLVE_RETRY_MS).await;
    if !command.is_active() || !command.is_current() {
        return first;
    }
    client.resolve_onchain_token(&command.request).await
}

fn retryable(error: &ApiError) -> bool {
    match error.problem.code.as_str() {
        "NETWORK" | "ONCHAIN_TOKEN_IDENTITY_TIMEOUT" => true,
        "ONCHAIN_TOKEN_IDENTITY_UNAVAILABLE" => {
            transient_upstream_identity_failure(&error.problem.message)
        }
        _ => false,
    }
}

fn transient_upstream_identity_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "rpc failed",
        "rpc returned http 5",
        "dns resolution failed",
        "timed out",
        "timeout",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_retry_covers_transport_and_cold_rpc_timeouts() {
        assert!(!retryable(&ApiError::client(
            "ONCHAIN_TOKEN_IDENTITY_UNAVAILABLE",
            "upstream unavailable",
        )));
        assert!(retryable(&ApiError::client(
            "ONCHAIN_TOKEN_IDENTITY_TIMEOUT",
            "RPC timed out",
        )));
        assert!(retryable(&ApiError::client("NETWORK", "offline")));
        assert!(retryable(&ApiError::client(
            "ONCHAIN_TOKEN_IDENTITY_UNAVAILABLE",
            "token metadata RPC failed: upstream reset",
        )));
        assert!(retryable(&ApiError::client(
            "ONCHAIN_TOKEN_IDENTITY_UNAVAILABLE",
            "Solana getTokenSupply RPC returned HTTP 502",
        )));
        assert!(!retryable(&ApiError::client(
            "ONCHAIN_TOKEN_IDENTITY_UNAVAILABLE",
            "地址没有 EVM 合约字节码，无法读取 ERC-20 身份",
        )));
        assert!(!retryable(&ApiError::client("TIMEOUT", "client timeout")));
        assert!(!retryable(&ApiError::client(
            "BAD_REQUEST",
            "invalid address",
        )));
    }
}
