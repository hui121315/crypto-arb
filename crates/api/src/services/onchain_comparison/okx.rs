use super::provider_runtime::env_key;
use super::provider_types::{
    OkxApprovalEnvelope, OkxApprovalTransaction, OkxCredentials, OkxDexQuote, OkxDexSwap,
    OkxQuoteEnvelope, OkxSwapEnvelope,
};
use super::quote::{decode_response, quote_client, transport_problem};
use chrono::{SecondsFormat, Utc};
use onchain_monitor::ProviderQuote;
use shared_types::{onchain_chain_preset, OnchainComparisonConfig};

pub(super) const OKX_QUOTE_ENDPOINT: &str = "https://web3.okx.com/api/v6/dex/aggregator/quote";
pub(super) const OKX_SWAP_ENDPOINT: &str = "https://web3.okx.com/api/v6/dex/aggregator/swap";
pub(super) const OKX_APPROVAL_ENDPOINT: &str =
    "https://web3.okx.com/api/v6/dex/aggregator/approve-transaction";
pub(super) const OKX_QUOTE_DOCS: &str =
    "https://web3.okx.com/zh-hans/onchainos/dev-docs/trade/dex-get-quote";
pub(super) const OKX_SWAP_DOCS: &str = "https://web3.okx.com/onchainos/dev-docs/trade/dex-swap";
pub(super) const OKX_APPROVAL_DOCS: &str =
    "https://web3.okx.com/onchainos/dev-docs/trade/dex-approve-transaction";
pub(super) const OKX_AUTH_DOCS: &str =
    "https://web3.okx.com/id/onchainos/dev-docs/home/api-access-and-usage";
pub(super) const OKX_INTERVAL_MS: i64 = 3_000;

const REQUEST_GAP_MS: u64 = 1_000;

#[derive(Clone, Copy)]
pub(super) struct OkxQuoteRequest<'a> {
    pub chain_id: u64,
    pub from_token: &'a str,
    pub to_token: &'a str,
    pub amount: &'a str,
}

#[derive(Clone, Copy)]
pub(super) struct OkxSwapRequest<'a> {
    pub chain_id: u64,
    pub from_token: &'a str,
    pub to_token: &'a str,
    pub amount: &'a str,
    pub slippage_percent: &'a str,
    pub wallet_address: &'a str,
}

#[derive(Clone, Copy)]
pub(super) struct OkxApprovalRequest<'a> {
    pub chain_id: u64,
    pub token_contract_address: &'a str,
    pub approve_amount: &'a str,
}

pub(super) fn credentials() -> Result<OkxCredentials, String> {
    let fields = [
        ("OKX_DEX_API_KEY", env_key("OKX_DEX_API_KEY")),
        ("OKX_DEX_SECRET_KEY", env_key("OKX_DEX_SECRET_KEY")),
        ("OKX_DEX_PASSPHRASE", env_key("OKX_DEX_PASSPHRASE")),
    ];
    let missing = fields
        .iter()
        .filter_map(|(name, value)| value.is_none().then_some(*name))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "需要配置 {} 才能读取 OKX DEX 报价；认证说明 {OKX_AUTH_DOCS}",
            missing.join(", "),
        ));
    }
    Ok(OkxCredentials {
        api_key: fields[0].1.clone().unwrap_or_default(),
        secret_key: fields[1].1.clone().unwrap_or_default(),
        passphrase: fields[2].1.clone().unwrap_or_default(),
    })
}

pub(super) async fn fetch_pair(
    config: &OnchainComparisonConfig,
) -> Result<(ProviderQuote, ProviderQuote, &'static str, &'static str), String> {
    let chain_id = onchain_chain_preset(&config.chain)
        .and_then(|preset| preset.chain_id)
        .ok_or_else(|| format!("chain {} has no EVM chain id", config.chain))?;
    let credentials = credentials()?;
    let reverse = provider_quote(
        fetch_quote(
            &credentials,
            chain_id,
            &config.quote_mint,
            &config.base_mint,
            &config.quote_amount_raw,
        )
        .await?,
        chain_id,
    )?;
    let base_amount_raw = super::quote::anchored_base_amount(&reverse)?;
    tokio::time::sleep(std::time::Duration::from_millis(REQUEST_GAP_MS)).await;
    let forward = provider_quote(
        fetch_quote(
            &credentials,
            chain_id,
            &config.base_mint,
            &config.quote_mint,
            &base_amount_raw,
        )
        .await?,
        chain_id,
    )?;
    Ok((forward, reverse, OKX_QUOTE_ENDPOINT, OKX_QUOTE_DOCS))
}

pub(super) async fn fetch_exact_in(
    config: &OnchainComparisonConfig,
    input_token: &str,
    output_token: &str,
    input_amount_raw: &str,
) -> Result<ProviderQuote, String> {
    let chain_id = onchain_chain_preset(&config.chain)
        .and_then(|preset| preset.chain_id)
        .ok_or_else(|| format!("chain {} has no EVM chain id", config.chain))?;
    let credentials = credentials()?;
    provider_quote(
        fetch_quote(
            &credentials,
            chain_id,
            input_token,
            output_token,
            input_amount_raw,
        )
        .await?,
        chain_id,
    )
}

async fn fetch_quote(
    credentials: &OkxCredentials,
    chain_id: u64,
    from_token: &str,
    to_token: &str,
    amount: &str,
) -> Result<OkxDexQuote, String> {
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let request = build_quote_request(
        quote_client(),
        credentials,
        &timestamp,
        OkxQuoteRequest {
            chain_id,
            from_token,
            to_token,
            amount,
        },
    )?;
    let response = quote_client()
        .execute(request)
        .await
        .map_err(|error| transport_problem("OKX DEX", "报价", "web3.okx.com", &error))?;
    let body = decode_response(response, "OKX DEX").await?;
    decode_envelope(&body)
}

pub(super) async fn fetch_swap(request: OkxSwapRequest<'_>) -> Result<OkxDexSwap, String> {
    let credentials = credentials()?;
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let request = build_swap_request(quote_client(), &credentials, &timestamp, request)?;
    let response = quote_client()
        .execute(request)
        .await
        .map_err(|error| transport_problem("OKX DEX", "交易计划", "web3.okx.com", &error))?;
    let body = decode_response(response, "OKX DEX").await?;
    decode_swap_envelope(&body)
}

pub(super) async fn fetch_approval(
    request: OkxApprovalRequest<'_>,
) -> Result<OkxApprovalTransaction, String> {
    let credentials = credentials()?;
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let request = build_approval_request(quote_client(), &credentials, &timestamp, request)?;
    let response = quote_client()
        .execute(request)
        .await
        .map_err(|error| transport_problem("OKX DEX", "授权计划", "web3.okx.com", &error))?;
    let body = decode_response(response, "OKX DEX").await?;
    decode_approval_envelope(&body)
}

pub(super) fn build_quote_request(
    client: &reqwest::Client,
    credentials: &OkxCredentials,
    timestamp: &str,
    quote: OkxQuoteRequest<'_>,
) -> Result<reqwest::Request, String> {
    let mut url = reqwest::Url::parse(OKX_QUOTE_ENDPOINT)
        .map_err(|_| "OKX DEX quote endpoint is invalid".to_owned())?;
    url.query_pairs_mut()
        .append_pair("chainIndex", &quote.chain_id.to_string())
        .append_pair("amount", quote.amount)
        .append_pair("swapMode", "exactIn")
        .append_pair("fromTokenAddress", quote.from_token)
        .append_pair("toTokenAddress", quote.to_token);
    let query = url
        .query()
        .ok_or_else(|| "OKX DEX quote query is empty".to_owned())?;
    let request_path = format!("{}?{query}", url.path());
    let prehash = format!("{timestamp}GET{request_path}");
    let signature =
        common::signing::hmac_sha256_base64(credentials.secret_key.as_bytes(), prehash.as_bytes());
    client
        .get(url)
        .header("OK-ACCESS-KEY", &credentials.api_key)
        .header("OK-ACCESS-SIGN", signature)
        .header("OK-ACCESS-TIMESTAMP", timestamp)
        .header("OK-ACCESS-PASSPHRASE", &credentials.passphrase)
        .build()
        .map_err(|_| "OKX DEX quote request could not be built".to_owned())
}

pub(super) fn build_swap_request(
    client: &reqwest::Client,
    credentials: &OkxCredentials,
    timestamp: &str,
    swap: OkxSwapRequest<'_>,
) -> Result<reqwest::Request, String> {
    let mut url = reqwest::Url::parse(OKX_SWAP_ENDPOINT)
        .map_err(|_| "OKX DEX swap endpoint is invalid".to_owned())?;
    url.query_pairs_mut()
        .append_pair("chainIndex", &swap.chain_id.to_string())
        .append_pair("amount", swap.amount)
        .append_pair("swapMode", "exactIn")
        .append_pair("fromTokenAddress", swap.from_token)
        .append_pair("toTokenAddress", swap.to_token)
        .append_pair("slippagePercent", swap.slippage_percent)
        .append_pair("userWalletAddress", swap.wallet_address);
    signed_get_request(client, credentials, timestamp, url)
}

pub(super) fn build_approval_request(
    client: &reqwest::Client,
    credentials: &OkxCredentials,
    timestamp: &str,
    approval: OkxApprovalRequest<'_>,
) -> Result<reqwest::Request, String> {
    let mut url = reqwest::Url::parse(OKX_APPROVAL_ENDPOINT)
        .map_err(|_| "OKX DEX approval endpoint is invalid".to_owned())?;
    url.query_pairs_mut()
        .append_pair("chainIndex", &approval.chain_id.to_string())
        .append_pair("tokenContractAddress", approval.token_contract_address)
        .append_pair("approveAmount", approval.approve_amount);
    signed_get_request(client, credentials, timestamp, url)
}

fn signed_get_request(
    client: &reqwest::Client,
    credentials: &OkxCredentials,
    timestamp: &str,
    url: reqwest::Url,
) -> Result<reqwest::Request, String> {
    let query = url
        .query()
        .ok_or_else(|| "OKX DEX request query is empty".to_owned())?;
    let request_path = format!("{}?{query}", url.path());
    let prehash = format!("{timestamp}GET{request_path}");
    let signature =
        common::signing::hmac_sha256_base64(credentials.secret_key.as_bytes(), prehash.as_bytes());
    client
        .get(url)
        .header("OK-ACCESS-KEY", &credentials.api_key)
        .header("OK-ACCESS-SIGN", signature)
        .header("OK-ACCESS-TIMESTAMP", timestamp)
        .header("OK-ACCESS-PASSPHRASE", &credentials.passphrase)
        .build()
        .map_err(|_| "OKX DEX request could not be built".to_owned())
}

pub(super) fn decode_envelope(body: &str) -> Result<OkxDexQuote, String> {
    let envelope: OkxQuoteEnvelope = serde_json::from_str(body)
        .map_err(|error| format!("OKX DEX quote decode failed: {error}"))?;
    if envelope.code != "0" {
        return Err(format!(
            "OKX DEX quote rejected with code {}: {}",
            envelope.code, envelope.msg
        ));
    }
    envelope
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "OKX DEX quote response has no data row".to_owned())
}

pub(super) fn decode_swap_envelope(body: &str) -> Result<OkxDexSwap, String> {
    let envelope: OkxSwapEnvelope = serde_json::from_str(body)
        .map_err(|error| format!("OKX DEX swap decode failed: {error}"))?;
    if envelope.code != "0" {
        return Err(format!(
            "OKX DEX swap rejected with code {}: {}",
            envelope.code, envelope.msg
        ));
    }
    let swap = envelope
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "OKX DEX swap response has no data row".to_owned())?;
    if !swap.signature_data.is_empty() {
        return Err("OKX DEX swap requires unsupported additional signature data".to_owned());
    }
    Ok(swap)
}

pub(super) fn decode_approval_envelope(body: &str) -> Result<OkxApprovalTransaction, String> {
    let envelope: OkxApprovalEnvelope = serde_json::from_str(body)
        .map_err(|error| format!("OKX DEX approval decode failed: {error}"))?;
    if envelope.code != "0" {
        return Err(format!(
            "OKX DEX approval rejected with code {}: {}",
            envelope.code, envelope.msg
        ));
    }
    envelope
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "OKX DEX approval response has no data row".to_owned())
}

pub(super) fn provider_quote(quote: OkxDexQuote, chain_id: u64) -> Result<ProviderQuote, String> {
    if quote.chain_index != chain_id.to_string() {
        return Err(format!(
            "OKX DEX quote chain {} does not match requested chain {chain_id}",
            quote.chain_index
        ));
    }
    Ok(ProviderQuote {
        input_address: quote.from_token.token_contract_address,
        output_address: quote.to_token.token_contract_address,
        input_amount_raw: quote.from_token_amount,
        output_amount_raw: quote.to_token_amount,
        router: quote.router,
    })
}
