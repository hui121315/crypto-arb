use super::*;

pub(in crate::services::onchain_comparison) const DOCS: &str =
    "https://docs.li.fi/api-reference/get-a-quote-for-a-token-transfer";

#[derive(Debug, Clone)]
pub(in crate::services::onchain_comparison) struct RecoveryQuoteRequest {
    pub from_chain: String,
    pub to_chain: String,
    pub from_token: String,
    pub to_token: String,
    pub from_wallet: String,
    pub to_wallet: String,
    pub amount_raw: String,
    pub slippage_bps: f64,
}

#[derive(Debug, Clone)]
pub(in crate::services::onchain_comparison) struct RecoveryQuote {
    pub route_id: String,
    pub output_raw: String,
    pub minimum_raw: String,
    pub fee_usd: Option<f64>,
    pub gas_usd: Option<f64>,
    pub duration_seconds: Option<u64>,
    pub transaction: OnchainUnsignedTransaction,
    pub observed_at_ms: i64,
    pub valid_until_ms: i64,
}

pub(in crate::services::onchain_comparison) async fn fetch(
    request: &RecoveryQuoteRequest,
) -> Result<RecoveryQuote, String> {
    fetch_at(
        quote_client(),
        LIFI_QUOTE_ENDPOINT,
        env_key("LIFI_API_KEY").as_deref(),
        request,
    )
    .await
}

async fn fetch_at(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: Option<&str>,
    request: &RecoveryQuoteRequest,
) -> Result<RecoveryQuote, String> {
    let from = chain_id(&request.from_chain).ok_or("LI.FI 未登记处置来源链")?;
    let to = chain_id(&request.to_chain).ok_or("LI.FI 未登记处置目标链")?;
    if !request.slippage_bps.is_finite()
        || !(0.0..=10_000.0).contains(&request.slippage_bps)
        || request.from_wallet.trim().is_empty()
        || request.to_wallet.trim().is_empty()
        || positive_raw(&request.amount_raw, "fromAmount").is_err()
    {
        return Err("处置报价的钱包、数量或滑点无效".into());
    }
    let mut call = client.get(endpoint).query(&[
        ("fromChain", from.to_string()),
        ("toChain", to.to_string()),
        ("fromToken", lifi_token_address(&request.from_token)),
        ("toToken", lifi_token_address(&request.to_token)),
        ("fromAddress", request.from_wallet.clone()),
        ("toAddress", request.to_wallet.clone()),
        ("fromAmount", request.amount_raw.clone()),
        ("slippage", (request.slippage_bps / 10_000.0).to_string()),
        ("order", "CHEAPEST".into()),
    ]);
    if let Some(key) = api_key {
        call = call.header("x-lifi-api-key", key);
    }
    let response = call
        .send()
        .await
        .map_err(|_| "LI.FI 处置报价网络请求失败".to_owned())?;
    let body = decode_response(response, "LI.FI").await?;
    let response: LifiQuoteResponse =
        serde_json::from_str(&body).map_err(|_| "LI.FI 处置报价格式不完整".to_owned())?;
    parse(response, request, common::time::now_ms())
}

fn parse(
    response: LifiQuoteResponse,
    request: &RecoveryQuoteRequest,
    now_ms: i64,
) -> Result<RecoveryQuote, String> {
    let from = chain_id(&request.from_chain).ok_or("来源链未知")?;
    let to = chain_id(&request.to_chain).ok_or("目标链未知")?;
    let action = &response.action;
    if action.from_chain_id != from
        || action.from_token.chain_id != from
        || action.to_chain_id != to
        || action.to_token.chain_id != to
        || !token_matches(from, &action.from_token.address, &request.from_token)
        || !token_matches(to, &action.to_token.address, &request.to_token)
        || !address_matches(from, &action.from_address, &request.from_wallet)
        || !address_matches(to, &action.to_address, &request.to_wallet)
        || action.from_amount != request.amount_raw
        || response.estimate.from_amount != request.amount_raw
    {
        return Err("LI.FI 处置报价改变了链、钱包、合约或精确输入数量".into());
    }
    let output = positive_raw(&response.estimate.to_amount, "toAmount")?;
    let minimum = positive_raw(&response.estimate.to_amount_min, "toAmountMin")?;
    if minimum > output || response.id.trim().is_empty() || response.tool.trim().is_empty() {
        return Err("LI.FI 处置报价缺少路径编号或有效最低到账量".into());
    }
    let approval = response.estimate.approval_address.as_deref();
    if from != LIFI_SOLANA_CHAIN_ID && !native_evm_token(&request.from_token) {
        validate_evm_address(
            approval.ok_or("LI.FI 未提供代币授权地址")?,
            "approvalAddress",
        )?;
    }
    // The quote id identifies this read-only proposal, never a bridge transfer or mined transaction.
    let transaction = parse_transaction(
        &response.transaction_request,
        &response.id,
        &response.tool,
        from,
        &request.from_wallet,
        &request.from_token,
        approval,
    )?;
    Ok(RecoveryQuote {
        route_id: response.id,
        output_raw: response.estimate.to_amount,
        minimum_raw: response.estimate.to_amount_min,
        fee_usd: optional_usd(response.estimate.fee_costs.as_deref())?,
        gas_usd: optional_usd(
            response
                .estimate
                .gas_costs
                .as_deref()
                .filter(|costs| !costs.is_empty()),
        )?,
        duration_seconds: response.estimate.execution_duration,
        transaction,
        observed_at_ms: now_ms,
        valid_until_ms: now_ms.saturating_add(LIFI_QUOTE_VALIDITY_MS),
    })
}

#[cfg(test)]
mod tests;
