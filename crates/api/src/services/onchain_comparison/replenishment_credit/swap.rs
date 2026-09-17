use super::*;
use shared_types::{
    OnchainChainSettlement, OnchainChainSettlementBasis, OnchainChainSettlementStatus as Status,
};

pub(in crate::services::onchain_comparison) async fn check(
    state: &AppState,
    basis: &OnchainChainSettlementBasis,
) -> OnchainChainSettlement {
    let result = async {
        let (url, _) = rpc_endpoint(state, &basis.chain).ok_or("缺少已验证的交易核算 RPC")?;
        let (url, client) = rpc_target::rpc_target(&url).await?;
        read_on_rpc(&client, url.as_str(), basis).await
    }
    .await;
    result.unwrap_or_else(|problem| pending_receipt(basis, problem))
}

pub(in crate::services::onchain_comparison) fn pending_receipt(
    basis: &OnchainChainSettlementBasis,
    problem: String,
) -> OnchainChainSettlement {
    OnchainChainSettlement {
        basis: basis.clone(),
        status: Status::Pending,
        input_amount_raw: None,
        output_amount_raw: None,
        additional_native_change_raw: None,
        network_cost: None,
        block_ref: None,
        observed_at_ms: None,
        problem: Some(problem),
        attempts: 0,
    }
}

pub(in crate::services::onchain_comparison) async fn read_on_rpc(
    client: &reqwest::Client,
    url: &str,
    basis: &OnchainChainSettlementBasis,
) -> Result<OnchainChainSettlement, String> {
    let receipt = super::wallet::read_on_rpc(
        client,
        url,
        &shared_types::OnchainWalletReceiptBasis {
            chain: basis.chain.clone(),
            wallet: basis.wallet.clone(),
            transaction_id: basis.transaction_id.clone(),
            assets: vec![basis.assets.input.clone(), basis.assets.output.clone()],
            require_sender: true,
        },
    )
    .await?;
    let mut row = pending_receipt(basis, "链上实际收支待核算".into());
    row.status = receipt.status;
    row.network_cost = receipt.network_cost;
    row.additional_native_change_raw = receipt.additional_native_change_raw;
    row.block_ref = receipt.block_ref;
    row.observed_at_ms = receipt.observed_at_ms;
    row.problem = receipt.problem;
    let input = receipt.asset_changes_raw[0]
        .as_deref()
        .and_then(|v| v.parse::<i128>().ok());
    let output = receipt.asset_changes_raw[1]
        .as_deref()
        .and_then(|v| v.parse::<i128>().ok());
    if row.status == Status::ReviewRequired {
        row.input_amount_raw = input.map(|v| v.unsigned_abs().to_string());
        row.output_amount_raw = output.map(|v| v.to_string());
        return Ok(row);
    }
    if input.is_some_and(|v| v >= 0) || output.is_some_and(|v| v <= 0) {
        row.status = Status::ReviewRequired;
        row.problem = Some("实际资产变化与买卖方向不一致，不能用报价代替成交".into());
        return Ok(row);
    }
    row.input_amount_raw = input.map(|v| v.unsigned_abs().to_string());
    row.output_amount_raw = output.map(|v| v.to_string());
    if let (Some(paid), Some(received)) = (input, output) {
        let maximum = basis
            .maximum_input_raw
            .parse::<u128>()
            .map_err(|_| "计划输入数量非法")?;
        let minimum = basis
            .minimum_output_raw
            .as_deref()
            .and_then(|v| v.parse::<u128>().ok());
        if paid.unsigned_abs() > maximum
            || minimum.is_none_or(|minimum| (received as u128) < minimum)
        {
            row.status = Status::ReviewRequired;
            row.problem = Some("实际扣款超过输入上限，或实际到账未满足最少到账约束".into());
        }
    }
    Ok(row)
}

#[cfg(test)]
pub(in crate::services::onchain_comparison) mod tests;
