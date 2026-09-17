use super::*;

pub(super) async fn read(
    client: &reqwest::Client,
    endpoint: &str,
    key: Option<&str>,
    rpc: &reqwest::Client,
    rpc_url: &str,
    main: &StockChainCost,
    native: &str,
    raw: u64,
) -> Result<StockNativeValuation, String> {
    jupiter_quota::wait_for_general_request(key.is_some()).await;
    let requested = common::time::now_ms();
    let amount = raw.to_string();
    let mut get = client
        .get(endpoint)
        .timeout(Duration::from_secs(4))
        .query(&[
            ("inputMint", comparison::SOLANA_USDC),
            ("outputMint", STOCK_WRAPPED_SOL),
            ("amount", amount.as_str()),
            ("taker", main.wallet_address.as_str()),
        ]);
    if let Some(key) = key {
        get = get.header("x-api-key", key);
    }
    let response = get.send().await.map_err(|_| "SOL 补仓交易构建失败")?;
    let body = quote::decode_jupiter_general_response(response, key.is_some(), "Jupiter").await?;
    let value: Value = serde_json::from_str(&body).map_err(|_| "SOL 补仓交易响应异常")?;
    if value.get("error").is_some_and(|v| !v.is_null()) {
        return Err("Jupiter 未能构建 SOL 补仓交易".into());
    }
    let quote = stock_quotes::parse_order_quote(
        &body,
        comparison::SOLANA_USDC,
        STOCK_WRAPPED_SOL,
        &amount,
        Some(&main.wallet_address),
        requested,
        common::time::now_ms(),
    )?;
    let encoded = value["transaction"]
        .as_str()
        .ok_or("SOL 补仓只返回报价，没有交易")?;
    let tx = transaction::inspect(encoded, &main.wallet_address)?;
    let payer = value["signatureFeePayer"]
        .as_str()
        .ok_or("SOL 补仓网络费付款方未返回")?;
    if payer != tx.fee_payer {
        return Err("SOL 补仓网络费付款方与交易不符".into());
    }
    let request_id = value["requestId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or("SOL 补仓缺少原始请求编号")?;
    let mode = value["mode"]
        .as_str()
        .filter(|s| ["ultra", "manual"].contains(s))
        .ok_or("SOL 补仓交易模式未知")?;
    let height = match value.get("lastValidBlockHeight").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(
            v.as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| v.as_u64())
                .filter(|n| *n > 0)
                .ok_or("SOL 补仓区块有效期无效")?,
        ),
    };
    let mut cost = main.clone();
    cost.quote = quote.clone();
    let sample = simulate_rpc(rpc, rpc_url, encoded, &tx, &cost).await?;
    let change =
        simulation::native_change(&sample.result, &tx, &cost, sample.fee, sample.minimum_slot)?;
    let debit = u64::try_from((-change).max(0)).map_err(|_| "SOL 补仓净扣溢出")?;
    let (outflow, required) = sample.funding(&tx, debit)?;
    let output = quote
        .output_raw
        .parse::<u64>()
        .map_err(|_| "SOL 补仓到账无效")?;
    let minimum = quote
        .minimum_output_raw
        .parse::<u64>()
        .map_err(|_| "SOL 补仓最低到账无效")?;
    // Ignore rent refunds and require the quoted native output after every traced wallet outflow.
    if change < i128::from(output) - i128::from(outflow) {
        return Err("补仓模拟原生 SOL 增量不足，不能用 WSOL 报价代替 Gas 到账".into());
    }
    let checked = common::time::now_ms();
    let valid_until = requested
        .saturating_add(comparison::STOCK_QUOTE_MAX_AGE_MS)
        .min(quote.expires_at_ms.unwrap_or(i64::MAX))
        .min(main.valid_until_ms);
    if checked >= valid_until {
        return Err("SOL 补仓交易模拟完成时已过期".into());
    }
    let transaction = shared_types::OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64: encoded.into(),
        request_id: request_id.into(),
        router: quote.router.clone(),
        mode: mode.into(),
        last_valid_block_height: height,
        expire_at_ms: quote.expires_at_ms,
    };
    Ok(StockNativeValuation {
        native_lamports: native.into(),
        quote,
        replenishment: Some(StockNativeReplenishment {
            wallet_address: main.wallet_address.clone(),
            transaction,
            transaction_fingerprint: tx.fingerprint,
            network_fee_lamports: sample.fee.to_string(),
            wallet_outflow_lamports: outflow.to_string(),
            wallet_required_lamports: required.to_string(),
            minimum_credit_lamports: minimum.saturating_sub(outflow).to_string(),
            simulation_slot: simulation::slot(&sample.result, sample.minimum_slot)?,
            checked_at_ms: checked,
            valid_until_ms: valid_until,
        }),
    })
}
