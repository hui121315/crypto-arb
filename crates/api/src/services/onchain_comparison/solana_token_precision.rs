use super::rpc_target::{endpoint_label, rpc_target};
use serde::Deserialize;

pub(super) const SOLANA_TOKEN_SUPPLY_DOCS: &str = "https://solana.com/docs/rpc/http/gettokensupply";
pub(super) const SOLANA_MAINNET_RPC: &str = "https://api.mainnet-beta.solana.com";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SolanaTokenPrecision {
    pub(super) decimals: u8,
    pub(super) source: String,
    pub(super) observed_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<TokenSupplyResult>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct TokenSupplyResult {
    value: TokenSupplyValue,
}

#[derive(Debug, Deserialize)]
struct TokenSupplyValue {
    decimals: u8,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub(super) async fn resolve(
    address: &str,
    custom_rpc_url: Option<&str>,
) -> Result<SolanaTokenPrecision, String> {
    let rpc_url = custom_rpc_url.unwrap_or(SOLANA_MAINNET_RPC);
    let source = custom_rpc_url.map_or_else(
        || "solana_mainnet_rpc".to_owned(),
        |url| {
            endpoint_label(url).map_or_else(
                || "custom_solana_rpc".to_owned(),
                |host| format!("custom_solana_rpc:{host}"),
            )
        },
    );
    let (url, client) = rpc_target(rpc_url).await?;
    let response = client
        .post(url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenSupply",
            "params": [address, { "commitment": "finalized" }]
        }))
        .send()
        .await
        .map_err(|error| format!("Solana getTokenSupply failed: {error}"))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|_| "Solana getTokenSupply response could not be read".to_owned())?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err("Solana getTokenSupply response is too large".to_owned());
    }
    if !status.is_success() {
        return Err(format!("Solana getTokenSupply returned HTTP {status}"));
    }
    let decimals = decode_decimals(&body)?;
    Ok(SolanaTokenPrecision {
        decimals,
        source,
        observed_at_ms: common::time::now_ms(),
    })
}

fn decode_decimals(body: &[u8]) -> Result<u8, String> {
    let response: JsonRpcResponse = serde_json::from_slice(body)
        .map_err(|_| "Solana getTokenSupply returned invalid JSON-RPC".to_owned())?;
    if let Some(error) = response.error {
        return Err(format!(
            "Solana getTokenSupply returned code {}: {}",
            error.code, error.message
        ));
    }
    let decimals = response
        .result
        .map(|result| result.value.decimals)
        .ok_or_else(|| "Solana getTokenSupply returned no token amount".to_owned())?;
    if decimals > 18 {
        return Err("Solana token decimals exceed the supported range 0..=18".to_owned());
    }
    Ok(decimals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_official_token_supply_shape() {
        let body = br#"{
          "jsonrpc":"2.0",
          "result":{"context":{"slot":1114},"value":{"amount":"100000","decimals":6,"uiAmount":0.1,"uiAmountString":"0.1"}},
          "id":1
        }"#;
        assert_eq!(decode_decimals(body), Ok(6));
    }

    #[test]
    fn rejects_rpc_errors_and_unsupported_precision() {
        let error =
            br#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid param"},"id":1}"#;
        let oversized = br#"{"jsonrpc":"2.0","result":{"value":{"decimals":19}},"id":1}"#;
        assert!(decode_decimals(error).is_err());
        assert!(decode_decimals(oversized).is_err());
    }
}
