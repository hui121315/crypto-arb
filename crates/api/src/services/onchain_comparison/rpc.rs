use super::rpc_target::{endpoint_label, rpc_target};
use serde::Deserialize;
use serde_json::Value;
use shared_types::{
    onchain_chain_preset, OnchainComparisonConfig, OnchainRpcMode, OnchainRpcStatus,
};

pub(super) const EVM_RPC_DOCS: &str = "https://ethereum.org/developers/docs/apis/json-rpc/";
pub(super) const SOLANA_RPC_DOCS: &str = "https://solana.com/docs/rpc/http/getgenesishash";
pub(super) const RPC_PROBE_INTERVAL_MS: i64 = 5_000;

const MAX_RPC_RESPONSE_BYTES: usize = 64 * 1024;
// RPC returns the full hash, not the 32-character CAIP-2 chain reference.
pub(super) const SOLANA_MAINNET_GENESIS_HASH: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[serde(default, deserialize_with = "present_result")]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

// A legitimate null means "not found yet", not a malformed response without result.
fn present_result<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(decoder).map(Some)
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub(super) fn probe_due(
    config: &OnchainComparisonConfig,
    status: &OnchainRpcStatus,
    now_ms: i64,
) -> bool {
    if config.rpc.mode != OnchainRpcMode::Custom {
        return false;
    }
    let expected = onchain_chain_preset(&config.chain).and_then(|preset| preset.chain_id);
    status.mode != OnchainRpcMode::Custom
        || status.expected_chain_id != expected
        || status
            .observed_at_ms
            .is_none_or(|observed| now_ms.saturating_sub(observed) >= RPC_PROBE_INTERVAL_MS)
}

pub(super) async fn probe(
    config: &OnchainComparisonConfig,
    rpc_url: Option<&str>,
    started_at_ms: i64,
) -> OnchainRpcStatus {
    let expected_chain_id = onchain_chain_preset(&config.chain).and_then(|preset| preset.chain_id);
    if config.rpc.mode != OnchainRpcMode::Custom {
        return provider_managed_status(expected_chain_id);
    }
    let endpoint_label = rpc_url.and_then(endpoint_label);
    let Some(rpc_url) = rpc_url else {
        return custom_problem(
            endpoint_label,
            expected_chain_id,
            "custom RPC endpoint is not configured",
            started_at_ms,
        );
    };
    let (url, client) = match rpc_target(rpc_url).await {
        Ok(target) => target,
        Err(problem) => {
            return custom_problem(endpoint_label, expected_chain_id, &problem, started_at_ms)
        }
    };
    if expected_chain_id.is_none() {
        return probe_solana(&client, url.as_str(), endpoint_label, started_at_ms).await;
    }
    probe_evm(
        &client,
        url.as_str(),
        endpoint_label,
        expected_chain_id,
        started_at_ms,
    )
    .await
}

async fn probe_evm(
    client: &reqwest::Client,
    rpc_url: &str,
    endpoint_label: Option<String>,
    expected_chain_id: Option<u64>,
    started_at_ms: i64,
) -> OnchainRpcStatus {
    let chain = rpc_result(client, rpc_url, "eth_chainId", serde_json::json!([]), 1);
    let block = rpc_result(client, rpc_url, "eth_blockNumber", serde_json::json!([]), 2);
    let (chain, block) = tokio::join!(chain, block);
    let observed_at_ms = common::time::now_ms();
    let latency_ms = observed_at_ms.saturating_sub(started_at_ms);
    let observed_chain_id = match chain.and_then(|value| {
        value
            .as_str()
            .ok_or_else(|| "custom RPC chain id is not a string".to_owned())
            .and_then(|value| parse_quantity(value, "chain id"))
    }) {
        Ok(value) => value,
        Err(problem) => {
            return custom_problem_at(
                endpoint_label,
                expected_chain_id,
                &problem,
                observed_at_ms,
                latency_ms,
            )
        }
    };
    let block_number = match block.and_then(|value| {
        value
            .as_str()
            .ok_or_else(|| "custom RPC block number is not a string".to_owned())
            .and_then(|value| parse_quantity(value, "block number"))
    }) {
        Ok(value) => value,
        Err(problem) => {
            return custom_problem_at(
                endpoint_label,
                expected_chain_id,
                &problem,
                observed_at_ms,
                latency_ms,
            )
        }
    };
    if expected_chain_id != Some(observed_chain_id) {
        return OnchainRpcStatus {
            mode: OnchainRpcMode::Custom,
            configured: true,
            endpoint_label,
            expected_chain_id,
            observed_chain_id: Some(observed_chain_id),
            block_number: Some(block_number),
            latency_ms: Some(latency_ms),
            observed_at_ms: Some(observed_at_ms),
            ready: false,
            problem: Some(format!(
                "RPC chain id {observed_chain_id} does not match configured chain {}",
                expected_chain_id.map_or_else(|| "unknown".to_owned(), |id| id.to_string())
            )),
            official_docs_url: EVM_RPC_DOCS.to_owned(),
        };
    }
    OnchainRpcStatus {
        mode: OnchainRpcMode::Custom,
        configured: true,
        endpoint_label,
        expected_chain_id,
        observed_chain_id: Some(observed_chain_id),
        block_number: Some(block_number),
        latency_ms: Some(latency_ms),
        observed_at_ms: Some(observed_at_ms),
        ready: true,
        problem: None,
        official_docs_url: EVM_RPC_DOCS.to_owned(),
    }
}

async fn probe_solana(
    client: &reqwest::Client,
    rpc_url: &str,
    endpoint_label: Option<String>,
    started_at_ms: i64,
) -> OnchainRpcStatus {
    let genesis = rpc_result(client, rpc_url, "getGenesisHash", serde_json::json!([]), 1);
    let slot = rpc_result(
        client,
        rpc_url,
        "getSlot",
        serde_json::json!([{ "commitment": "finalized" }]),
        2,
    );
    let (genesis, slot) = tokio::join!(genesis, slot);
    let observed_at_ms = common::time::now_ms();
    let latency_ms = observed_at_ms.saturating_sub(started_at_ms);
    let genesis = match genesis.and_then(|value| {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "custom Solana RPC genesis hash is not a string".to_owned())
    }) {
        Ok(value) => value,
        Err(problem) => {
            return custom_problem_at(endpoint_label, None, &problem, observed_at_ms, latency_ms)
        }
    };
    let slot = match slot.and_then(|value| {
        value
            .as_u64()
            .ok_or_else(|| "custom Solana RPC slot is not an unsigned integer".to_owned())
    }) {
        Ok(value) => value,
        Err(problem) => {
            return custom_problem_at(endpoint_label, None, &problem, observed_at_ms, latency_ms)
        }
    };
    if genesis != SOLANA_MAINNET_GENESIS_HASH {
        return custom_problem_at(
            endpoint_label,
            None,
            "custom Solana RPC is not connected to mainnet-beta",
            observed_at_ms,
            latency_ms,
        );
    }
    OnchainRpcStatus {
        mode: OnchainRpcMode::Custom,
        configured: true,
        endpoint_label,
        expected_chain_id: None,
        observed_chain_id: None,
        block_number: Some(slot),
        latency_ms: Some(latency_ms),
        observed_at_ms: Some(observed_at_ms),
        ready: true,
        problem: None,
        official_docs_url: SOLANA_RPC_DOCS.to_owned(),
    }
}

fn provider_managed_status(expected_chain_id: Option<u64>) -> OnchainRpcStatus {
    OnchainRpcStatus {
        mode: OnchainRpcMode::ProviderManaged,
        expected_chain_id,
        official_docs_url: rpc_docs(expected_chain_id).to_owned(),
        ..OnchainRpcStatus::default()
    }
}

fn custom_problem(
    endpoint_label: Option<String>,
    expected_chain_id: Option<u64>,
    problem: &str,
    started_at_ms: i64,
) -> OnchainRpcStatus {
    let observed_at_ms = common::time::now_ms();
    custom_problem_at(
        endpoint_label,
        expected_chain_id,
        problem,
        observed_at_ms,
        observed_at_ms.saturating_sub(started_at_ms),
    )
}

fn custom_problem_at(
    endpoint_label: Option<String>,
    expected_chain_id: Option<u64>,
    problem: &str,
    observed_at_ms: i64,
    latency_ms: i64,
) -> OnchainRpcStatus {
    OnchainRpcStatus {
        mode: OnchainRpcMode::Custom,
        configured: endpoint_label.is_some(),
        endpoint_label,
        expected_chain_id,
        observed_chain_id: None,
        block_number: None,
        latency_ms: Some(latency_ms),
        observed_at_ms: Some(observed_at_ms),
        ready: false,
        problem: Some(problem.to_owned()),
        official_docs_url: rpc_docs(expected_chain_id).to_owned(),
    }
}

const fn rpc_docs(expected_chain_id: Option<u64>) -> &'static str {
    if expected_chain_id.is_some() {
        EVM_RPC_DOCS
    } else {
        SOLANA_RPC_DOCS
    }
}

pub(super) async fn rpc_result(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
    id: u64,
) -> Result<Value, String> {
    rpc_result_with_limit(client, rpc_url, method, params, id, MAX_RPC_RESPONSE_BYTES).await
}

pub(super) async fn rpc_result_with_limit(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
    id: u64,
    max_response_bytes: usize,
) -> Result<Value, String> {
    let mut response = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        }))
        .send()
        .await
        .map_err(|error| rpc_transport_problem(&error))?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > max_response_bytes as u64)
    {
        return Err(format!("RPC {method} response is too large"));
    }
    let mut body = Vec::with_capacity(max_response_bytes.min(64 * 1024));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| format!("RPC {method} response could not be read"))?
    {
        if body.len().saturating_add(chunk.len()) > max_response_bytes {
            return Err(format!("RPC {method} response is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(format!("RPC {method} returned HTTP {status}"));
    }
    let decoded: JsonRpcResponse = serde_json::from_slice(&body)
        .map_err(|_| format!("RPC {method} returned invalid JSON-RPC"))?;
    if let Some(error) = decoded.error {
        return Err(format!(
            "RPC {method} returned code {}: {}",
            error.code, error.message
        ));
    }
    decoded
        .result
        .ok_or_else(|| format!("RPC {method} response is missing result"))
}

fn parse_quantity(value: &str, label: &str) -> Result<u64, String> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| format!("custom RPC {label} is not a hex quantity"))?;
    if hex.is_empty() {
        return Err(format!("custom RPC {label} is empty"));
    }
    u64::from_str_radix(hex, 16).map_err(|_| format!("custom RPC {label} exceeds u64"))
}

fn rpc_transport_problem(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "RPC request timed out".to_owned()
    } else if error.is_connect() {
        "RPC connection failed".to_owned()
    } else {
        "RPC transport failed".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn solana_mainnet_identity_requires_full_rpc_hash_not_caip_reference() {
        use axum::{routing::post, Json, Router};
        let genesis = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        let captured = genesis.clone();
        let router=Router::new().route("/",post(move|Json(body):Json<Value>|{
            let genesis=captured.clone();async move {Json(serde_json::json!({"jsonrpc":"2.0","id":body["id"],"result":
                if body["method"]=="getGenesisHash"{Value::String(genesis.lock().clone())}else{Value::from(400000000u64)}}))}
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let mut actual = vec![];
        for value in [
            "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
            "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
            "EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
        ] {
            *genesis.lock() = value.into();
            actual.push(
                probe_solana(&client, &url, None, common::time::now_ms())
                    .await
                    .ready,
            );
        }
        server.abort();
        assert_eq!(actual, [true, false, false]);
        assert_eq!(
            bs58::decode(SOLANA_MAINNET_GENESIS_HASH)
                .into_vec()
                .unwrap()
                .len(),
            32
        );
    }

    #[tokio::test]
    #[ignore = "explicit public read-only Solana RPC probe; no wallets, signing or funds"]
    async fn solana_mainnet_public_rpc_identity_probe() {
        let mut config = OnchainComparisonConfig::default();
        config.chain = "solana".into();
        config.rpc.mode = OnchainRpcMode::Custom;
        let status = probe(
            &config,
            Some("https://api.mainnet-beta.solana.com"),
            common::time::now_ms(),
        )
        .await;
        assert!(status.ready, "{:?}", status.problem);
        assert!(status.block_number.is_some_and(|n| n > 0));
        eprintln!(
            "public mainnet identity accepted; finalized slot={:?}",
            status.block_number
        );
    }

    #[test]
    fn rpc_result_distinguishes_pending_null_from_missing_or_error() {
        let parse = |s| serde_json::from_str::<JsonRpcResponse>(s).unwrap();
        assert_eq!(parse(r#"{"result":null}"#).result, Some(Value::Null));
        assert_eq!(parse(r#"{"result":0}"#).result, Some(Value::from(0)));
        assert!(parse("{}").result.is_none());
        let failure = parse(r#"{"error":{"code":-32000,"message":"unavailable"}}"#);
        assert!(failure.result.is_none());
        assert_eq!(failure.error.unwrap().code, -32000);
    }

    #[test]
    fn rpc_quantities_require_canonical_hex_shape() {
        assert_eq!(parse_quantity("0x2105", "chain id"), Ok(8_453));
        assert!(parse_quantity("8453", "chain id").is_err());
        assert!(parse_quantity("0x", "block number").is_err());
    }
}
