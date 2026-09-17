use super::quote::{decode_response, quote_client, transport_problem};
use onchain_monitor::ProviderQuote;
use serde::{Deserialize, Serialize};
use shared_types::{OnchainComparisonConfig, EVM_NATIVE_TOKEN_ADDRESS};

pub(super) const COW_QUOTE_DOCS: &str = "https://api.cow.fi/docs/#/default/post_api_v1_quote";
pub(super) const COW_INTERVAL_MS: i64 = 1_000;

const COW_API_ROOT: &str = "https://api.cow.fi";
const COW_OBSERVER_ADDRESS: &str = "0x0000000000000000000000000000000000000001";
const COW_QUOTE_TIMEOUT_MS: u64 = 1_000;

#[derive(Clone, Copy)]
struct CowNetwork {
    slug: &'static str,
    wrapped_native: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CowQuoteRequest<'a> {
    sell_token: &'a str,
    buy_token: &'a str,
    from: &'static str,
    kind: &'static str,
    sell_amount_before_fee: &'a str,
    price_quality: &'static str,
    signing_scheme: &'static str,
    onchain_order: bool,
    timeout: u64,
}

#[derive(Deserialize)]
struct CowQuoteResponse {
    quote: CowOrderQuote,
    verified: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CowOrderQuote {
    sell_token: String,
    buy_token: String,
    sell_amount: String,
    buy_amount: String,
    fee_amount: String,
    kind: String,
}

pub(super) async fn fetch_pair(
    config: &OnchainComparisonConfig,
) -> Result<(ProviderQuote, ProviderQuote, String, &'static str), String> {
    let network = network(&config.chain)
        .ok_or_else(|| format!("CoW Protocol does not support chain {}", config.chain))?;
    let endpoint = format!("{COW_API_ROOT}/{}/api/v1/quote", network.slug);
    let base = quote_token_address(&config.chain, &config.base_mint)
        .ok_or_else(|| format!("CoW Protocol does not support chain {}", config.chain))?;
    let quote = quote_token_address(&config.chain, &config.quote_mint)
        .ok_or_else(|| format!("CoW Protocol does not support chain {}", config.chain))?;
    let reverse = fetch_quote(
        quote_client(),
        &endpoint,
        quote,
        base,
        &config.quote_amount_raw,
    )
    .await?;
    let base_amount_raw = super::quote::anchored_base_amount(&reverse)?;
    let forward = fetch_quote(quote_client(), &endpoint, base, quote, &base_amount_raw).await?;
    Ok((forward, reverse, endpoint, COW_QUOTE_DOCS))
}

pub(super) async fn fetch_exact_in(
    config: &OnchainComparisonConfig,
    input_token: &str,
    output_token: &str,
    input_amount_raw: &str,
) -> Result<ProviderQuote, String> {
    let network = network(&config.chain)
        .ok_or_else(|| format!("CoW Protocol does not support chain {}", config.chain))?;
    let endpoint = format!("{COW_API_ROOT}/{}/api/v1/quote", network.slug);
    let input = quote_token_address(&config.chain, input_token)
        .ok_or_else(|| format!("CoW Protocol does not support chain {}", config.chain))?;
    let output = quote_token_address(&config.chain, output_token)
        .ok_or_else(|| format!("CoW Protocol does not support chain {}", config.chain))?;
    fetch_quote(quote_client(), &endpoint, input, output, input_amount_raw).await
}

pub(super) fn quote_token_address<'a>(chain: &str, configured: &'a str) -> Option<&'a str> {
    let network = network(chain)?;
    if configured.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS) {
        Some(network.wrapped_native)
    } else {
        Some(configured)
    }
}

async fn fetch_quote(
    client: &reqwest::Client,
    endpoint: &str,
    sell_token: &str,
    buy_token: &str,
    sell_amount: &str,
) -> Result<ProviderQuote, String> {
    let response = build_quote_request(client, endpoint, sell_token, buy_token, sell_amount)
        .send()
        .await
        .map_err(|error| transport_problem("CoW", "报价", "api.cow.fi", &error))?;
    let body = decode_response(response, "CoW").await?;
    decode_quote(&body, sell_token, buy_token, sell_amount)
}

pub(super) fn build_quote_request(
    client: &reqwest::Client,
    endpoint: &str,
    sell_token: &str,
    buy_token: &str,
    sell_amount: &str,
) -> reqwest::RequestBuilder {
    client.post(endpoint).json(&CowQuoteRequest {
        sell_token,
        buy_token,
        from: COW_OBSERVER_ADDRESS,
        kind: "sell",
        sell_amount_before_fee: sell_amount,
        price_quality: "fast",
        signing_scheme: "eip712",
        onchain_order: false,
        timeout: COW_QUOTE_TIMEOUT_MS,
    })
}

pub(super) fn decode_quote(
    body: &str,
    sell_token: &str,
    buy_token: &str,
    requested_sell_amount: &str,
) -> Result<ProviderQuote, String> {
    let response: CowQuoteResponse =
        serde_json::from_str(body).map_err(|error| format!("CoW quote decode failed: {error}"))?;
    let quote = response.quote;
    if quote.kind != "sell"
        || !quote.sell_token.eq_ignore_ascii_case(sell_token)
        || !quote.buy_token.eq_ignore_ascii_case(buy_token)
    {
        return Err("CoW quote identity or side does not match the request".to_owned());
    }
    let requested = parse_positive_amount(requested_sell_amount, "requested sell amount")?;
    let quoted = parse_positive_amount(&quote.sell_amount, "sellAmount")?;
    let fee = quote
        .fee_amount
        .parse::<u128>()
        .map_err(|_| "CoW quote feeAmount is invalid".to_owned())?;
    if quoted.checked_add(fee) != Some(requested) {
        return Err("CoW sellAmount plus feeAmount does not match sellAmountBeforeFee".to_owned());
    }
    parse_positive_amount(&quote.buy_amount, "buyAmount")?;
    let verification = if response.verified {
        "verified"
    } else {
        "unverified"
    };
    Ok(ProviderQuote {
        input_address: quote.sell_token,
        output_address: quote.buy_token,
        input_amount_raw: requested_sell_amount.to_owned(),
        output_amount_raw: quote.buy_amount,
        router: Some(format!("CoW fast · {verification}")),
    })
}

fn parse_positive_amount(value: &str, field: &str) -> Result<u128, String> {
    value
        .parse::<u128>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or_else(|| format!("CoW quote {field} is invalid"))
}

fn network(chain: &str) -> Option<CowNetwork> {
    let normalized = chain.trim();
    if normalized.eq_ignore_ascii_case("ethereum") {
        Some(CowNetwork {
            slug: "mainnet",
            wrapped_native: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
        })
    } else if normalized.eq_ignore_ascii_case("arbitrum") {
        Some(CowNetwork {
            slug: "arbitrum_one",
            wrapped_native: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
        })
    } else if normalized.eq_ignore_ascii_case("base") {
        Some(CowNetwork {
            slug: "base",
            wrapped_native: "0x4200000000000000000000000000000000000006",
        })
    } else if normalized.eq_ignore_ascii_case("polygon") {
        Some(CowNetwork {
            slug: "polygon",
            wrapped_native: "0x0d500b1d8e8ef31e21c99d1db9a6444d3adf1270",
        })
    } else if normalized.eq_ignore_ascii_case("bnb-smart-chain") {
        Some(CowNetwork {
            slug: "bnb",
            wrapped_native: "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c",
        })
    } else if normalized.eq_ignore_ascii_case("avalanche") {
        Some(CowNetwork {
            slug: "avalanche",
            wrapped_native: "0xb31f66aa3c1e785363f0875a1b74e27b85fd66c7",
        })
    } else {
        None
    }
}
