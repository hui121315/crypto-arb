use super::execution_submit;
use super::okx::{self, OkxApprovalRequest, OKX_APPROVAL_DOCS};
use super::provider_runtime::env_key;
use super::provider_types::ZeroExFirmQuote;
use super::quote::{decode_response, quote_client, transport_problem, ZEROEX_HOST};
use super::{rpc, rpc_target};
use crate::state::AppState;
use shared_types::{
    onchain_chain_preset, OnchainComparisonConfig, OnchainComparisonDirection,
    OnchainUnsignedTransaction, EVM_NATIVE_TOKEN_ADDRESS,
};

const ZEROEX_QUOTE_ENDPOINT: &str = "https://api.0x.org/swap/allowance-holder/quote";
const ZEROEX_ALLOWANCE_DOCS: &str = "https://docs.0x.org/docs/core-concepts/contracts";
const ALLOWANCE_SELECTOR: &str = "dd62ed3e";
const APPROVE_SELECTOR: &str = "095ea7b3";
const APPROVE_SELECTOR_BYTES: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TokenApprovalPlan {
    pub(super) token_address: String,
    pub(super) token_symbol: String,
    pub(super) token_decimals: u8,
    pub(super) spender: String,
    pub(super) required_amount_raw: String,
    pub(super) current_allowance_raw: String,
    pub(super) transactions: Vec<OnchainUnsignedTransaction>,
    pub(super) official_docs_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TokenAllowanceEvidence {
    pub(super) current_amount_raw: String,
    pub(super) sufficient: bool,
    pub(super) official_docs_url: &'static str,
}

struct ApprovalContext<'a> {
    rpc_url: &'a str,
    config: &'a OnchainComparisonConfig,
    chain_id: u64,
    token: &'a str,
    token_symbol: &'a str,
    token_decimals: u8,
    output: &'a str,
    required_amount_raw: &'a str,
    required: [u8; 32],
}

struct ApprovalCallContext<'a> {
    rpc_url: &'a str,
    chain_id: u64,
    wallet: &'a str,
    token: &'a str,
    spender: &'a str,
}

impl ApprovalCallContext<'_> {
    fn unsigned(
        &self,
        data: String,
        gas: String,
        gas_price: Option<String>,
    ) -> OnchainUnsignedTransaction {
        OnchainUnsignedTransaction::EvmCall {
            chain_id: self.chain_id,
            from: self.wallet.to_owned(),
            to: self.token.to_owned(),
            data,
            value: "0".to_owned(),
            gas,
            gas_price,
            max_priority_fee_per_gas: None,
            allowance_spender: Some(self.spender.to_owned()),
        }
    }
}

pub(super) async fn build_plan(
    state: &AppState,
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    required_amount_raw: &str,
) -> Result<TokenApprovalPlan, String> {
    let chain_id = onchain_chain_preset(&config.chain)
        .and_then(|preset| preset.chain_id)
        .ok_or_else(|| "SPL Token 不使用 EVM ERC-20 授权流程".to_owned())?;
    let (token, token_symbol, token_decimals, output) = approval_assets(config, direction);
    if token.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS) {
        return Err("原生 Gas 资产不需要 ERC-20 approve".to_owned());
    }
    let required = decimal_word(required_amount_raw)?;
    if word_is_zero(&required) {
        return Err("ERC-20 授权数量必须大于 0".to_owned());
    }
    let rpc_url = execution_submit::verified_submission_rpc(state, config).await?;
    let context = ApprovalContext {
        rpc_url: &rpc_url,
        config,
        chain_id,
        token,
        token_symbol,
        token_decimals,
        output,
        required_amount_raw,
        required,
    };
    match config.provider.as_str() {
        "zeroex_swap_v2" => build_zeroex_plan(&context).await,
        "okx_dex_v6" => build_okx_plan(&context).await,
        "cow_protocol" => Err(
            "CoW Protocol 还需要 EIP-712 订单签名与独立授权语义，当前禁止复用普通 approve 流程"
                .to_owned(),
        ),
        provider => Err(format!("Provider {provider} 尚未接入 ERC-20 授权计划")),
    }
}

pub(super) async fn inspect_exact_allowance(
    state: &AppState,
    config: &OnchainComparisonConfig,
    token: &str,
    spender: &str,
    required_amount_raw: &str,
) -> Result<TokenAllowanceEvidence, String> {
    if token.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS) {
        return Err("原生 Gas 资产不使用 ERC-20 allowance".to_owned());
    }
    let required = decimal_word(required_amount_raw)?;
    if word_is_zero(&required) {
        return Err("ERC-20 执行数量必须大于 0".to_owned());
    }
    let rpc_url = execution_submit::verified_submission_rpc(state, config).await?;
    let current = read_allowance(&rpc_url, &config.wallet_address, token, spender).await?;
    Ok(TokenAllowanceEvidence {
        current_amount_raw: word_decimal(current),
        sufficient: current >= required,
        official_docs_url: "https://eips.ethereum.org/EIPS/eip-20",
    })
}

fn approval_assets(
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
) -> (&str, &str, u8, &str) {
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => (
            &config.quote_mint,
            &config.quote_token,
            config.quote_decimals,
            &config.base_mint,
        ),
        OnchainComparisonDirection::BuyCexSellOnchain => (
            &config.base_mint,
            &config.base_token,
            config.base_decimals,
            &config.quote_mint,
        ),
    }
}

async fn build_zeroex_plan(context: &ApprovalContext<'_>) -> Result<TokenApprovalPlan, String> {
    let quote = fetch_zeroex_quote(
        context.config,
        context.chain_id,
        context.token,
        context.output,
        context.required_amount_raw,
    )
    .await?;
    validate_zeroex_quote(
        &quote,
        context.token,
        context.output,
        context.required_amount_raw,
    )?;
    let spender = quote
        .issues
        .as_ref()
        .and_then(|issues| issues.allowance.as_ref())
        .map(|allowance| allowance.spender.clone())
        .or(quote.allowance_target)
        .ok_or_else(|| "0x firm quote 没有返回 allowance target，已拒绝猜测授权地址".to_owned())?;
    let current = read_allowance(
        context.rpc_url,
        &context.config.wallet_address,
        context.token,
        &spender,
    )
    .await?;
    let transactions = if current >= context.required {
        Vec::new()
    } else {
        let call = ApprovalCallContext {
            rpc_url: context.rpc_url,
            chain_id: context.chain_id,
            wallet: &context.config.wallet_address,
            token: context.token,
            spender: &spender,
        };
        zeroex_approval_transactions(&call, current, context.required).await?
    };
    Ok(TokenApprovalPlan {
        token_address: context.token.to_owned(),
        token_symbol: context.token_symbol.to_owned(),
        token_decimals: context.token_decimals,
        spender,
        required_amount_raw: context.required_amount_raw.to_owned(),
        current_allowance_raw: word_decimal(current),
        transactions,
        official_docs_url: ZEROEX_ALLOWANCE_DOCS.to_owned(),
    })
}

async fn fetch_zeroex_quote(
    config: &OnchainComparisonConfig,
    chain_id: u64,
    token: &str,
    output: &str,
    amount: &str,
) -> Result<ZeroExFirmQuote, String> {
    let api_key = env_key("ZEROX_API_KEY")
        .ok_or_else(|| "ZEROX_API_KEY is required for a firm 0x quote".to_owned())?;
    let slippage = checked_slippage_bps(config.slippage_bps)?;
    let response = quote_client()
        .get(ZEROEX_QUOTE_ENDPOINT)
        .header("0x-api-key", api_key)
        .header("0x-version", "v2")
        .query(&[
            ("chainId", chain_id.to_string()),
            ("sellToken", token.to_owned()),
            ("buyToken", output.to_owned()),
            ("sellAmount", amount.to_owned()),
            ("taker", config.wallet_address.clone()),
            ("slippageBps", slippage),
        ])
        .send()
        .await
        .map_err(|error| transport_problem("0x", "授权核验", ZEROEX_HOST, &error))?;
    let body = decode_response(response, "0x").await?;
    serde_json::from_str(&body).map_err(|error| format!("0x firm quote decode failed: {error}"))
}

fn validate_zeroex_quote(
    quote: &ZeroExFirmQuote,
    token: &str,
    output: &str,
    amount: &str,
) -> Result<(), String> {
    if quote.liquidity_available == Some(false) {
        return Err("0x firm quote reports no route liquidity".to_owned());
    }
    if !quote.sell_token.eq_ignore_ascii_case(token)
        || !quote.buy_token.eq_ignore_ascii_case(output)
        || quote.sell_amount.as_deref() != Some(amount)
    {
        return Err("0x 授权核验返回了不同的代币或数量".to_owned());
    }
    Ok(())
}

async fn zeroex_approval_transactions(
    context: &ApprovalCallContext<'_>,
    current: [u8; 32],
    required: [u8; 32],
) -> Result<Vec<OnchainUnsignedTransaction>, String> {
    let mut transactions = Vec::with_capacity(if word_is_zero(&current) { 1 } else { 2 });
    if !word_is_zero(&current) {
        transactions.push(estimated_approval_call(context, [0; 32]).await?);
    }
    transactions.push(estimated_approval_call(context, required).await?);
    Ok(transactions)
}

async fn estimated_approval_call(
    context: &ApprovalCallContext<'_>,
    amount: [u8; 32],
) -> Result<OnchainUnsignedTransaction, String> {
    let data = encode_approve(context.spender, amount)?;
    let (url, client) = rpc_target::rpc_target(context.rpc_url).await?;
    let gas = rpc::rpc_result(
        &client,
        url.as_str(),
        "eth_estimateGas",
        serde_json::json!([{
            "from": context.wallet,
            "to": context.token,
            "data": data,
            "value": "0x0"
        }]),
        303,
    )
    .await?
    .as_str()
    .map(str::to_owned)
    .ok_or_else(|| "EVM RPC estimateGas 不是 hex quantity".to_owned())?;
    Ok(context.unsigned(data, gas, None))
}

async fn build_okx_plan(context: &ApprovalContext<'_>) -> Result<TokenApprovalPlan, String> {
    let approval = okx::fetch_approval(OkxApprovalRequest {
        chain_id: context.chain_id,
        token_contract_address: context.token,
        approve_amount: context.required_amount_raw,
    })
    .await?;
    validate_approve_calldata(
        &approval.data,
        &approval.dex_contract_address,
        context.required,
    )?;
    let current = read_allowance(
        context.rpc_url,
        &context.config.wallet_address,
        context.token,
        &approval.dex_contract_address,
    )
    .await?;
    let mut transactions = Vec::new();
    if current < context.required {
        if !word_is_zero(&current) {
            let reset = okx::fetch_approval(OkxApprovalRequest {
                chain_id: context.chain_id,
                token_contract_address: context.token,
                approve_amount: "0",
            })
            .await?;
            if !reset
                .dex_contract_address
                .eq_ignore_ascii_case(&approval.dex_contract_address)
            {
                return Err("OKX DEX 两次授权计划返回了不同 spender".to_owned());
            }
            validate_approve_calldata(&reset.data, &reset.dex_contract_address, [0; 32])?;
            transactions.push(okx_approval_call(context, &reset)?);
        }
        transactions.push(okx_approval_call(context, &approval)?);
    }
    Ok(TokenApprovalPlan {
        token_address: context.token.to_owned(),
        token_symbol: context.token_symbol.to_owned(),
        token_decimals: context.token_decimals,
        spender: approval.dex_contract_address,
        required_amount_raw: context.required_amount_raw.to_owned(),
        current_allowance_raw: word_decimal(current),
        transactions,
        official_docs_url: OKX_APPROVAL_DOCS.to_owned(),
    })
}

fn okx_approval_call(
    context: &ApprovalContext<'_>,
    approval: &super::provider_types::OkxApprovalTransaction,
) -> Result<OnchainUnsignedTransaction, String> {
    positive_quantity("OKX DEX gasLimit", &approval.gas_limit)?;
    positive_quantity("OKX DEX gasPrice", &approval.gas_price)?;
    Ok(ApprovalCallContext {
        rpc_url: context.rpc_url,
        chain_id: context.chain_id,
        wallet: &context.config.wallet_address,
        token: context.token,
        spender: &approval.dex_contract_address,
    }
    .unsigned(
        approval.data.clone(),
        approval.gas_limit.clone(),
        Some(approval.gas_price.clone()),
    ))
}

async fn read_allowance(
    rpc_url: &str,
    owner: &str,
    token: &str,
    spender: &str,
) -> Result<[u8; 32], String> {
    let data = encode_allowance(owner, spender)?;
    let (url, client) = rpc_target::rpc_target(rpc_url).await?;
    let result = rpc::rpc_result(
        &client,
        url.as_str(),
        "eth_call",
        serde_json::json!([{ "to": token, "data": data }, "latest"]),
        302,
    )
    .await?;
    result
        .as_str()
        .ok_or_else(|| "EVM allowance() 结果不是 hex data".to_owned())
        .and_then(hex_word)
}

fn encode_allowance(owner: &str, spender: &str) -> Result<String, String> {
    Ok(format!(
        "0x{ALLOWANCE_SELECTOR}{}{}",
        address_word(owner)?,
        address_word(spender)?,
    ))
}

fn encode_approve(spender: &str, amount: [u8; 32]) -> Result<String, String> {
    Ok(format!(
        "0x{APPROVE_SELECTOR}{}{}",
        address_word(spender)?,
        hex::encode(amount),
    ))
}

fn validate_approve_calldata(data: &str, spender: &str, amount: [u8; 32]) -> Result<(), String> {
    let bytes = data
        .strip_prefix("0x")
        .and_then(|value| hex::decode(value).ok())
        .filter(|value| value.len() == 68)
        .ok_or_else(|| "approve calldata 不是标准 ERC-20 approve(address,uint256)".to_owned())?;
    let spender_word = address_word_bytes(spender)?;
    if bytes[..4] != APPROVE_SELECTOR_BYTES || bytes[4..36] != spender_word || bytes[36..] != amount
    {
        return Err("approve calldata 的 spender 或金额与授权计划不一致".to_owned());
    }
    Ok(())
}

fn address_word(address: &str) -> Result<String, String> {
    address_word_bytes(address).map(hex::encode)
}

fn address_word_bytes(address: &str) -> Result<[u8; 32], String> {
    let address_bytes = address
        .strip_prefix("0x")
        .filter(|value| value.len() == 40)
        .and_then(|value| hex::decode(value).ok())
        .ok_or_else(|| format!("EVM 地址非法：{address}"))?;
    let mut word = [0_u8; 32];
    word[12..].copy_from_slice(&address_bytes);
    Ok(word)
}

fn decimal_word(value: &str) -> Result<[u8; 32], String> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("ERC-20 原始数量必须是十进制整数".to_owned());
    }
    let mut word = [0_u8; 32];
    for digit in value.bytes().map(|byte| byte - b'0') {
        let mut carry = u16::from(digit);
        for byte in word.iter_mut().rev() {
            let next = u16::from(*byte) * 10 + carry;
            *byte = next as u8;
            carry = next >> 8;
        }
        if carry != 0 {
            return Err("ERC-20 原始数量超过 uint256".to_owned());
        }
    }
    Ok(word)
}

fn hex_word(value: &str) -> Result<[u8; 32], String> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| "EVM uint256 结果缺少 0x".to_owned())?;
    let normalized = if hex.len() % 2 == 0 {
        hex.to_owned()
    } else {
        format!("0{hex}")
    };
    let bytes = hex::decode(normalized).map_err(|_| "EVM uint256 结果不是 hex".to_owned())?;
    if bytes.len() > 32 {
        return Err("EVM uint256 结果超过 32 bytes".to_owned());
    }
    let mut word = [0_u8; 32];
    word[32 - bytes.len()..].copy_from_slice(&bytes);
    Ok(word)
}

fn word_decimal(mut word: [u8; 32]) -> String {
    if word_is_zero(&word) {
        return "0".to_owned();
    }
    let mut digits = Vec::with_capacity(78);
    while !word_is_zero(&word) {
        let mut carry = 0_u16;
        for byte in &mut word {
            let value = carry * 256 + u16::from(*byte);
            *byte = (value / 10) as u8;
            carry = value % 10;
        }
        digits.push((carry as u8 + b'0') as char);
    }
    digits.into_iter().rev().collect()
}

fn word_is_zero(word: &[u8; 32]) -> bool {
    word.iter().all(|byte| *byte == 0)
}

fn checked_slippage_bps(value: f64) -> Result<String, String> {
    if !value.is_finite() || !(0.0..=10_000.0).contains(&value) {
        return Err("slippage must be between 0 and 10000 basis points".to_owned());
    }
    Ok(value.floor().to_string())
}

fn positive_quantity(label: &str, value: &str) -> Result<(), String> {
    let word = decimal_word(value).map_err(|_| format!("{label} 不是正十进制整数"))?;
    if word_is_zero(&word) {
        return Err(format!("{label} 不是正十进制整数"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_uint256_conversion_round_trips_max_value() {
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        let word = decimal_word(max).expect("uint256 max");
        assert_eq!(word, [u8::MAX; 32]);
        assert_eq!(word_decimal(word), max);
        assert!(decimal_word(&format!("{max}0")).is_err());
    }

    #[test]
    fn erc20_calls_encode_exact_owner_spender_and_amount() {
        let owner = "0x1111111111111111111111111111111111111111";
        let spender = "0x2222222222222222222222222222222222222222";
        let amount = decimal_word("1000000").expect("amount");
        let allowance = encode_allowance(owner, spender).expect("allowance");
        assert!(allowance.starts_with("0xdd62ed3e"));
        let approve = encode_approve(spender, amount).expect("approve");
        assert!(approve.starts_with("0x095ea7b3"));
        assert!(validate_approve_calldata(&approve, spender, amount).is_ok());
        assert!(validate_approve_calldata(&approve, owner, amount).is_err());
    }

    #[test]
    fn odd_hex_quantities_are_left_padded() {
        let word = hex_word("0xf").expect("hex");
        assert_eq!(word[31], 15);
        assert_eq!(word_decimal(word), "15");
    }
}
