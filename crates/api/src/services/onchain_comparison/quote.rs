use super::jupiter_quota;
use super::provider_runtime::{env_key, provider_runtime};
use super::provider_types::{JupiterOrderQuote, ZeroExPrice};
use super::{cow, okx};
use onchain_monitor::{OnchainQuotePair, ProviderQuote};
use shared_types::{onchain_chain_preset, OnchainComparisonConfig};
use std::sync::OnceLock;

pub(super) const JUPITER_ORDER_ENDPOINT: &str = "https://api.jup.ag/swap/v2/order";
pub(super) const JUPITER_ORDER_DOCS: &str = "https://developers.jup.ag/docs/swap/order-and-execute";
pub(super) const ZEROEX_PRICE_ENDPOINT: &str = "https://api.0x.org/swap/allowance-holder/price";
pub(super) const ZEROEX_PRICE_DOCS: &str =
    "https://docs.0x.org/api-reference/evm-ap-is/swap/allowanceholder-getprice";
pub(super) const JUPITER_HOST: &str = "api.jup.ag";
pub(super) const ZEROEX_HOST: &str = "api.0x.org";
const QUOTE_CONNECT_TIMEOUT_SECS: u64 = 4;
const QUOTE_REQUEST_TIMEOUT_SECS: u64 = 8;
const JUPITER_REQUEST_TIMEOUT_SECS: u64 = 4;

pub(super) async fn fetch_pair(
    config: &OnchainComparisonConfig,
    started_at_ms: i64,
) -> Result<OnchainQuotePair, String> {
    let runtime = provider_runtime(&config.provider);
    if !runtime.configured {
        return Err(runtime
            .problem
            .unwrap_or_else(|| "quote provider is unavailable".to_owned()));
    }
    let (forward, reverse, endpoint, docs) = match config.provider.as_str() {
        "jupiter_swap_v2" | "jupiter_swap_v2_keyed" => {
            let keyed = config.provider == "jupiter_swap_v2_keyed";
            let (forward, reverse, endpoint, docs) = fetch_jupiter_pair(config, keyed).await?;
            (forward, reverse, endpoint.to_owned(), docs)
        }
        "zeroex_swap_v2" => {
            let (forward, reverse, endpoint, docs) = fetch_zeroex_pair(config).await?;
            (forward, reverse, endpoint.to_owned(), docs)
        }
        "okx_dex_v6" => {
            let (forward, reverse, endpoint, docs) = okx::fetch_pair(config).await?;
            (forward, reverse, endpoint.to_owned(), docs)
        }
        "cow_protocol" => cow::fetch_pair(config).await?,
        provider => return Err(format!("unsupported quote provider {provider}")),
    };
    let observed_at_ms = common::time::now_ms();
    Ok(OnchainQuotePair {
        chain: config.chain.clone(),
        provider: config.provider.clone(),
        endpoint,
        official_docs_url: docs.to_owned(),
        base_address: config.base_mint.clone(),
        quote_address: config.quote_mint.clone(),
        forward,
        reverse,
        observed_at_ms,
        request_latency_ms: observed_at_ms.saturating_sub(started_at_ms),
        quote_interval_ms: runtime.quote_interval_ms,
    })
}

pub(super) async fn fetch_exact_in(
    config: &OnchainComparisonConfig,
    provider: &str,
    input_token: &str,
    output_token: &str,
    input_amount_raw: &str,
) -> Result<ProviderQuote, String> {
    let runtime = provider_runtime(provider);
    if !runtime.configured {
        return Err(runtime
            .problem
            .unwrap_or_else(|| format!("quote provider {provider} is unavailable")));
    }
    match provider {
        "jupiter_swap_v2" | "jupiter_swap_v2_keyed" => {
            let api_key = if provider == "jupiter_swap_v2_keyed" {
                Some(env_key("JUPITER_API_KEY").ok_or_else(|| {
                    "JUPITER_API_KEY is required for the keyed Jupiter route".to_owned()
                })?)
            } else {
                None
            };
            fetch_jupiter_quote(
                quote_client(),
                api_key.as_deref(),
                input_token,
                output_token,
                input_amount_raw,
            )
            .await
            .map(jupiter_provider_quote)
        }
        "zeroex_swap_v2" => {
            let chain_id = onchain_chain_preset(&config.chain)
                .and_then(|preset| preset.chain_id)
                .ok_or_else(|| format!("chain {} has no EVM chain id", config.chain))?;
            let api_key = env_key("ZEROX_API_KEY")
                .ok_or_else(|| "ZEROX_API_KEY is required for 0x Swap API".to_owned())?;
            fetch_zeroex_price(
                quote_client(),
                &api_key,
                chain_id,
                input_token,
                output_token,
                input_amount_raw,
            )
            .await
            .and_then(zeroex_provider_quote)
        }
        "okx_dex_v6" => {
            okx::fetch_exact_in(config, input_token, output_token, input_amount_raw).await
        }
        "cow_protocol" => {
            cow::fetch_exact_in(config, input_token, output_token, input_amount_raw).await
        }
        provider => Err(format!("unsupported quote provider {provider}")),
    }
}

async fn fetch_jupiter_pair(
    config: &OnchainComparisonConfig,
    keyed: bool,
) -> Result<(ProviderQuote, ProviderQuote, &'static str, &'static str), String> {
    let api_key =
        if keyed {
            Some(env_key("JUPITER_API_KEY").ok_or_else(|| {
                "JUPITER_API_KEY is required for the keyed Jupiter route".to_owned()
            })?)
        } else {
            None
        };
    let reverse = jupiter_provider_quote(
        fetch_jupiter_quote(
            quote_client(),
            api_key.as_deref(),
            &config.quote_mint,
            &config.base_mint,
            &config.quote_amount_raw,
        )
        .await?,
    );
    let base_amount_raw = anchored_base_amount(&reverse)?;
    let forward = jupiter_provider_quote(
        fetch_jupiter_quote(
            quote_client(),
            api_key.as_deref(),
            &config.base_mint,
            &config.quote_mint,
            &base_amount_raw,
        )
        .await?,
    );
    Ok((forward, reverse, JUPITER_ORDER_ENDPOINT, JUPITER_ORDER_DOCS))
}

async fn fetch_zeroex_pair(
    config: &OnchainComparisonConfig,
) -> Result<(ProviderQuote, ProviderQuote, &'static str, &'static str), String> {
    let preset = onchain_chain_preset(&config.chain)
        .ok_or_else(|| format!("unsupported 0x chain {}", config.chain))?;
    let chain_id = preset
        .chain_id
        .ok_or_else(|| format!("chain {} has no EVM chain id", config.chain))?;
    let api_key = env_key("ZEROX_API_KEY")
        .ok_or_else(|| "ZEROX_API_KEY is required for 0x Swap API".to_owned())?;
    let reverse = zeroex_provider_quote(
        fetch_zeroex_price(
            quote_client(),
            &api_key,
            chain_id,
            &config.quote_mint,
            &config.base_mint,
            &config.quote_amount_raw,
        )
        .await?,
    )?;
    let base_amount_raw = anchored_base_amount(&reverse)?;
    let forward = zeroex_provider_quote(
        fetch_zeroex_price(
            quote_client(),
            &api_key,
            chain_id,
            &config.base_mint,
            &config.quote_mint,
            &base_amount_raw,
        )
        .await?,
    )?;
    Ok((forward, reverse, ZEROEX_PRICE_ENDPOINT, ZEROEX_PRICE_DOCS))
}

pub(super) fn anchored_base_amount(reverse: &ProviderQuote) -> Result<String, String> {
    reverse
        .output_amount_raw
        .parse::<u128>()
        .ok()
        .filter(|amount| *amount > 0)
        .map(|_| reverse.output_amount_raw.clone())
        .ok_or_else(|| "quote-to-base response returned an invalid base amount".to_owned())
}

async fn fetch_jupiter_quote(
    client: &reqwest::Client,
    api_key: Option<&str>,
    input_mint: &str,
    output_mint: &str,
    amount: &str,
) -> Result<JupiterOrderQuote, String> {
    let body = fetch_jupiter_quote_body(client, api_key, input_mint, output_mint, amount).await?;
    serde_json::from_str(&body).map_err(|error| format!("Jupiter quote decode failed: {error}"))
}

pub(super) async fn fetch_jupiter_quote_body(
    client: &reqwest::Client,
    api_key: Option<&str>,
    input_mint: &str,
    output_mint: &str,
    amount: &str,
) -> Result<String, String> {
    fetch_jupiter_quote_body_at(
        client,
        JUPITER_ORDER_ENDPOINT,
        api_key,
        input_mint,
        output_mint,
        amount,
    )
    .await
}

pub(super) async fn fetch_jupiter_quote_body_at(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: Option<&str>,
    input_mint: &str,
    output_mint: &str,
    amount: &str,
) -> Result<String, String> {
    let keyed = api_key.is_some();
    jupiter_quota::wait_for_quote_request(keyed)
        .await
        .map_err(|retry_after_ms| {
            format!(
                "Jupiter 官方配额窗口尚未释放；系统会在窗口开放后自动刷新，无需手动重试 · source=jupiter_rate_limit · rate_limit_wait · retry_after_ms={retry_after_ms}"
            )
        })?;
    let mut request = client
        .get(endpoint)
        .timeout(std::time::Duration::from_secs(JUPITER_REQUEST_TIMEOUT_SECS))
        .query(&[
            ("inputMint", input_mint),
            ("outputMint", output_mint),
            ("amount", amount),
        ]);
    if let Some(api_key) = api_key {
        request = request.header("x-api-key", api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|error| transport_problem("Jupiter", "报价", JUPITER_HOST, &error))?;
    decode_jupiter_general_response(response, keyed, "Jupiter").await
}

async fn fetch_zeroex_price(
    client: &reqwest::Client,
    api_key: &str,
    chain_id: u64,
    sell_token: &str,
    buy_token: &str,
    sell_amount: &str,
) -> Result<ZeroExPrice, String> {
    let response = client
        .get(ZEROEX_PRICE_ENDPOINT)
        .header("0x-api-key", api_key)
        .header("0x-version", "v2")
        .query(&[
            ("chainId", chain_id.to_string()),
            ("sellToken", sell_token.to_owned()),
            ("buyToken", buy_token.to_owned()),
            ("sellAmount", sell_amount.to_owned()),
        ])
        .send()
        .await
        .map_err(|error| transport_problem("0x", "报价", ZEROEX_HOST, &error))?;
    decode_response(response, "0x").await.and_then(|body| {
        serde_json::from_str(&body).map_err(|error| format!("0x price decode failed: {error}"))
    })
}

pub(super) async fn decode_response(
    response: reqwest::Response,
    provider: &str,
) -> Result<String, String> {
    decode_response_with_retry_hint(response, provider, None).await
}

pub(super) async fn decode_jupiter_general_response(
    response: reqwest::Response,
    keyed: bool,
    provider: &str,
) -> Result<String, String> {
    let retry_after_ms = jupiter_quota::observe_general_response(
        keyed,
        response.status(),
        response.headers(),
        common::time::now_ms(),
    )
    .await;
    decode_response_with_retry_hint(response, provider, retry_after_ms).await
}

async fn decode_response_with_retry_hint(
    response: reqwest::Response,
    provider: &str,
    retry_after_ms: Option<i64>,
) -> Result<String, String> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .text()
        .await
        .map_err(|error| format!("{provider} response read failed: {error}"))?;
    if status.is_success() {
        return Ok(body);
    }
    let detail = body.chars().take(240).collect::<String>();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry = retry_after_ms.map_or_else(
            || {
                retry_after
                    .map(|value| format!("；上游建议 {value} 秒后重试"))
                    .unwrap_or_default()
            },
            |delay_ms| {
                format!(
                    "；上游配额窗口约 {} 秒后释放",
                    delay_ms.saturating_add(999) / 1_000
                )
            },
        );
        return Err(format!(
            "{provider} API 已限速（HTTP 429）{retry}；系统会按官方配额自动退避，无需反复点击重试：{detail}"
        ));
    }
    Err(format!("{provider} quote returned HTTP {status}: {detail}"))
}

pub(super) fn transport_problem(
    provider: &str,
    operation: &str,
    host: &str,
    error: &reqwest::Error,
) -> String {
    if error.is_timeout() {
        format!(
            "{provider} {operation}超时（{host}）；请检查本机代理或上游连通性，系统会自动退避重试 · transport=timeout"
        )
    } else if error.is_connect() {
        format!(
            "{provider} {operation}无法连接（{host}）；请检查本机代理与域名连通性，系统会自动退避重试 · transport=request failed"
        )
    } else {
        format!(
            "{provider} {operation}网络传输失败（{host}）；系统会自动退避重试 · transport=request failed"
        )
    }
}

pub(super) fn jupiter_provider_quote(quote: JupiterOrderQuote) -> ProviderQuote {
    ProviderQuote {
        input_address: quote.input_mint,
        output_address: quote.output_mint,
        input_amount_raw: quote.in_amount,
        output_amount_raw: quote.out_amount,
        router: quote.router,
    }
}

pub(super) fn zeroex_provider_quote(quote: ZeroExPrice) -> Result<ProviderQuote, String> {
    if quote.liquidity_available == Some(false) {
        return Err("0x price reports no route liquidity".to_owned());
    }
    let input_amount_raw = quote
        .sell_amount
        .ok_or_else(|| "0x price response missing sellAmount".to_owned())?;
    let output_amount_raw = quote
        .buy_amount
        .ok_or_else(|| "0x price response missing buyAmount".to_owned())?;
    let router = quote.route.and_then(|route| {
        let mut sources = route
            .fills
            .into_iter()
            .map(|fill| fill.source)
            .filter(|source| !source.trim().is_empty())
            .collect::<Vec<_>>();
        sources.sort();
        sources.dedup();
        (!sources.is_empty()).then(|| sources.join(" + "))
    });
    Ok(ProviderQuote {
        input_address: quote.sell_token,
        output_address: quote.buy_token,
        input_amount_raw,
        output_amount_raw,
        router,
    })
}

pub(super) fn quote_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(QUOTE_CONNECT_TIMEOUT_SECS))
            .timeout(std::time::Duration::from_secs(QUOTE_REQUEST_TIMEOUT_SECS))
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

pub(super) fn quote_price(
    quote: &ProviderQuote,
    input_decimals: u8,
    output_decimals: u8,
) -> Option<f64> {
    Some(
        raw_units(&quote.output_amount_raw, output_decimals)?
            / raw_units(&quote.input_amount_raw, input_decimals)?,
    )
}

pub(super) fn raw_units(value: &str, decimals: u8) -> Option<f64> {
    let raw = value.parse::<u128>().ok()?;
    (raw > 0).then(|| raw as f64 / 10_f64.powi(i32::from(decimals)))
}
