use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use onchain_monitor::OnchainBridgeQuote;
use serde::Deserialize;
use shared_types::{OnchainUnsignedTransaction, EVM_NATIVE_TOKEN_ADDRESS};

use super::provider_runtime::env_key;
use super::quote::{decode_response, quote_client};

mod recovery_quote;
pub(super) use recovery_quote::{fetch as fetch_recovery_quote, RecoveryQuote, RecoveryQuoteRequest, DOCS as RECOVERY_QUOTE_DOCS};

pub(super) const LIFI_QUOTE_ENDPOINT: &str = "https://li.quest/v1/quote";
pub(super) const LIFI_STATUS_ENDPOINT: &str = "https://li.quest/v1/status";
pub(super) const LIFI_QUOTE_DOCS: &str =
    "https://docs.li.fi/agents/reference/endpoint-specs#get-quote";
pub(super) const LIFI_STATUS_DOCS: &str =
    "https://docs.li.fi/api-reference/check-the-status-of-a-cross-chain-transfer";
const LIFI_SOLANA_CHAIN_ID: u64 = 1_151_111_081_099_710;
const EVM_ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
// LI.FI documents an approximately 60 second quote lifetime. Keep a five second
// local safety margin so an authorization ticket cannot outlive provider data.
const LIFI_QUOTE_VALIDITY_MS: i64 = 55_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiQuoteResponse {
    id: String,
    #[serde(default)]
    transaction_id: String,
    tool: String,
    action: LifiAction,
    estimate: LifiEstimate,
    transaction_request: LifiTransactionRequest,
    #[serde(default)]
    included_steps: Vec<LifiIncludedStep>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiAction {
    from_token: LifiToken,
    to_token: LifiToken,
    from_amount: String,
    from_chain_id: u64,
    to_chain_id: u64,
    from_address: String,
    to_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiToken {
    address: String,
    chain_id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiEstimate {
    from_amount: String,
    to_amount: String,
    to_amount_min: String,
    #[serde(default)]
    fee_costs: Option<Vec<LifiUsdCost>>,
    #[serde(default)]
    gas_costs: Option<Vec<LifiUsdCost>>,
    execution_duration: Option<u64>,
    approval_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiTransactionRequest {
    data: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    gas_limit: Option<String>,
    #[serde(default)]
    gas_price: Option<String>,
    #[serde(default)]
    chain_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiUsdCost {
    #[serde(rename = "amountUSD")]
    amount_usd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LifiIncludedStep {
    tool: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiStatusResponse {
    status: String,
    #[serde(default)]
    substatus: Option<String>,
    #[serde(default)]
    substatus_message: Option<String>,
    #[serde(default)]
    transaction_id: Option<String>,
    #[serde(default)]
    sending: Option<LifiStatusTransaction>,
    #[serde(default)]
    receiving: Option<LifiStatusTransaction>,
    #[serde(default)]
    to_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiStatusTransaction {
    #[serde(default)]
    tx_hash: Option<String>,
    #[serde(default)]
    amount: Option<String>,
    #[serde(default)]
    chain_id: Option<u64>,
    #[serde(default)]
    token: Option<LifiToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LifiTransferState {
    NotFound,
    Pending,
    Completed,
    CompletedEvidenceMissing,
    Partial,
    Refunded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LifiTransferEvidence {
    pub(super) state: LifiTransferState,
    pub(super) provider_status: String,
    pub(super) substatus: Option<String>,
    pub(super) message: Option<String>,
    pub(super) transaction_id: Option<String>,
    pub(super) sending_tx_hash: Option<String>,
    pub(super) receiving_tx_hash: Option<String>,
    pub(super) receiving_amount_raw: Option<String>,
    pub(super) sending_chain_id: Option<u64>,
    pub(super) receiving_chain_id: Option<u64>,
    pub(super) receiving_token: Option<String>,
    pub(super) receiving_token_chain_id: Option<u64>,
    pub(super) to_address: Option<String>,
    pub(super) observed_at_ms: i64,
    pub(super) official_docs_url: String,
}

pub(crate) fn chain_id(chain: &str) -> Option<u64> {
    let preset = shared_types::onchain_chain_preset(chain)?;
    preset.chain_id.or_else(|| {
        chain
            .eq_ignore_ascii_case("solana")
            .then_some(LIFI_SOLANA_CHAIN_ID)
    })
}

pub(super) async fn fetch_bridge(
    from_chain: &str,
    to_chain: &str,
    from_token: &str,
    to_token: &str,
    from_amount_raw: &str,
    from_address: &str,
    to_address: &str,
    slippage_bps: f64,
) -> Result<OnchainBridgeQuote, String> {
    let from_chain_id = chain_id(from_chain)
        .ok_or_else(|| format!("LI.FI does not support configured source chain {from_chain}"))?;
    let to_chain_id = chain_id(to_chain)
        .ok_or_else(|| format!("LI.FI does not support configured target chain {to_chain}"))?;
    if from_chain_id == to_chain_id {
        return Err("LI.FI bridge quote requires two distinct chains".to_owned());
    }
    if from_address.trim().is_empty() || to_address.trim().is_empty() {
        return Err("cross-chain quote requires source and target wallet addresses".to_owned());
    }
    if !from_amount_raw
        .parse::<u128>()
        .is_ok_and(|amount| amount > 0)
    {
        return Err("cross-chain quote input amount is invalid".to_owned());
    }

    let configured_from_token = from_token.trim().to_owned();
    let configured_to_token = to_token.trim().to_owned();
    let from_token = lifi_token_address(&configured_from_token);
    let to_token = lifi_token_address(&configured_to_token);
    let slippage = (slippage_bps.max(0.0) / 10_000.0).to_string();
    let mut request = quote_client().get(LIFI_QUOTE_ENDPOINT).query(&[
        ("fromChain", from_chain_id.to_string()),
        ("toChain", to_chain_id.to_string()),
        ("fromToken", from_token.clone()),
        ("toToken", to_token.clone()),
        ("fromAmount", from_amount_raw.to_owned()),
        ("fromAddress", from_address.trim().to_owned()),
        ("toAddress", to_address.trim().to_owned()),
        ("slippage", slippage),
        ("order", "FASTEST".to_owned()),
    ]);
    if let Some(api_key) = env_key("LIFI_API_KEY") {
        request = request.header("x-lifi-api-key", api_key);
    }
    let body = decode_response(
        request
            .send()
            .await
            .map_err(|error| format!("LI.FI bridge quote transport failed: {error}"))?,
        "LI.FI",
    )
    .await?;
    let response: LifiQuoteResponse = serde_json::from_str(&body)
        .map_err(|error| format!("LI.FI quote decode failed: {error}"))?;
    let mut quote = parse_quote(
        response,
        from_chain_id,
        to_chain_id,
        &from_token,
        &to_token,
        from_amount_raw,
        from_address,
        to_address,
        common::time::now_ms(),
    )?;
    quote.from_token = configured_from_token;
    quote.to_token = configured_to_token;
    Ok(quote)
}

pub(super) async fn fetch_status(
    tx_hash: &str,
    from_chain_id: u64,
    to_chain_id: u64,
    bridge: &str,
) -> Result<LifiTransferEvidence, String> {
    if tx_hash.trim().is_empty() || bridge.trim().is_empty() {
        return Err("LI.FI status requires a transaction hash and bridge tool".to_owned());
    }
    let mut request = quote_client().get(LIFI_STATUS_ENDPOINT).query(&[
        ("txHash", tx_hash.trim().to_owned()),
        ("fromChain", from_chain_id.to_string()),
        ("toChain", to_chain_id.to_string()),
        ("bridge", bridge.trim().to_owned()),
    ]);
    if let Some(api_key) = env_key("LIFI_API_KEY") {
        request = request.header("x-lifi-api-key", api_key);
    }
    let body = decode_response(
        request
            .send()
            .await
            .map_err(|error| format!("LI.FI status transport failed: {error}"))?,
        "LI.FI",
    )
    .await?;
    let response: LifiStatusResponse = serde_json::from_str(&body)
        .map_err(|error| format!("LI.FI status decode failed: {error}"))?;
    parse_status(response, common::time::now_ms())
}

fn parse_quote(
    response: LifiQuoteResponse,
    from_chain_id: u64,
    to_chain_id: u64,
    from_token: &str,
    to_token: &str,
    from_amount_raw: &str,
    from_address: &str,
    to_address: &str,
    observed_at_ms: i64,
) -> Result<OnchainBridgeQuote, String> {
    if response.action.from_chain_id != from_chain_id
        || response.action.from_token.chain_id != from_chain_id
        || response.action.to_chain_id != to_chain_id
        || response.action.to_token.chain_id != to_chain_id
    {
        return Err("LI.FI quote returned a different chain path".to_owned());
    }
    if !token_matches(
        from_chain_id,
        from_token,
        &response.action.from_token.address,
    ) || !token_matches(to_chain_id, to_token, &response.action.to_token.address)
    {
        return Err("LI.FI quote returned a different token path".to_owned());
    }
    if response.action.from_amount != from_amount_raw
        || response.estimate.from_amount != from_amount_raw
    {
        return Err("LI.FI quote changed the exact input amount".to_owned());
    }
    if !address_matches(from_chain_id, from_address, &response.action.from_address)
        || !address_matches(to_chain_id, to_address, &response.action.to_address)
    {
        return Err("LI.FI quote returned a different wallet path".to_owned());
    }
    let expected = positive_raw(&response.estimate.to_amount, "toAmount")?;
    let minimum = positive_raw(&response.estimate.to_amount_min, "toAmountMin")?;
    if minimum > expected {
        return Err("LI.FI quote minimum output exceeds expected output".to_owned());
    }
    if response.id.trim().is_empty()
        || response.transaction_id.trim().is_empty()
        || response.tool.trim().is_empty()
    {
        return Err("LI.FI quote omitted route identity evidence".to_owned());
    }
    let approval_address = response
        .estimate
        .approval_address
        .as_deref()
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(str::to_owned);
    if from_chain_id != LIFI_SOLANA_CHAIN_ID && !native_evm_token(from_token) {
        let approval = approval_address
            .as_deref()
            .ok_or_else(|| "LI.FI quote omitted ERC20 approvalAddress".to_owned())?;
        validate_evm_address(approval, "approvalAddress")?;
    }
    let transaction = parse_transaction(
        &response.transaction_request,
        &response.transaction_id,
        &response.tool,
        from_chain_id,
        &response.action.from_address,
        from_token,
        approval_address.as_deref(),
    )?;
    let mut route_tools = response
        .included_steps
        .into_iter()
        .map(|step| step.tool)
        .filter(|tool| !tool.trim().is_empty())
        .collect::<Vec<_>>();
    route_tools.push(response.tool.clone());
    route_tools.sort();
    route_tools.dedup();
    Ok(OnchainBridgeQuote {
        provider: "lifi".to_owned(),
        route_id: response.id,
        transaction_id: response.transaction_id,
        tool: response.tool,
        route_tools,
        from_chain_id,
        to_chain_id,
        from_token: from_token.to_owned(),
        to_token: to_token.to_owned(),
        from_address: response.action.from_address,
        to_address: response.action.to_address,
        from_amount_raw: response.estimate.from_amount,
        to_amount_raw: response.estimate.to_amount,
        to_amount_min_raw: response.estimate.to_amount_min,
        fee_usd: optional_usd(response.estimate.fee_costs.as_deref())?,
        gas_usd: optional_usd(response.estimate.gas_costs.as_deref().filter(|costs| !costs.is_empty()))?,
        execution_duration_seconds: response.estimate.execution_duration,
        approval_address,
        transaction,
        official_docs_url: LIFI_QUOTE_DOCS.to_owned(),
        observed_at_ms,
        valid_until_ms: observed_at_ms.saturating_add(LIFI_QUOTE_VALIDITY_MS),
    })
}

fn parse_transaction(
    request: &LifiTransactionRequest,
    transaction_id: &str,
    tool: &str,
    from_chain_id: u64,
    from_address: &str,
    from_token: &str,
    approval_address: Option<&str>,
) -> Result<OnchainUnsignedTransaction, String> {
    if from_chain_id == LIFI_SOLANA_CHAIN_ID {
        let decoded = BASE64_STANDARD
            .decode(request.data.trim())
            .map_err(|_| "LI.FI returned invalid Solana transaction base64".to_owned())?;
        if decoded.is_empty() {
            return Err("LI.FI returned an empty Solana transaction".to_owned());
        }
        return Ok(OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64: request.data.trim().to_owned(),
            request_id: transaction_id.to_owned(),
            router: tool.to_owned(),
            mode: "lifi_cross_chain".to_owned(),
            last_valid_block_height: None,
            expire_at_ms: None,
        });
    }

    validate_evm_address(from_address, "action.fromAddress")?;
    if request
        .chain_id
        .filter(|chain_id| *chain_id == from_chain_id)
        .is_none()
    {
        return Err("LI.FI transactionRequest returned a different chainId".to_owned());
    }
    if request
        .from
        .as_deref()
        .is_some_and(|observed| !observed.eq_ignore_ascii_case(from_address))
    {
        return Err("LI.FI transactionRequest returned a different sender".to_owned());
    }
    let to = required_request_field(request.to.as_deref(), "to")?;
    let value = required_request_field(request.value.as_deref(), "value")?;
    let gas_limit = required_request_field(request.gas_limit.as_deref(), "gasLimit")?;
    validate_evm_address(to, "transactionRequest.to")?;
    validate_hex(&request.data, "transactionRequest.data", false)?;
    validate_hex(value, "transactionRequest.value", true)?;
    validate_hex(gas_limit, "transactionRequest.gasLimit", true)?;
    if let Some(gas_price) = request.gas_price.as_deref() {
        validate_hex(gas_price, "transactionRequest.gasPrice", true)?;
    }
    Ok(OnchainUnsignedTransaction::EvmCall {
        chain_id: from_chain_id,
        from: from_address.to_owned(),
        to: to.to_owned(),
        data: request.data.clone(),
        value: value.to_owned(),
        gas: gas_limit.to_owned(),
        gas_price: request.gas_price.clone(),
        max_priority_fee_per_gas: None,
        allowance_spender: (!native_evm_token(from_token))
            .then(|| approval_address.map(str::to_owned))
            .flatten(),
    })
}

fn optional_usd(costs: Option<&[LifiUsdCost]>) -> Result<Option<f64>, String> {
    costs.map(sum_usd).transpose().map(Option::flatten)
}

fn parse_status(
    response: LifiStatusResponse,
    observed_at_ms: i64,
) -> Result<LifiTransferEvidence, String> {
    let provider_status = response.status.trim().to_ascii_uppercase();
    let substatus = response
        .substatus
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_uppercase);
    let receiving_tx_hash = nonempty_status_field(
        response
            .receiving
            .as_ref()
            .and_then(|transaction| transaction.tx_hash.as_deref()),
    );
    let receiving_amount_raw = nonempty_status_field(
        response
            .receiving
            .as_ref()
            .and_then(|transaction| transaction.amount.as_deref()),
    );
    let destination_proven = receiving_tx_hash.is_some()
        && receiving_amount_raw
            .as_deref()
            .and_then(|amount| amount.parse::<u128>().ok())
            .is_some_and(|amount| amount > 0);
    let state = match (provider_status.as_str(), substatus.as_deref()) {
        ("NOT_FOUND", _) => LifiTransferState::NotFound,
        ("PENDING", _) => LifiTransferState::Pending,
        ("DONE", Some("COMPLETED")) if destination_proven => LifiTransferState::Completed,
        ("DONE", Some("COMPLETED")) => LifiTransferState::CompletedEvidenceMissing,
        ("DONE", Some("PARTIAL")) => LifiTransferState::Partial,
        ("DONE", Some("REFUNDED")) => LifiTransferState::Refunded,
        ("FAILED", Some("REFUNDED")) => LifiTransferState::Refunded,
        ("FAILED", _) => LifiTransferState::Failed,
        ("DONE", Some(other)) => {
            return Err(format!(
                "LI.FI status returned unsupported DONE substatus {other}"
            ))
        }
        ("DONE", None) => return Err("LI.FI DONE status omitted substatus".to_owned()),
        (other, _) => return Err(format!("LI.FI status returned unsupported state {other}")),
    };
    Ok(LifiTransferEvidence {
        state,
        provider_status,
        substatus,
        message: nonempty_status_field(response.substatus_message.as_deref()),
        transaction_id: nonempty_status_field(response.transaction_id.as_deref()),
        sending_tx_hash: nonempty_status_field(
            response
                .sending
                .as_ref()
                .and_then(|transaction| transaction.tx_hash.as_deref()),
        ),
        receiving_tx_hash,
        receiving_amount_raw,
        sending_chain_id: response.sending.as_ref().and_then(|tx| tx.chain_id),
        receiving_chain_id: response.receiving.as_ref().and_then(|tx| tx.chain_id),
        receiving_token: response
            .receiving
            .as_ref()
            .and_then(|tx| tx.token.as_ref())
            .map(|token| token.address.clone()),
        receiving_token_chain_id: response
            .receiving
            .as_ref()
            .and_then(|tx| tx.token.as_ref())
            .map(|token| token.chain_id),
        to_address: nonempty_status_field(response.to_address.as_deref()),
        observed_at_ms,
        official_docs_url: LIFI_STATUS_DOCS.to_owned(),
    })
}

fn nonempty_status_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_request_field<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("LI.FI transactionRequest omitted {field}"))
}

pub(crate) fn address_matches(chain_id: u64, expected: &str, observed: &str) -> bool {
    if chain_id == LIFI_SOLANA_CHAIN_ID {
        expected.trim() == observed.trim()
    } else {
        expected.trim().eq_ignore_ascii_case(observed.trim())
    }
}

fn validate_evm_address(address: &str, field: &str) -> Result<(), String> {
    let value = address.trim();
    if value.len() != 42
        || !value.starts_with("0x")
        || !value[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("LI.FI returned invalid {field}"));
    }
    Ok(())
}

fn validate_hex(value: &str, field: &str, allow_single_nibble: bool) -> Result<(), String> {
    let value = value.trim();
    let hex = value.strip_prefix("0x").unwrap_or_default();
    let valid_length = allow_single_nibble || hex.len() % 2 == 0;
    if hex.is_empty()
        || !valid_length
        || !hex.chars().all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("LI.FI returned invalid {field}"));
    }
    Ok(())
}

fn lifi_token_address(address: &str) -> String {
    if address.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS) {
        EVM_ZERO_ADDRESS.to_owned()
    } else {
        address.trim().to_owned()
    }
}

pub(crate) fn token_matches(chain_id: u64, expected: &str, observed: &str) -> bool {
    if chain_id == LIFI_SOLANA_CHAIN_ID {
        expected == observed
    } else {
        expected.eq_ignore_ascii_case(observed)
            || (native_evm_token(expected) && native_evm_token(observed))
    }
}

fn native_evm_token(address: &str) -> bool {
    address.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS)
        || address.eq_ignore_ascii_case(EVM_ZERO_ADDRESS)
}

fn positive_raw(value: &str, field: &str) -> Result<u128, String> {
    value
        .parse::<u128>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or_else(|| format!("LI.FI quote returned invalid {field}"))
}

fn sum_usd(costs: &[LifiUsdCost]) -> Result<Option<f64>, String> {
    let mut total = 0.0;
    for cost in costs {
        let Some(amount) = cost.amount_usd.as_deref() else {
            return Ok(None);
        };
        let amount = amount
            .parse::<f64>()
            .ok()
            .filter(|amount| amount.is_finite() && *amount >= 0.0)
            .ok_or_else(|| "LI.FI quote returned invalid USD cost evidence".to_owned())?;
        total += amount;
        if !total.is_finite() { return Err("LI.FI quote USD cost total overflow".into()); }
    }
    Ok(Some(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimum_output_fees_gas_duration_and_route_tools() {
        let response: LifiQuoteResponse = serde_json::from_str(
            r#"{
                "id":"route-1","transactionId":"tx-1","tool":"across",
                "action":{
                    "fromToken":{"address":"0x0000000000000000000000000000000000000000","chainId":8453},
                    "toToken":{"address":"0x0000000000000000000000000000000000000000","chainId":42161},
                    "fromAmount":"1000000000000000000","fromChainId":8453,"toChainId":42161,
                    "fromAddress":"0x1111111111111111111111111111111111111111",
                    "toAddress":"0x2222222222222222222222222222222222222222"
                },
                "estimate":{
                    "fromAmount":"1000000000000000000","toAmount":"999000000000000000","toAmountMin":"995000000000000000",
                    "feeCosts":[{"amountUSD":"0.25"}],"gasCosts":[{"amountUSD":"0.10"}],"executionDuration":12
                },
                "includedSteps":[{"tool":"feeCollection"},{"tool":"across"}],
                "transactionRequest":{
                    "to":"0x3333333333333333333333333333333333333333","data":"0x1234","value":"0x0",
                    "from":"0x1111111111111111111111111111111111111111","chainId":8453,
                    "gasPrice":"0x10","gasLimit":"0x5208"
                }
            }"#,
        )
        .expect("fixture");
        let quote = parse_quote(
            response,
            8_453,
            42_161,
            EVM_NATIVE_TOKEN_ADDRESS,
            EVM_NATIVE_TOKEN_ADDRESS,
            "1000000000000000000",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            100,
        )
        .expect("valid quote");

        assert_eq!(quote.to_amount_min_raw, "995000000000000000");
        assert_eq!(quote.fee_usd, Some(0.25));
        assert_eq!(quote.gas_usd, Some(0.10));
        assert_eq!(quote.execution_duration_seconds, Some(12));
        assert_eq!(quote.route_tools, vec!["across", "feeCollection"]);
        assert_eq!(quote.transaction_id, "tx-1");
        assert_eq!(quote.valid_until_ms, 55_100);
        assert!(matches!(
            quote.transaction,
            OnchainUnsignedTransaction::EvmCall {
                chain_id: 8_453,
                ..
            }
        ));
    }

    #[test]
    fn rejects_changed_exact_input_or_token_path() {
        let response: LifiQuoteResponse = serde_json::from_str(
            r#"{
                "id":"route-1","transactionId":"tx-1","tool":"across",
                "action":{
                    "fromToken":{"address":"0x1111111111111111111111111111111111111111","chainId":1},
                    "toToken":{"address":"0x2222222222222222222222222222222222222222","chainId":10},
                    "fromAmount":"9","fromChainId":1,"toChainId":10,
                    "fromAddress":"0x3333333333333333333333333333333333333333",
                    "toAddress":"0x4444444444444444444444444444444444444444"
                },
                "estimate":{"fromAmount":"9","toAmount":"8","toAmountMin":"7","feeCosts":[],"gasCosts":[],
                    "approvalAddress":"0x5555555555555555555555555555555555555555"},
                "includedSteps":[],
                "transactionRequest":{"to":"0x6666666666666666666666666666666666666666","data":"0x1234",
                    "value":"0x0","chainId":1,"gasLimit":"0x5208"}
            }"#,
        )
        .expect("fixture");

        assert!(parse_quote(
            response,
            1,
            10,
            "0xdead",
            "0xbeef",
            "10",
            "0x3333333333333333333333333333333333333333",
            "0x4444444444444444444444444444444444444444",
            1,
        )
        .is_err());
    }

    #[test]
    fn parses_official_solana_base64_transaction_shape() {
        let response: LifiQuoteResponse = serde_json::from_str(
            r#"{
                "id":"route-sol","transactionId":"tx-sol","tool":"mayan",
                "action":{
                    "fromToken":{"address":"So11111111111111111111111111111111111111112","chainId":1151111081099710},
                    "toToken":{"address":"0x2222222222222222222222222222222222222222","chainId":8453},
                    "fromAmount":"1000","fromChainId":1151111081099710,"toChainId":8453,
                    "fromAddress":"SolanaWallet1111111111111111111111111111111",
                    "toAddress":"0x4444444444444444444444444444444444444444"
                },
                "estimate":{"fromAmount":"1000","toAmount":"900","toAmountMin":"850","feeCosts":[],"gasCosts":[]},
                "includedSteps":[{"tool":"mayan"}],
                "transactionRequest":{"data":"AQID"}
            }"#,
        )
        .expect("fixture");

        let quote = parse_quote(
            response,
            LIFI_SOLANA_CHAIN_ID,
            8_453,
            "So11111111111111111111111111111111111111112",
            "0x2222222222222222222222222222222222222222",
            "1000",
            "SolanaWallet1111111111111111111111111111111",
            "0x4444444444444444444444444444444444444444",
            500,
        )
        .expect("valid SVM quote");

        assert!(matches!(
            quote.transaction,
            OnchainUnsignedTransaction::SolanaVersioned {
                ref transaction_base64,
                ..
            } if transaction_base64 == "AQID"
        ));
    }

    #[test]
    fn http_200_not_found_never_becomes_a_success_or_retry_signal() {
        let response: LifiStatusResponse = serde_json::from_str(
            r#"{"status":"NOT_FOUND","substatusMessage":"Transaction not found"}"#,
        )
        .expect("fixture");
        let evidence = parse_status(response, 100).expect("known status");

        assert_eq!(evidence.state, LifiTransferState::NotFound);
        assert!(evidence.receiving_tx_hash.is_none());
        assert!(evidence.receiving_amount_raw.is_none());
    }

    #[test]
    fn token_identity_preserves_solana_case_and_evm_native_aliases() {
        let mint = "So11111111111111111111111111111111111111112";
        assert!(token_matches(LIFI_SOLANA_CHAIN_ID, mint, mint));
        assert!(!token_matches(
            LIFI_SOLANA_CHAIN_ID,
            mint,
            &mint.to_ascii_lowercase()
        ));
        assert!(token_matches(
            8453,
            EVM_NATIVE_TOKEN_ADDRESS,
            EVM_ZERO_ADDRESS
        ));
        assert!(!token_matches(
            LIFI_SOLANA_CHAIN_ID,
            EVM_NATIVE_TOKEN_ADDRESS,
            EVM_ZERO_ADDRESS
        ));
    }

    #[test]
    fn completed_requires_destination_hash_and_positive_amount() {
        let complete: LifiStatusResponse = serde_json::from_str(
            r#"{
                "status":"DONE","substatus":"COMPLETED","transactionId":"lifi-1",
                "toAddress":"0xwallet",
                "sending":{"txHash":"0xsource","amount":"100","chainId":8453},
                "receiving":{"txHash":"0xdestination","amount":"98","chainId":42161,
                    "token":{"address":"0xtoken","chainId":42161}}
            }"#,
        )
        .expect("fixture");
        let missing: LifiStatusResponse = serde_json::from_str(
            r#"{
                "status":"DONE","substatus":"COMPLETED","transactionId":"lifi-1",
                "sending":{"txHash":"0xsource","amount":"100"},
                "receiving":{"amount":"98"}
            }"#,
        )
        .expect("fixture");

        let complete = parse_status(complete, 100).expect("complete evidence");
        let missing = parse_status(missing, 100).expect("incomplete evidence");
        assert_eq!(complete.state, LifiTransferState::Completed);
        assert_eq!(complete.receiving_amount_raw.as_deref(), Some("98"));
        assert_eq!(complete.sending_chain_id, Some(8453));
        assert_eq!(complete.receiving_chain_id, Some(42161));
        assert_eq!(complete.receiving_token_chain_id, Some(42161));
        assert_eq!(complete.receiving_token.as_deref(), Some("0xtoken"));
        assert_eq!(complete.to_address.as_deref(), Some("0xwallet"));
        assert_eq!(missing.state, LifiTransferState::CompletedEvidenceMissing);
    }

    #[test]
    fn partial_refunded_and_failed_are_distinct_terminal_outcomes() {
        for (status, substatus, expected) in [
            ("DONE", "PARTIAL", LifiTransferState::Partial),
            ("DONE", "REFUNDED", LifiTransferState::Refunded),
            ("FAILED", "REFUNDED", LifiTransferState::Refunded),
            ("FAILED", "SLIPPAGE_EXCEEDED", LifiTransferState::Failed),
        ] {
            let response: LifiStatusResponse = serde_json::from_value(serde_json::json!({
                "status": status,
                "substatus": substatus,
                "transactionId": "lifi-1",
                "sending": { "txHash": "0xsource", "amount": "100" }
            }))
            .expect("fixture");
            assert_eq!(
                parse_status(response, 100).expect("known terminal").state,
                expected
            );
        }
    }
}
