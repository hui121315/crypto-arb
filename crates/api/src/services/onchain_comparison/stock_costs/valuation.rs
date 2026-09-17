use super::*;

const CALIBRATION_USDC_RAW: u64 = 1_000_000;
const MAX_BUDGET_RAW: u64 = 100_000_000_000;

#[cfg(test)]
pub(super) async fn read_at(
    client: &reqwest::Client,
    endpoint: &str,
    key: Option<&str>,
    native: &str,
) -> Result<StockNativeValuation, String> {
    let target = native
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("SOL 费用数量无效")?;
    let calibration = read_quote(client, endpoint, key, CALIBRATION_USDC_RAW).await?;
    let mut amount = next_amount(CALIBRATION_USDC_RAW, target, &calibration)?;
    for attempt in 0..3 {
        // Scaling only proposes a request; only that request's actual minimum output can pass.
        let quote = if attempt == 0 && amount == CALIBRATION_USDC_RAW {
            calibration.clone()
        } else {
            read_quote(client, endpoint, key, amount).await?
        };
        if !comparison::quote_current(&quote, common::time::now_ms()) {
            return Err("SOL 补回报价已过期，成本保持未知".into());
        }
        let evidence = StockNativeValuation {
            native_lamports: native.into(),
            quote,
            replenishment: None,
        };
        if evidence
            .usdc_budget(native, common::time::now_ms())
            .is_some()
        {
            return Ok(evidence);
        }
        if attempt < 2 {
            amount = next_amount(amount, target, &evidence.quote)?;
        }
    }
    Err("三次 SOL 补回报价仍未覆盖费用数量，成本保持未知".into())
}

pub(super) async fn read_complete_at(
    client: &reqwest::Client,
    endpoint: &str,
    key: Option<&str>,
    rpc: &reqwest::Client,
    rpc_url: &str,
    main: &StockChainCost,
    native: &str,
) -> Result<StockNativeValuation, String> {
    let target = native.parse::<u64>().map_err(|_| "SOL 补仓目标无效")?;
    let calibration = read_quote(client, endpoint, key, CALIBRATION_USDC_RAW).await?;
    let mut raw = next_amount(CALIBRATION_USDC_RAW, target, &calibration)?;
    for attempt in 0..3 {
        let evidence =
            replenishment::read(client, endpoint, key, rpc, rpc_url, main, native, raw).await?;
        if evidence
            .complete_budget(native, &main.wallet_address, common::time::now_ms())
            .is_some()
        {
            return Ok(evidence);
        }
        if attempt < 2 {
            let outflow = evidence
                .replenishment
                .as_ref()
                .ok_or("补仓模拟证据缺失")?
                .wallet_outflow_lamports
                .parse::<u64>()
                .map_err(|_| "补仓支出无效")?;
            let total = target.checked_add(outflow).ok_or("SOL 补仓数量溢出")?;
            raw = next_amount(raw, total, &evidence.quote)?;
        }
    }
    Err("三次补仓模拟仍不能在扣费后补足原生 SOL，完整成本保持未知".into())
}

fn next_amount(input: u64, target: u64, quote: &StockDexQuote) -> Result<u64, String> {
    let minimum = quote
        .minimum_output_raw
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("SOL 最低到账无效")?;
    let numerator = u128::from(input) * u128::from(target);
    let proposed = numerator.div_ceil(u128::from(minimum));
    u64::try_from(proposed)
        .ok()
        .filter(|n| *n > 0 && *n <= MAX_BUDGET_RAW)
        .ok_or("SOL 补回报价超出 100000 USDC 只读试算范围".into())
}

async fn read_quote(
    client: &reqwest::Client,
    endpoint: &str,
    key: Option<&str>,
    raw: u64,
) -> Result<StockDexQuote, String> {
    let requested = common::time::now_ms();
    let body = quote::fetch_jupiter_quote_body_at(
        client,
        endpoint,
        key,
        comparison::SOLANA_USDC,
        STOCK_WRAPPED_SOL,
        &raw.to_string(),
    )
    .await
    .map_err(|_| "SOL 补回报价未取得，未将其当作零费用")?;
    let value: Value = serde_json::from_str(&body).map_err(|_| "SOL 补回报价响应异常")?;
    if value.get("error").is_some_and(|e| !e.is_null()) {
        return Err("SOL 补回报价返回错误".into());
    }
    stock_quotes::parse_order_quote(
        &body,
        comparison::SOLANA_USDC,
        STOCK_WRAPPED_SOL,
        &raw.to_string(),
        None,
        requested,
        common::time::now_ms(),
    )
}

#[cfg(test)]
mod tests;

mod replenishment;
