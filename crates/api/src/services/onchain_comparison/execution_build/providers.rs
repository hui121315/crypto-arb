use super::super::jupiter_quota;
use super::super::okx::{self, OkxSwapRequest, OKX_SWAP_DOCS};
use super::super::provider_runtime::env_key;
use super::super::provider_types::{JupiterOrderBuild, ZeroExFirmQuote};
use super::super::quote::{
    decode_jupiter_general_response, decode_response, quote_client, transport_problem,
    JUPITER_HOST, JUPITER_ORDER_DOCS, ZEROEX_HOST,
};
use onchain_monitor::ProviderQuote;
use shared_types::{
    onchain_chain_preset, OnchainComparisonConfig, OnchainComparisonDirection,
    OnchainUnsignedTransaction, EVM_NATIVE_TOKEN_ADDRESS,
};

const ZEROEX_QUOTE_ENDPOINT: &str = "https://api.0x.org/swap/allowance-holder/quote";
const ZEROEX_QUOTE_DOCS: &str = "https://docs.0x.org/docs/upgrading/upgrading-to-swap-v2";

pub(super) struct FirmChainBuild {
    pub(super) quote: ProviderQuote,
    pub(super) minimum_output_amount_raw: String,
    pub(super) transaction: OnchainUnsignedTransaction,
    pub(super) official_docs_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TokenApprovalRequirement {
    pub(super) token_address: String,
    pub(super) spender: String,
    pub(super) required_amount_raw: String,
    pub(super) current_allowance_raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FirmBuildError {
    TokenApprovalRequired(TokenApprovalRequirement),
    Rejected(String),
}

fn zeroex_allowance_requirement(
    allowance: &super::super::provider_types::ZeroExAllowanceIssue,
    token_address: &str,
    required_amount_raw: &str,
) -> TokenApprovalRequirement {
    TokenApprovalRequirement {
        token_address: token_address.to_owned(),
        spender: allowance.spender.clone(),
        required_amount_raw: required_amount_raw.to_owned(),
        current_allowance_raw: allowance
            .actual
            .clone()
            .unwrap_or_else(|| "unknown".to_owned()),
    }
}

impl From<String> for FirmBuildError {
    fn from(problem: String) -> Self {
        Self::Rejected(problem)
    }
}

pub(super) async fn build(
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    input_amount_raw: &str,
) -> Result<FirmChainBuild, FirmBuildError> {
    let (input_token, output_token) = trade_tokens(config, direction);
    match config.provider.as_str() {
        "jupiter_swap_v2" | "jupiter_swap_v2_keyed" => {
            build_jupiter(config, input_token, output_token, input_amount_raw)
                .await
                .map_err(Into::into)
        }
        "zeroex_swap_v2" => build_zeroex(config, input_token, output_token, input_amount_raw).await,
        "okx_dex_v6" => build_okx(config, input_token, output_token, input_amount_raw)
            .await
            .map_err(Into::into),
        "cow_protocol" => Err(FirmBuildError::Rejected(
            "CoW Protocol 仍需要 EIP-712 order 签名与 solver 终态接线，当前禁止构建".to_owned(),
        )),
        provider => Err(FirmBuildError::Rejected(format!(
            "provider {provider} does not support firm execution builds"
        ))),
    }
}

fn trade_tokens(
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
) -> (&str, &str) {
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => (&config.quote_mint, &config.base_mint),
        OnchainComparisonDirection::BuyCexSellOnchain => (&config.base_mint, &config.quote_mint),
    }
}

async fn build_jupiter(
    config: &OnchainComparisonConfig,
    input_token: &str,
    output_token: &str,
    input_amount_raw: &str,
) -> Result<FirmChainBuild, String> {
    let api_key = match config.provider.as_str() {
        "jupiter_swap_v2_keyed" => Some(
            env_key("JUPITER_API_KEY")
                .ok_or_else(|| "JUPITER_API_KEY is required for this route".to_owned())?,
        ),
        _ => None,
    };
    let slippage_bps = slippage_bps(config)?;
    let mut request = quote_client()
        .get(super::super::quote::JUPITER_ORDER_ENDPOINT)
        .query(&[
            ("inputMint", input_token),
            ("outputMint", output_token),
            ("amount", input_amount_raw),
            ("taker", config.wallet_address.as_str()),
            ("slippageBps", slippage_bps.as_str()),
        ]);
    if let Some(api_key) = api_key.as_deref() {
        request = request.header("x-api-key", api_key);
    }
    jupiter_quota::wait_for_general_request(api_key.is_some()).await;
    let response = request
        .send()
        .await
        .map_err(|error| transport_problem("Jupiter", "交易计划", JUPITER_HOST, &error))?;
    let body = decode_jupiter_general_response(response, api_key.is_some(), "Jupiter").await?;
    let order: JupiterOrderBuild = serde_json::from_str(&body)
        .map_err(|error| format!("Jupiter order decode failed: {error}"))?;
    validate_quote_identity(
        "Jupiter",
        config,
        &order.input_mint,
        &order.output_mint,
        input_token,
        output_token,
    )?;
    if order.in_amount != input_amount_raw {
        return Err("Jupiter firm order changed the requested input amount".to_owned());
    }
    ensure_raw_amount("Jupiter outAmount", &order.out_amount)?;
    let minimum_output_amount_raw = minimum_output_from_bps(
        &order.out_amount,
        slippage_bps
            .parse::<u32>()
            .map_err(|_| "Jupiter slippageBps could not be converted to an integer".to_owned())?,
    )?;
    let transaction = order
        .transaction
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "Jupiter returned pricing but no buildable transaction: router={} code={} {}",
                order.router,
                order
                    .error_code
                    .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
                order
                    .error_message
                    .unwrap_or_else(|| "no error detail".to_owned())
            )
        })?;
    if order.request_id.trim().is_empty() {
        return Err("Jupiter order response is missing requestId".to_owned());
    }
    Ok(FirmChainBuild {
        quote: ProviderQuote {
            input_address: order.input_mint,
            output_address: order.output_mint,
            input_amount_raw: order.in_amount,
            output_amount_raw: order.out_amount,
            router: Some(order.router.clone()),
        },
        minimum_output_amount_raw,
        transaction: OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64: transaction,
            request_id: order.request_id,
            router: order.router,
            mode: order.mode,
            last_valid_block_height: order.last_valid_block_height,
            expire_at_ms: parse_expire_at_ms(order.expire_at.as_ref()),
        },
        official_docs_url: JUPITER_ORDER_DOCS.to_owned(),
    })
}

async fn build_zeroex(
    config: &OnchainComparisonConfig,
    input_token: &str,
    output_token: &str,
    input_amount_raw: &str,
) -> Result<FirmChainBuild, FirmBuildError> {
    let chain_id = evm_chain_id(config)?;
    let api_key = env_key("ZEROX_API_KEY")
        .ok_or_else(|| "ZEROX_API_KEY is required for a firm 0x quote".to_owned())?;
    let response = quote_client()
        .get(ZEROEX_QUOTE_ENDPOINT)
        .header("0x-api-key", api_key)
        .header("0x-version", "v2")
        .query(&[
            ("chainId", chain_id.to_string()),
            ("sellToken", input_token.to_owned()),
            ("buyToken", output_token.to_owned()),
            ("sellAmount", input_amount_raw.to_owned()),
            ("taker", config.wallet_address.clone()),
            ("slippageBps", slippage_bps(config)?),
        ])
        .send()
        .await
        .map_err(|error| transport_problem("0x", "交易计划", ZEROEX_HOST, &error))?;
    let body = decode_response(response, "0x").await?;
    let quote: ZeroExFirmQuote = serde_json::from_str(&body)
        .map_err(|error| format!("0x firm quote decode failed: {error}"))?;
    if quote.liquidity_available == Some(false) {
        return Err("0x firm quote reports no route liquidity".to_owned().into());
    }
    let issues = quote.issues.unwrap_or_default();
    if let Some(allowance) = issues.allowance.as_ref() {
        return Err(FirmBuildError::TokenApprovalRequired(
            zeroex_allowance_requirement(allowance, input_token, input_amount_raw),
        ));
    }
    if issues.balance.is_some() {
        return Err("0x firm quote reports insufficient wallet balance"
            .to_owned()
            .into());
    }
    if issues.simulation_incomplete == Some(true) {
        return Err("0x firm quote simulation is incomplete".to_owned().into());
    }
    validate_quote_identity(
        "0x",
        config,
        &quote.sell_token,
        &quote.buy_token,
        input_token,
        output_token,
    )?;
    let sell_amount = quote
        .sell_amount
        .ok_or_else(|| "0x firm quote is missing sellAmount".to_owned())?;
    if sell_amount != input_amount_raw {
        return Err("0x firm quote changed the requested sell amount"
            .to_owned()
            .into());
    }
    let buy_amount = quote
        .buy_amount
        .ok_or_else(|| "0x firm quote is missing buyAmount".to_owned())?;
    ensure_raw_amount("0x buyAmount", &buy_amount)?;
    let minimum_output_amount_raw = quote
        .min_buy_amount
        .ok_or_else(|| "0x firm quote is missing minBuyAmount".to_owned())?;
    ensure_raw_amount("0x minBuyAmount", &minimum_output_amount_raw)?;
    if minimum_output_amount_raw
        .parse::<u128>()
        .ok()
        .zip(buy_amount.parse::<u128>().ok())
        .is_none_or(|(minimum, quoted)| minimum > quoted)
    {
        return Err("0x minBuyAmount exceeds buyAmount".to_owned().into());
    }
    let transaction = quote
        .transaction
        .ok_or_else(|| "0x firm quote is missing transaction calldata".to_owned())?;
    validate_evm_transaction(
        &config.wallet_address,
        &transaction.to,
        &transaction.data,
        &transaction.value,
        &transaction.gas,
    )?;
    let allowance_spender = (!input_token.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS))
        .then(|| transaction.to.clone());
    Ok(FirmChainBuild {
        quote: ProviderQuote {
            input_address: quote.sell_token,
            output_address: quote.buy_token,
            input_amount_raw: sell_amount,
            output_amount_raw: buy_amount,
            router: Some("0x-allowance-holder".to_owned()),
        },
        minimum_output_amount_raw,
        transaction: OnchainUnsignedTransaction::EvmCall {
            chain_id,
            from: config.wallet_address.clone(),
            to: transaction.to,
            data: transaction.data,
            value: transaction.value,
            gas: transaction.gas,
            gas_price: transaction.gas_price,
            max_priority_fee_per_gas: None,
            allowance_spender,
        },
        official_docs_url: ZEROEX_QUOTE_DOCS.to_owned(),
    })
}

async fn build_okx(
    config: &OnchainComparisonConfig,
    input_token: &str,
    output_token: &str,
    input_amount_raw: &str,
) -> Result<FirmChainBuild, String> {
    let chain_id = evm_chain_id(config)?;
    let slippage_percent = slippage_percent(config)?;
    let swap = okx::fetch_swap(OkxSwapRequest {
        chain_id,
        from_token: input_token,
        to_token: output_token,
        amount: input_amount_raw,
        slippage_percent: &slippage_percent,
        wallet_address: &config.wallet_address,
    })
    .await?;
    let route = swap.router_result;
    if route.chain_index != chain_id.to_string() {
        return Err("OKX DEX swap returned a different chain".to_owned());
    }
    validate_quote_identity(
        "OKX DEX",
        config,
        &route.from_token.token_contract_address,
        &route.to_token.token_contract_address,
        input_token,
        output_token,
    )?;
    if route.from_token_amount != input_amount_raw {
        return Err("OKX DEX swap changed the requested input amount".to_owned());
    }
    ensure_raw_amount("OKX DEX toTokenAmount", &route.to_token_amount)?;
    let minimum_output_amount_raw =
        minimum_output_from_bps(&route.to_token_amount, config.slippage_bps.floor() as u32)?;
    let transaction = swap.tx;
    if !transaction
        .from
        .eq_ignore_ascii_case(&config.wallet_address)
    {
        return Err(
            "OKX DEX swap transaction sender does not match the configured wallet".to_owned(),
        );
    }
    validate_evm_transaction(
        &transaction.from,
        &transaction.to,
        &transaction.data,
        &transaction.value,
        &transaction.gas,
    )?;
    Ok(FirmChainBuild {
        quote: ProviderQuote {
            input_address: route.from_token.token_contract_address,
            output_address: route.to_token.token_contract_address,
            input_amount_raw: route.from_token_amount,
            output_amount_raw: route.to_token_amount,
            router: route.router,
        },
        minimum_output_amount_raw,
        transaction: OnchainUnsignedTransaction::EvmCall {
            chain_id,
            from: transaction.from,
            to: transaction.to,
            data: transaction.data,
            value: transaction.value,
            gas: transaction.gas,
            gas_price: transaction.gas_price,
            max_priority_fee_per_gas: transaction.max_priority_fee_per_gas,
            allowance_spender: None,
        },
        official_docs_url: OKX_SWAP_DOCS.to_owned(),
    })
}

fn evm_chain_id(config: &OnchainComparisonConfig) -> Result<u64, String> {
    onchain_chain_preset(&config.chain)
        .and_then(|preset| preset.chain_id)
        .ok_or_else(|| format!("chain {} has no EVM chain id", config.chain))
}

fn slippage_bps(config: &OnchainComparisonConfig) -> Result<String, String> {
    if !config.slippage_bps.is_finite() || !(0.0..=10_000.0).contains(&config.slippage_bps) {
        return Err("slippage must be between 0 and 10000 basis points".to_owned());
    }
    Ok(config.slippage_bps.floor().to_string())
}

fn slippage_percent(config: &OnchainComparisonConfig) -> Result<String, String> {
    if !config.slippage_bps.is_finite() || !(0.0..=10_000.0).contains(&config.slippage_bps) {
        return Err("slippage must be between 0 and 100 percent".to_owned());
    }
    Ok(format!("{:.6}", config.slippage_bps.floor() / 100.0))
}

pub(super) fn minimum_output_from_bps(
    output_amount_raw: &str,
    slippage_bps: u32,
) -> Result<String, String> {
    if slippage_bps > 10_000 {
        return Err("slippage must be between 0 and 10000 basis points".to_owned());
    }
    let output = output_amount_raw
        .parse::<u128>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "quoted output amount is not a positive integer".to_owned())?;
    let minimum = output
        .checked_mul(u128::from(10_000_u32.saturating_sub(slippage_bps)))
        .map(|value| value / 10_000)
        .filter(|value| *value > 0)
        .ok_or_else(|| "minimum output amount is zero or overflowed".to_owned())?;
    Ok(minimum.to_string())
}

fn validate_quote_identity(
    provider: &str,
    config: &OnchainComparisonConfig,
    actual_input: &str,
    actual_output: &str,
    expected_input: &str,
    expected_output: &str,
) -> Result<(), String> {
    let matches = if config.chain.eq_ignore_ascii_case("solana") {
        actual_input == expected_input && actual_output == expected_output
    } else {
        actual_input.eq_ignore_ascii_case(expected_input)
            && actual_output.eq_ignore_ascii_case(expected_output)
    };
    if matches {
        Ok(())
    } else {
        Err(format!(
            "{provider} firm quote token identity does not match the configured pair"
        ))
    }
}

fn ensure_raw_amount(label: &str, value: &str) -> Result<(), String> {
    value
        .parse::<u128>()
        .ok()
        .filter(|amount| *amount > 0)
        .map(|_| ())
        .ok_or_else(|| format!("{label} is not a positive integer"))
}

fn validate_evm_transaction(
    from: &str,
    to: &str,
    data: &str,
    value: &str,
    gas: &str,
) -> Result<(), String> {
    let address = |value: &str| {
        value.len() == 42
            && value.starts_with("0x")
            && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    if !address(from) || !address(to) || !data.starts_with("0x") || data.len() <= 2 {
        return Err("firm EVM transaction contains invalid from/to/calldata fields".to_owned());
    }
    for (label, value) in [("value", value), ("gas", gas)] {
        if value.trim().is_empty() {
            return Err(format!("firm EVM transaction is missing {label}"));
        }
    }
    Ok(())
}

fn parse_expire_at_ms(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    if let Some(number) = value.as_i64() {
        return Some(if number < 10_000_000_000 {
            number.saturating_mul(1_000)
        } else {
            number
        });
    }
    let text = value.as_str()?;
    text.parse::<i64>()
        .ok()
        .map(|number| {
            if number < 10_000_000_000 {
                number.saturating_mul(1_000)
            } else {
                number
            }
        })
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|value| value.timestamp_millis())
        })
}

#[cfg(test)]
mod tests {
    use super::super::super::provider_types::ZeroExAllowanceIssue;
    use super::*;

    #[test]
    fn minimum_output_uses_exact_integer_units() {
        assert_eq!(
            minimum_output_from_bps("100000001", 10),
            Ok("99900000".to_owned())
        );
        assert!(minimum_output_from_bps("1", 10_000).is_err());
    }

    #[test]
    fn provider_direction_uses_the_correct_chain_leg() {
        let config = OnchainComparisonConfig::default();
        assert_eq!(
            trade_tokens(&config, OnchainComparisonDirection::BuyOnchainSellCex),
            (config.quote_mint.as_str(), config.base_mint.as_str())
        );
        assert_eq!(
            trade_tokens(&config, OnchainComparisonDirection::BuyCexSellOnchain),
            (config.base_mint.as_str(), config.quote_mint.as_str())
        );
    }

    #[test]
    fn expiry_parser_accepts_seconds_milliseconds_and_rfc3339() {
        assert_eq!(
            parse_expire_at_ms(Some(&serde_json::json!(1_700_000_000))),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_expire_at_ms(Some(&serde_json::json!(1_700_000_000_123_i64))),
            Some(1_700_000_000_123)
        );
        assert_eq!(
            parse_expire_at_ms(Some(&serde_json::json!("2026-08-11T00:00:00Z"))),
            Some(1_786_406_400_000)
        );
    }

    #[test]
    fn zeroex_allowance_requirement_preserves_exact_contract_values() {
        let requirement = zeroex_allowance_requirement(
            &ZeroExAllowanceIssue {
                actual: Some("0".to_owned()),
                spender: "0x0000000000001fF3684f28c67538d4D072C22734".to_owned(),
            },
            "0x1111111111111111111111111111111111111111",
            "1000000",
        );

        assert_eq!(
            requirement,
            TokenApprovalRequirement {
                token_address: "0x1111111111111111111111111111111111111111".to_owned(),
                spender: "0x0000000000001fF3684f28c67538d4D072C22734".to_owned(),
                required_amount_raw: "1000000".to_owned(),
                current_allowance_raw: "0".to_owned(),
            }
        );
    }
}
