#[cfg(test)]
use super::rpc::EVM_RPC_DOCS;
use super::rpc_target::{endpoint_label, rpc_target};
use serde::Deserialize;
use shared_types::{onchain_chain_preset, OnchainTokenIdentity, OnchainTokenResolution};

const ERC20_STANDARD_URL: &str = "https://eips.ethereum.org/EIPS/eip-20";
const MAX_RPC_RESPONSE_BYTES: usize = 128 * 1024;
const OPTIONAL_IDENTITY_WAIT: std::time::Duration = std::time::Duration::from_millis(1_500);
const SYMBOL_SELECTOR: &str = "0x95d89b41";
const DECIMALS_SELECTOR: &str = "0x313ce567";
const NAME_SELECTOR: &str = "0x06fdde03";

#[derive(Clone, Copy)]
pub(super) struct PublicRpc {
    pub(super) url: &'static str,
    pub(super) source: &'static str,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: u64,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub(super) async fn resolve(
    chain: &str,
    address: &str,
    custom_rpc_url: Option<&str>,
) -> Result<OnchainTokenResolution, String> {
    let preset = onchain_chain_preset(chain)
        .filter(|preset| preset.chain_id.is_some())
        .ok_or_else(|| format!("{chain} is not a supported EVM chain"))?;
    let expected_chain_id = preset.chain_id.unwrap_or_default();
    let public = public_rpc(chain);
    let rpc_url = custom_rpc_url
        .or_else(|| public.map(|endpoint| endpoint.url))
        .ok_or_else(|| format!("{chain} 合约识别需要先配置可信公网 HTTPS RPC"))?;
    let source = custom_rpc_url.map_or_else(
        || {
            public
                .map_or("public_rpc", |endpoint| endpoint.source)
                .to_owned()
        },
        |url| {
            endpoint_label(url).map_or_else(
                || "custom_rpc".to_owned(),
                |host| format!("custom_rpc:{host}"),
            )
        },
    );
    let rows = fetch_metadata(rpc_url, address).await?;
    let observed_chain_id = parse_quantity(required_result(&rows, 1, "eth_chainId")?)?;
    if observed_chain_id != expected_chain_id {
        return Err(format!(
            "RPC chain id {observed_chain_id} does not match {chain} ({expected_chain_id})"
        ));
    }
    let code = required_result(&rows, 2, "eth_getCode")?;
    if matches!(code, "0x" | "0x0") {
        return Err("地址没有 EVM 合约字节码，无法读取 ERC-20 身份".to_owned());
    }
    project_metadata(preset.id, address, source, &rows)
}

fn project_metadata(
    chain: &str,
    address: &str,
    source: String,
    rows: &[JsonRpcResponse],
) -> Result<OnchainTokenResolution, String> {
    let decimals = decode_decimals(required_result(rows, 4, "decimals()")?)?;
    let observed_at_ms = common::time::now_ms();
    let symbol =
        match required_result(rows, 3, "symbol()").and_then(|value| decode_abi_string(value, 32)) {
            Ok(symbol) => symbol,
            Err(problem) => {
                return Ok(OnchainTokenResolution {
                    chain: chain.to_owned(),
                    address: address.to_owned(),
                    decimals,
                    precision_source: source,
                    precision_evidence_url: ERC20_STANDARD_URL.to_owned(),
                    identity: None,
                    identity_problem: Some(format!(
                        "ERC-20 decimals() 已读取，但 symbol() 没有返回可用符号：{problem}"
                    )),
                    observed_at_ms,
                });
            }
        };
    let name = optional_result(rows, 5).and_then(|value| decode_abi_string(value, 96).ok());
    Ok(OnchainTokenResolution::complete(OnchainTokenIdentity {
        chain: chain.to_owned(),
        address: address.to_owned(),
        symbol: symbol.to_ascii_uppercase(),
        name,
        decimals,
        source,
        evidence_url: ERC20_STANDARD_URL.to_owned(),
        verified: false,
        native: false,
        observed_at_ms,
    }))
}

async fn fetch_metadata(rpc_url: &str, address: &str) -> Result<Vec<JsonRpcResponse>, String> {
    let (url, client) = rpc_target(rpc_url).await?;
    let precision_calls = serde_json::json!([
        {"jsonrpc": "2.0", "method": "eth_chainId", "params": [], "id": 1},
        {"jsonrpc": "2.0", "method": "eth_getCode", "params": [address, "latest"], "id": 2},
        {"jsonrpc": "2.0", "method": "eth_call", "params": [{"to": address, "data": DECIMALS_SELECTOR}, "latest"], "id": 4}
    ]);
    let identity_calls = serde_json::json!([
        {"jsonrpc": "2.0", "method": "eth_call", "params": [{"to": address, "data": SYMBOL_SELECTOR}, "latest"], "id": 3},
        {"jsonrpc": "2.0", "method": "eth_call", "params": [{"to": address, "data": NAME_SELECTOR}, "latest"], "id": 5}
    ]);
    let precision_url = url.clone();
    let precision_client = client.clone();
    let (precision, identity) = tokio::join!(
        fetch_rpc_batch(precision_client, precision_url, precision_calls),
        tokio::time::timeout(
            OPTIONAL_IDENTITY_WAIT,
            fetch_rpc_batch(client, url, identity_calls),
        )
    );
    let mut rows = precision?;
    match identity {
        Ok(Ok(identity)) => rows.extend(identity),
        Ok(Err(problem)) => rows.push(identity_problem_row(problem)),
        Err(_) => rows.push(identity_problem_row(format!(
            "optional ERC-20 symbol/name metadata exceeded {}ms",
            OPTIONAL_IDENTITY_WAIT.as_millis()
        ))),
    }
    Ok(rows)
}

async fn fetch_rpc_batch(
    client: reqwest::Client,
    url: reqwest::Url,
    calls: serde_json::Value,
) -> Result<Vec<JsonRpcResponse>, String> {
    let response = client
        .post(url)
        .json(&calls)
        .send()
        .await
        .map_err(|error| format!("token metadata RPC failed: {error}"))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|_| "token metadata RPC response could not be read".to_owned())?;
    if body.len() > MAX_RPC_RESPONSE_BYTES {
        return Err("token metadata RPC response is too large".to_owned());
    }
    if !status.is_success() {
        return Err(format!("token metadata RPC returned HTTP {status}"));
    }
    serde_json::from_slice(&body)
        .map_err(|_| "token metadata RPC returned invalid JSON-RPC batch".to_owned())
}

fn identity_problem_row(message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        id: 3,
        result: None,
        error: Some(JsonRpcError {
            code: -32_000,
            message,
        }),
    }
}

fn required_result<'a>(
    rows: &'a [JsonRpcResponse],
    id: u64,
    label: &str,
) -> Result<&'a str, String> {
    let row = rows
        .iter()
        .find(|row| row.id == id)
        .ok_or_else(|| format!("token metadata RPC is missing {label}"))?;
    if let Some(error) = row.error.as_ref() {
        return Err(format!(
            "token metadata {label} returned code {}: {}",
            error.code, error.message
        ));
    }
    row.result
        .as_deref()
        .ok_or_else(|| format!("token metadata {label} has no result"))
}

fn optional_result(rows: &[JsonRpcResponse], id: u64) -> Option<&str> {
    rows.iter()
        .find(|row| row.id == id && row.error.is_none())
        .and_then(|row| row.result.as_deref())
}

fn parse_quantity(value: &str) -> Result<u64, String> {
    value
        .strip_prefix("0x")
        .filter(|hex| !hex.is_empty())
        .ok_or_else(|| "RPC chain id is not a hex quantity".to_owned())
        .and_then(|hex| {
            u64::from_str_radix(hex, 16).map_err(|_| "RPC chain id exceeds u64".to_owned())
        })
}

fn decode_decimals(value: &str) -> Result<u8, String> {
    let bytes = decode_hex(value, "decimals()")?;
    if bytes.len() != 32 || bytes[..31].iter().any(|byte| *byte != 0) || bytes[31] > 18 {
        return Err(
            "ERC-20 decimals() must return a supported uint8 value from 0 to 18".to_owned(),
        );
    }
    Ok(bytes[31])
}

fn decode_abi_string(value: &str, max_chars: usize) -> Result<String, String> {
    let bytes = decode_hex(value, "string metadata")?;
    let content = if bytes.len() == 32 {
        bytes
            .iter()
            .copied()
            .take_while(|byte| *byte != 0)
            .collect()
    } else {
        if bytes.len() < 64 {
            return Err("ERC-20 string metadata has invalid ABI length".to_owned());
        }
        let offset = word_usize(&bytes[..32])?;
        let length_end = offset
            .checked_add(32)
            .ok_or_else(|| "ERC-20 string metadata offset overflow".to_owned())?;
        if length_end > bytes.len() {
            return Err("ERC-20 string metadata offset is out of bounds".to_owned());
        }
        let length = word_usize(&bytes[offset..length_end])?;
        let end = length_end
            .checked_add(length)
            .ok_or_else(|| "ERC-20 string metadata length overflow".to_owned())?;
        if end > bytes.len() || length > 256 {
            return Err("ERC-20 string metadata is out of bounds".to_owned());
        }
        bytes[length_end..end].to_vec()
    };
    let decoded =
        String::from_utf8(content).map_err(|_| "ERC-20 string metadata is not UTF-8".to_owned())?;
    let trimmed = decoded.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || trimmed.chars().any(char::is_control)
    {
        return Err("ERC-20 string metadata is empty or unsupported".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn decode_hex(value: &str, label: &str) -> Result<Vec<u8>, String> {
    value
        .strip_prefix("0x")
        .ok_or_else(|| format!("ERC-20 {label} is not hex data"))
        .and_then(|value| hex::decode(value).map_err(|_| format!("ERC-20 {label} is invalid hex")))
}

fn word_usize(word: &[u8]) -> Result<usize, String> {
    if word.len() != 32 || word[..24].iter().any(|byte| *byte != 0) {
        return Err("ERC-20 ABI word exceeds usize".to_owned());
    }
    let mut tail = [0_u8; 8];
    tail.copy_from_slice(&word[24..]);
    usize::try_from(u64::from_be_bytes(tail))
        .map_err(|_| "ERC-20 ABI word exceeds usize".to_owned())
}

pub(super) fn public_rpc(chain: &str) -> Option<PublicRpc> {
    match chain {
        // Official endpoint catalog: https://ethereum.publicnode.com/
        "ethereum" => Some(PublicRpc {
            url: "https://ethereum-rpc.publicnode.com",
            source: "publicnode_ethereum_rpc",
        }),
        "arbitrum" => Some(PublicRpc {
            url: "https://arb1.arbitrum.io/rpc",
            source: "arbitrum_public_rpc",
        }),
        "base" => Some(PublicRpc {
            url: "https://mainnet.base.org",
            source: "base_public_rpc",
        }),
        "optimism" => Some(PublicRpc {
            url: "https://mainnet.optimism.io",
            source: "optimism_public_rpc",
        }),
        "polygon" => Some(PublicRpc {
            url: "https://polygon.drpc.org",
            source: "polygon_public_rpc",
        }),
        "bnb-smart-chain" => Some(PublicRpc {
            url: "https://bsc-dataseed.bnbchain.org",
            source: "bnb_chain_public_rpc",
        }),
        "avalanche" => Some(PublicRpc {
            url: "https://api.avax.network/ext/bc/C/rpc",
            source: "avalanche_public_rpc",
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_dynamic_symbol_and_uint8_decimals() {
        let symbol = "0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000045553444300000000000000000000000000000000000000000000000000000000";
        let decimals = "0x0000000000000000000000000000000000000000000000000000000000000006";
        assert_eq!(decode_abi_string(symbol, 32), Ok("USDC".to_owned()));
        assert_eq!(decode_decimals(decimals), Ok(6));
    }

    #[test]
    fn unsupported_metadata_fails_closed() {
        assert!(decode_abi_string("0x", 32).is_err());
        assert!(decode_decimals("0x13").is_err());
    }

    #[test]
    fn supported_evm_chains_have_public_precision_fallbacks() {
        assert_eq!(
            public_rpc("ethereum").map(|rpc| (rpc.url, rpc.source)),
            Some((
                "https://ethereum-rpc.publicnode.com",
                "publicnode_ethereum_rpc"
            ))
        );
        assert_eq!(
            public_rpc("base").map(|rpc| rpc.url),
            Some("https://mainnet.base.org")
        );
    }

    #[test]
    fn metadata_evidence_uses_the_erc20_standard() {
        assert_eq!(ERC20_STANDARD_URL, "https://eips.ethereum.org/EIPS/eip-20");
        assert_eq!(
            EVM_RPC_DOCS,
            "https://ethereum.org/developers/docs/apis/json-rpc/"
        );
    }

    #[test]
    fn missing_symbol_preserves_contract_precision_as_partial_evidence() {
        let rows = vec![
            JsonRpcResponse {
                id: 3,
                result: None,
                error: Some(JsonRpcError {
                    code: -32_000,
                    message: "execution reverted".to_owned(),
                }),
            },
            JsonRpcResponse {
                id: 4,
                result: Some(
                    "0x0000000000000000000000000000000000000000000000000000000000000006".to_owned(),
                ),
                error: None,
            },
        ];

        let resolution = project_metadata("base", "0xtoken", "custom_rpc".to_owned(), &rows)
            .expect("decimals evidence should survive a non-standard symbol response");
        assert_eq!(resolution.decimals, 6);
        assert!(resolution.identity.is_none());
        assert!(resolution
            .identity_problem
            .as_deref()
            .is_some_and(|problem| problem.contains("symbol()")));
    }
}
