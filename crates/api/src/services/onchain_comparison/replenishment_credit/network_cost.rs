use super::*;
use shared_types::OnchainReplenishmentNetworkCost;

const OP_ORACLE: &str = "0x420000000000000000000000000000000000000F";
const OP_DOCS: &str = "https://specs.optimism.io/protocol/isthmus/predeploys.html";

pub(super) fn solana(
    transaction: &Value,
    transaction_id: &str,
) -> Option<OnchainReplenishmentNetworkCost> {
    let first = transaction.pointer("/transaction/message/accountKeys/0")?;
    let payer = first.as_str().or_else(|| first.get("pubkey")?.as_str())?;
    let mut cost = cost(
        "solana",
        transaction_id,
        &transaction["slot"].as_u64()?.to_string(),
        payer,
    )?;
    let fee = transaction
        .pointer("/meta/fee")
        .and_then(Value::as_u64)
        .map(u128::from);
    set_fees(&mut cost, fee, Ok(0));
    cost.source = SOLANA_TRANSACTION_DOCS.into();
    Some(cost)
}

pub(super) async fn evm(
    client: &reqwest::Client,
    rpc_url: &str,
    chain: &str,
    receipt: &Value,
) -> Option<OnchainReplenishmentNetworkCost> {
    let mut cost = cost(
        chain,
        receipt["transactionHash"].as_str()?,
        receipt["blockHash"].as_str()?,
        receipt["from"].as_str()?,
    )?;
    let gas = quantity(receipt.get("gasUsed"));
    let execution = gas
        .zip(quantity(receipt.get("effectiveGasPrice")))
        .and_then(|(gas, price)| gas.checked_mul(price));
    // Arbitrum already includes its poster fee in gasUsed. OP Stack has
    // separate L1/operator charges; blob fees are separate on Ethereum.
    let extra = if matches!(chain, "base" | "optimism") {
        cost.source = format!("{ETHEREUM_RPC_DOCS} · {OP_DOCS}");
        op_extra(client, rpc_url, receipt, gas).await
    } else if receipt.get("blobGasUsed").is_some()
        || receipt.get("blobGasPrice").is_some()
        || receipt.get("type").and_then(Value::as_str) == Some("0x3")
    {
        quantity(receipt.get("blobGasUsed"))
            .zip(quantity(receipt.get("blobGasPrice")))
            .and_then(|(gas, price)| gas.checked_mul(price))
            .ok_or("Blob 网络费缺失或溢出".into())
    } else {
        Ok(0)
    };
    set_fees(&mut cost, execution, extra);
    Some(cost)
}

async fn op_extra(
    client: &reqwest::Client,
    url: &str,
    receipt: &Value,
    gas: Option<u128>,
) -> Result<u128, String> {
    let l1 = quantity(receipt.get("l1Fee")).ok_or("L1 数据费缺失，不能把执行费当作完整网络费")?;
    // OP receipts omit both operator fields only when the operator fee is zero.
    // Query the historical oracle when present: Isthmus and Jovian use different
    // formulas, and today's parameters must not price an older transaction.
    let operator =
        if receipt.get("operatorFeeScalar").is_none()
            && receipt.get("operatorFeeConstant").is_none()
        {
            0
        } else {
            let gas = gas.ok_or("运营费核验缺少 gasUsed")?;
            let selector = common::signing::keccak256(b"getOperatorFee(uint256)");
            let output = rpc::rpc_result(client, url, "eth_call", serde_json::json!([
            {"to":OP_ORACLE,"data":format!("0x{}{:064x}", hex::encode(&selector[..4]), gas)},
            {"blockHash":receipt["blockHash"],"requireCanonical":true}
        ]), 706).await.map_err(|_| "原交易区块的运营费暂不可读，网络总费待核验".to_owned())?;
            let word = output
                .as_str()
                .and_then(|value| value.strip_prefix("0x"))
                .filter(|value| {
                    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or("运营费返回值不是完整 ABI 数值")?;
            u128::from_str_radix(word, 16).map_err(|_| "运营费超出支持范围")?
        };
    l1.checked_add(operator)
        .ok_or_else(|| "L1 数据费与运营费合计溢出".into())
}

fn cost(
    chain: &str,
    transaction_id: &str,
    block_ref: &str,
    payer: &str,
) -> Option<OnchainReplenishmentNetworkCost> {
    let preset = shared_types::onchain_chain_preset(chain)?;
    if payer.trim().is_empty() || block_ref.is_empty() || transaction_id.is_empty() {
        return None;
    }
    Some(OnchainReplenishmentNetworkCost {
        chain: preset.id.into(),
        transaction_id: transaction_id.into(),
        block_ref: block_ref.into(),
        payer: payer.into(),
        asset: preset.base_token.into(),
        execution_fee_exact: None,
        additional_fee_exact: None,
        total_fee_exact: None,
        source: ETHEREUM_RPC_DOCS.into(),
        observed_at_ms: common::time::now_ms(),
        problem: None,
        usd_valuation: None,
    })
}

fn set_fees(
    cost: &mut OnchainReplenishmentNetworkCost,
    execution: Option<u128>,
    extra: Result<u128, String>,
) {
    let decimals = if cost.chain == "solana" { 9 } else { 18 };
    cost.execution_fee_exact = execution.and_then(|value| raw_to_decimal(value, decimals));
    match extra {
        Ok(extra) => {
            cost.additional_fee_exact = raw_to_decimal(extra, decimals);
            cost.total_fee_exact = execution
                .and_then(|value| value.checked_add(extra))
                .and_then(|value| raw_to_decimal(value, decimals));
            if cost.total_fee_exact.is_none() {
                cost.problem = Some("执行费缺失或费用合计溢出，实扣总费待核验".into());
            }
        }
        Err(problem) => cost.problem = Some(problem),
    }
}

fn raw_to_decimal(value: u128, decimals: u32) -> Option<String> {
    rust_decimal::Decimal::try_from_i128_with_scale(value.try_into().ok()?, decimals)
        .ok()
        .map(|value| value.normalize().to_string())
}

fn quantity(value: Option<&Value>) -> Option<u128> {
    let raw = value?.as_str()?.strip_prefix("0x")?;
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    u128::from_str_radix(raw, 16).ok()
}
