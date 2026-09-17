use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_types::{OnchainComparisonConfig, OnchainRpcMode, OnchainUnsignedTransaction};
use std::sync::OnceLock;

use crate::services::{onchain_rpc_registry, onchain_signer, venue_credentials};
use crate::state::AppState;

use super::super::quote::quote_client;
use super::super::{rpc, rpc_target};

const JUPITER_EXECUTE_ENDPOINT: &str = "https://api.jup.ag/swap/v2/execute";
const SOLANA_MAINNET_RPC: &str = "https://api.mainnet-beta.solana.com";
const FINALITY_POLLS: usize = 12;
const FINALITY_POLL_MS: u64 = 250;
const RPC_VERIFY_CACHE_MS: i64 = 5_000;
const RPC_VERIFY_CACHE_MAX: usize = 16;
const RPC_VERIFY_CACHE_KEY: &[u8] = b"crossline-onchain-rpc-verify-v1";

#[derive(Debug, Clone)]
pub(super) enum PreparedChainSubmission {
    Jupiter {
        signed_transaction: String,
        request_id: String,
        last_valid_block_height: Option<u64>,
        api_key: String,
        local_transaction_id: String,
    },
    SolanaRpc {
        signed_transaction: String,
        rpc_url: String,
        local_transaction_id: String,
    },
    EvmRpc {
        raw_transaction: String,
        rpc_url: String,
        local_transaction_id: String,
    },
}

impl PreparedChainSubmission {
    pub(super) fn transaction_id(&self) -> &str {
        match self {
            Self::Jupiter {
                local_transaction_id,
                ..
            }
            | Self::SolanaRpc {
                local_transaction_id,
                ..
            }
            | Self::EvmRpc {
                local_transaction_id,
                ..
            } => local_transaction_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChainSubmissionOutcome {
    Confirmed {
        transaction_id: String,
    },
    Rejected {
        transaction_id: String,
        problem: String,
    },
    Pending {
        transaction_id: String,
        problem: String,
    },
}

pub(super) fn readiness(state: &AppState, config: &OnchainComparisonConfig) -> Result<(), String> {
    onchain_signer::readiness(&config.chain, &config.wallet_address)?;
    if config.chain.eq_ignore_ascii_case("solana") {
        if venue_credentials::secret("JUPITER_API_KEY").is_some() {
            return Ok(());
        }
        return require_custom_rpc(state, config).map(|_| ());
    }
    require_custom_rpc(state, config).map(|_| ())
}

pub(super) async fn prepare(
    state: &AppState,
    config: &OnchainComparisonConfig,
    transaction: &OnchainUnsignedTransaction,
) -> Result<PreparedChainSubmission, String> {
    readiness(state, config)?;
    match transaction {
        OnchainUnsignedTransaction::SolanaVersioned {
            request_id,
            last_valid_block_height,
            ..
        } => {
            let signed = onchain_signer::sign(
                &config.chain,
                &config.wallet_address,
                transaction,
                None,
                None,
            )?;
            let local_transaction_id = onchain_signer::solana_transaction_id(&signed)?;
            if let Some(api_key) = venue_credentials::secret("JUPITER_API_KEY") {
                return Ok(PreparedChainSubmission::Jupiter {
                    signed_transaction: signed,
                    request_id: request_id.clone(),
                    last_valid_block_height: *last_valid_block_height,
                    api_key,
                    local_transaction_id,
                });
            }
            Ok(PreparedChainSubmission::SolanaRpc {
                signed_transaction: signed,
                rpc_url: require_verified_custom_rpc(state, config).await?,
                local_transaction_id,
            })
        }
        OnchainUnsignedTransaction::EvmCall { .. } => {
            let rpc_url = require_verified_custom_rpc(state, config).await?;
            prepare_evm_rpc(config, transaction, rpc_url).await
        }
    }
}

/// Prepare a wallet transaction for the verified custom RPC only. Unlike the
/// swap path, this never sends an arbitrary transfer through Jupiter Execute.
pub(super) async fn prepare_rpc(
    state: &AppState,
    config: &OnchainComparisonConfig,
    transaction: &OnchainUnsignedTransaction,
) -> Result<PreparedChainSubmission, String> {
    onchain_signer::readiness(&config.chain, &config.wallet_address)?;
    let rpc_url = require_verified_custom_rpc(state, config).await?;
    match transaction {
        OnchainUnsignedTransaction::SolanaVersioned { .. } => {
            let signed = onchain_signer::sign(
                &config.chain,
                &config.wallet_address,
                transaction,
                None,
                None,
            )?;
            let local_transaction_id = onchain_signer::solana_transaction_id(&signed)?;
            Ok(PreparedChainSubmission::SolanaRpc {
                signed_transaction: signed,
                rpc_url,
                local_transaction_id,
            })
        }
        OnchainUnsignedTransaction::EvmCall { .. } => {
            prepare_evm_rpc(config, transaction, rpc_url).await
        }
    }
}

async fn prepare_evm_rpc(
    config: &OnchainComparisonConfig,
    transaction: &OnchainUnsignedTransaction,
    rpc_url: String,
) -> Result<PreparedChainSubmission, String> {
    let OnchainUnsignedTransaction::EvmCall { gas_price, .. } = transaction else {
        return Err("不是 EVM 交易".to_owned());
    };
    let (url, client) = rpc_target::rpc_target(&rpc_url).await?;
    let nonce = rpc::rpc_result(
        &client,
        url.as_str(),
        "eth_getTransactionCount",
        serde_json::json!([config.wallet_address, "pending"]),
        101,
    )
    .await?
    .as_str()
    .map(str::to_owned)
    .ok_or_else(|| "EVM RPC nonce 不是 hex quantity".to_owned())?;
    let fallback_gas_price = if gas_price.is_none() {
        Some(
            rpc::rpc_result(
                &client,
                url.as_str(),
                "eth_gasPrice",
                serde_json::json!([]),
                102,
            )
            .await?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "EVM RPC gasPrice 不是 hex quantity".to_owned())?,
        )
    } else {
        None
    };
    let raw_transaction = onchain_signer::sign(
        &config.chain,
        &config.wallet_address,
        transaction,
        Some(&nonce),
        fallback_gas_price.as_deref(),
    )?;
    let raw = hex::decode(raw_transaction.trim_start_matches("0x"))
        .map_err(|_| "EVM 已签名交易不是合法 hex".to_owned())?;
    let local_transaction_id = format!("0x{}", hex::encode(common::signing::keccak256(&raw)));
    Ok(PreparedChainSubmission::EvmRpc {
        raw_transaction,
        rpc_url,
        local_transaction_id,
    })
}

pub(super) async fn broadcast(prepared: PreparedChainSubmission) -> ChainSubmissionOutcome {
    match prepared {
        PreparedChainSubmission::Jupiter {
            signed_transaction,
            request_id,
            last_valid_block_height,
            api_key,
            local_transaction_id,
        } => {
            broadcast_jupiter(
                signed_transaction,
                request_id,
                last_valid_block_height,
                api_key,
                local_transaction_id,
            )
            .await
        }
        PreparedChainSubmission::SolanaRpc {
            signed_transaction,
            rpc_url,
            local_transaction_id,
        } => broadcast_solana_rpc(signed_transaction, rpc_url, local_transaction_id).await,
        PreparedChainSubmission::EvmRpc {
            raw_transaction,
            rpc_url,
            local_transaction_id,
        } => broadcast_evm_rpc(raw_transaction, rpc_url, local_transaction_id).await,
    }
}

pub(super) async fn recheck(
    prepared: &PreparedChainSubmission,
    transaction_id: &str,
) -> ChainSubmissionOutcome {
    match prepared {
        PreparedChainSubmission::Jupiter { .. } => {
            query_solana_finality(SOLANA_MAINNET_RPC, transaction_id).await
        }
        PreparedChainSubmission::SolanaRpc { rpc_url, .. } => {
            query_solana_finality(rpc_url, transaction_id).await
        }
        PreparedChainSubmission::EvmRpc { rpc_url, .. } => {
            query_evm_finality(rpc_url, transaction_id).await
        }
    }
}

pub(super) async fn recheck_transaction(
    state: &AppState,
    config: &OnchainComparisonConfig,
    transaction_id: &str,
) -> ChainSubmissionOutcome {
    if config.chain.eq_ignore_ascii_case("solana") {
        let rpc_url = if config.rpc.mode == OnchainRpcMode::Custom {
            match require_verified_custom_rpc(state, config).await {
                Ok(url) => url,
                Err(problem) => return pending(transaction_id.to_owned(), problem),
            }
        } else {
            SOLANA_MAINNET_RPC.to_owned()
        };
        return query_solana_finality(&rpc_url, transaction_id).await;
    }
    let rpc_url = match require_verified_custom_rpc(state, config).await {
        Ok(url) => url,
        Err(problem) => return pending(transaction_id.to_owned(), problem),
    };
    query_evm_finality(&rpc_url, transaction_id).await
}

async fn query_solana_finality(rpc_url: &str, transaction_id: &str) -> ChainSubmissionOutcome {
    let (url, client) = match rpc_target::rpc_target(rpc_url).await {
        Ok(target) => target,
        Err(problem) => return pending(transaction_id.to_owned(), problem),
    };
    poll_solana_finality(&client, url.as_str(), transaction_id.to_owned()).await
}

async fn query_evm_finality(rpc_url: &str, transaction_id: &str) -> ChainSubmissionOutcome {
    let (url, client) = match rpc_target::rpc_target(rpc_url).await {
        Ok(target) => target,
        Err(problem) => return pending(transaction_id.to_owned(), problem),
    };
    poll_evm_finality(&client, url.as_str(), transaction_id.to_owned()).await
}

pub(super) fn require_custom_rpc(
    state: &AppState,
    config: &OnchainComparisonConfig,
) -> Result<String, String> {
    if let Some(url) = onchain_rpc_registry::configured_url(&config.chain) {
        return Ok(url);
    }
    if config.rpc.mode == OnchainRpcMode::Custom
        && state
            .onchain_monitor()
            .snapshot()
            .config
            .chain
            .eq_ignore_ascii_case(&config.chain)
    {
        if let Some(url) = state.onchain_monitor().custom_rpc_url() {
            return Ok(url.as_str().to_owned());
        }
    }
    Err(format!(
        "{} 链上提交缺少该链独立的已核验自定义 RPC",
        config.chain
    ))
}

pub(super) async fn require_verified_custom_rpc(
    state: &AppState,
    config: &OnchainComparisonConfig,
) -> Result<String, String> {
    let url = require_custom_rpc(state, config)?;
    let now_ms = common::time::now_ms();
    let cache_key = rpc_verify_cache_key(&config.chain, &url);
    let cache = rpc_verify_cache();
    if cache
        .get(&cache_key)
        .is_some_and(|verified_at| now_ms.saturating_sub(*verified_at) <= RPC_VERIFY_CACHE_MS)
    {
        return Ok(url);
    }
    let mut probe_config = config.clone();
    probe_config.rpc.mode = OnchainRpcMode::Custom;
    let status = rpc::probe(&probe_config, Some(&url), now_ms).await;
    if status.ready {
        if cache.len() >= RPC_VERIFY_CACHE_MAX {
            cache.clear();
        }
        cache.insert(cache_key, status.observed_at_ms.unwrap_or(now_ms));
        Ok(url)
    } else {
        Err(status
            .problem
            .unwrap_or_else(|| "自定义 RPC 尚未通过链 ID 与最新区块核验".to_owned()))
    }
}

fn rpc_verify_cache() -> &'static dashmap::DashMap<String, i64> {
    static CACHE: OnceLock<dashmap::DashMap<String, i64>> = OnceLock::new();
    CACHE.get_or_init(dashmap::DashMap::new)
}

fn rpc_verify_cache_key(chain: &str, url: &str) -> String {
    let canonical = format!("{}\n{}", chain.trim().to_ascii_lowercase(), url.trim());
    common::signing::hmac_sha256_hex(RPC_VERIFY_CACHE_KEY, canonical.as_bytes())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JupiterExecuteRequest<'a> {
    signed_transaction: &'a str,
    request_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_valid_block_height: Option<String>,
}

#[derive(Deserialize)]
struct JupiterExecuteResponse {
    #[serde(default)]
    status: String,
    #[serde(default)]
    signature: String,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    code: Option<i64>,
}

async fn broadcast_jupiter(
    signed_transaction: String,
    request_id: String,
    last_valid_block_height: Option<u64>,
    api_key: String,
    local_transaction_id: String,
) -> ChainSubmissionOutcome {
    let response = quote_client()
        .post(JUPITER_EXECUTE_ENDPOINT)
        .header("x-api-key", api_key)
        .json(&JupiterExecuteRequest {
            signed_transaction: &signed_transaction,
            request_id: &request_id,
            last_valid_block_height: last_valid_block_height.map(|value| value.to_string()),
        })
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return pending(
                local_transaction_id,
                format!("Jupiter execute 传输结果不确定：{error}"),
            )
        }
    };
    let status = response.status();
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            return pending(
                local_transaction_id,
                format!("Jupiter execute 响应读取失败：{error}"),
            )
        }
    };
    let decoded: JupiterExecuteResponse = match serde_json::from_slice(&body) {
        Ok(decoded) => decoded,
        Err(error) => {
            return pending(
                local_transaction_id,
                format!("Jupiter execute 响应无法解析：HTTP {status} · {error}"),
            )
        }
    };
    let transaction_id = if decoded.signature.trim().is_empty() {
        local_transaction_id.clone()
    } else {
        decoded.signature
    };
    if status.is_success() && decoded.status.eq_ignore_ascii_case("success") {
        return ChainSubmissionOutcome::Confirmed { transaction_id };
    }
    let problem = format!(
        "Jupiter execute 未确认成功：HTTP {status} · status={} · code={} · error={}",
        decoded.status,
        decoded
            .code
            .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
        decoded
            .error
            .map_or_else(|| "unknown".to_owned(), |error| error.to_string())
    );
    if jupiter_result_is_ambiguous(decoded.code) {
        pending(transaction_id, problem)
    } else {
        ChainSubmissionOutcome::Rejected {
            transaction_id,
            problem,
        }
    }
}

fn jupiter_result_is_ambiguous(code: Option<i64>) -> bool {
    matches!(code, None | Some(-1001 | -2001))
}

async fn broadcast_solana_rpc(
    signed_transaction: String,
    rpc_url: String,
    local_transaction_id: String,
) -> ChainSubmissionOutcome {
    let (url, client) = match rpc_target::rpc_target(&rpc_url).await {
        Ok(target) => target,
        Err(problem) => return pending(local_transaction_id, problem),
    };
    let transaction_id = match rpc::rpc_result(
        &client,
        url.as_str(),
        "sendTransaction",
        serde_json::json!([
            signed_transaction,
            {
                "encoding": "base64",
                "skipPreflight": false,
                "preflightCommitment": "confirmed",
                "maxRetries": 2
            }
        ]),
        201,
    )
    .await
    {
        Ok(value) => value
            .as_str()
            .map(str::to_owned)
            .unwrap_or(local_transaction_id),
        Err(problem) => return pending(local_transaction_id, problem),
    };
    poll_solana_finality(&client, url.as_str(), transaction_id).await
}

async fn poll_solana_finality(
    client: &reqwest::Client,
    rpc_url: &str,
    transaction_id: String,
) -> ChainSubmissionOutcome {
    for index in 0..FINALITY_POLLS {
        tokio::time::sleep(std::time::Duration::from_millis(FINALITY_POLL_MS)).await;
        let Ok(value) = rpc::rpc_result(
            client,
            rpc_url,
            "getSignatureStatuses",
            serde_json::json!([[transaction_id], { "searchTransactionHistory": true }]),
            220 + index as u64,
        )
        .await
        else {
            continue;
        };
        let status = value
            .get("value")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .filter(|row| !row.is_null());
        let Some(status) = status else { continue };
        if status.get("err").is_some_and(|error| !error.is_null()) {
            return ChainSubmissionOutcome::Rejected {
                transaction_id,
                problem: format!("Solana 交易执行失败：{}", status["err"]),
            };
        }
        if status
            .get("confirmationStatus")
            .and_then(Value::as_str)
            .is_some_and(|value| matches!(value, "confirmed" | "finalized"))
        {
            return ChainSubmissionOutcome::Confirmed { transaction_id };
        }
    }
    pending(
        transaction_id,
        "Solana 交易已广播，但确认状态仍在等待".to_owned(),
    )
}

async fn broadcast_evm_rpc(
    raw_transaction: String,
    rpc_url: String,
    local_transaction_id: String,
) -> ChainSubmissionOutcome {
    let (url, client) = match rpc_target::rpc_target(&rpc_url).await {
        Ok(target) => target,
        Err(problem) => return pending(local_transaction_id, problem),
    };
    let transaction_id = match rpc::rpc_result(
        &client,
        url.as_str(),
        "eth_sendRawTransaction",
        serde_json::json!([raw_transaction]),
        301,
    )
    .await
    {
        Ok(value) => value
            .as_str()
            .map(str::to_owned)
            .unwrap_or(local_transaction_id),
        Err(problem) => return pending(local_transaction_id, problem),
    };
    poll_evm_finality(&client, url.as_str(), transaction_id).await
}

async fn poll_evm_finality(
    client: &reqwest::Client,
    rpc_url: &str,
    transaction_id: String,
) -> ChainSubmissionOutcome {
    for index in 0..FINALITY_POLLS {
        tokio::time::sleep(std::time::Duration::from_millis(FINALITY_POLL_MS)).await;
        let Ok(receipt) = rpc::rpc_result(
            client,
            rpc_url,
            "eth_getTransactionReceipt",
            serde_json::json!([transaction_id]),
            320 + index as u64,
        )
        .await
        else {
            continue;
        };
        if receipt.is_null() {
            continue;
        }
        match receipt.get("status").and_then(Value::as_str) {
            Some("0x1") => return ChainSubmissionOutcome::Confirmed { transaction_id },
            Some("0x0") => {
                return ChainSubmissionOutcome::Rejected {
                    transaction_id,
                    problem: "EVM 交易 receipt status=0x0".to_owned(),
                }
            }
            _ => {}
        }
    }
    pending(
        transaction_id,
        "EVM 交易已广播，但 receipt 仍在等待".to_owned(),
    )
}

fn pending(transaction_id: String, problem: String) -> ChainSubmissionOutcome {
    ChainSubmissionOutcome::Pending {
        transaction_id,
        problem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_broadcast_failures_are_ambiguous_not_definitive_rejections() {
        let outcome = pending("0xabc".to_owned(), "timeout".to_owned());
        assert_eq!(
            outcome,
            ChainSubmissionOutcome::Pending {
                transaction_id: "0xabc".to_owned(),
                problem: "timeout".to_owned(),
            }
        );
    }

    #[test]
    fn only_jupiter_unknown_results_remain_ambiguous() {
        assert!(jupiter_result_is_ambiguous(None));
        assert!(jupiter_result_is_ambiguous(Some(-1001)));
        assert!(jupiter_result_is_ambiguous(Some(-2001)));
        assert!(!jupiter_result_is_ambiguous(Some(-1000)));
        assert!(!jupiter_result_is_ambiguous(Some(-1002)));
        assert!(!jupiter_result_is_ambiguous(Some(0)));
    }
}
