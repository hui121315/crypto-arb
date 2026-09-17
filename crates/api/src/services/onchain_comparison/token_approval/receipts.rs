use super::super::{replenishment_credit, rpc, rpc_target};
use crate::{
    services::onchain_token_approval_run_store::{self as store, ApprovalRecord},
    state::AppState,
};
use serde_json::{json, Value};
use shared_types::{OnchainChainSettlementStatus as Status, OnchainWalletReceipt};

const APPROVAL_TOPIC: &str = "0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925";

pub(crate) async fn refresh(state: &AppState, now: i64) {
    if state.onchain_token_approval_runs().readiness().is_err() {
        return;
    }
    for (row, hash) in state.onchain_token_approval_runs().due(now) {
        let receipt = read(state, &row, &hash).await;
        if let Ok(receipt) = receipt {
            if let Err(problem) = state.onchain_token_approval_runs().receipt(
                &row.response.run_id,
                receipt,
                common::time::now_ms(),
            ) {
                tracing::warn!(%problem, "approval receipt was not durable");
            }
        }
    }
}

pub(super) async fn confirmed(state: &AppState, run_id: &str, hash: &str) -> Result<(), String> {
    let row = state
        .onchain_token_approval_runs()
        .record(run_id)
        .ok_or("授权记录未落盘")?;
    let receipt = read(state, &row, hash).await?;
    let complete = receipt.status == Status::Complete;
    let problem = receipt.problem.clone();
    state
        .onchain_token_approval_runs()
        .receipt(run_id, receipt, common::time::now_ms())?;
    if !complete {
        return Err(problem.unwrap_or_else(|| "授权事件或费用回执尚未核清".into()));
    }
    state.onchain_token_approval_runs().confirm(run_id, hash)
}

async fn read(
    state: &AppState,
    row: &ApprovalRecord,
    hash: &str,
) -> Result<OnchainWalletReceipt, String> {
    let basis = store::basis(row, hash)?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(12), async {
        let (url, _) = replenishment_credit::rpc_endpoint(state, &row.plan.chain)
            .ok_or("授权费用核验 RPC 未配置")?;
        let (url, client) = rpc_target::rpc_target(&url).await?;
        read_on_rpc(&client, url.as_str(), row, hash).await
    })
    .await
    .unwrap_or_else(|_| Err("授权费用核验超时，不重复广播".into()));
    Ok(result
        .unwrap_or_else(|problem| replenishment_credit::wallet::pending_receipt(&basis, problem)))
}

async fn read_on_rpc(
    client: &reqwest::Client,
    url: &str,
    row: &ApprovalRecord,
    hash: &str,
) -> Result<OnchainWalletReceipt, String> {
    let basis = store::basis(row, hash)?;
    let mut receipt = replenishment_credit::wallet::read_on_rpc(client, url, &basis).await?;
    if receipt.status == Status::Complete {
        let tx =
            rpc::rpc_result(client, url, "eth_getTransactionReceipt", json!([hash]), 771).await;
        match tx {
            Ok(tx) => {
                if let Err(problem) = validate_event(row, hash, &receipt, &tx) {
                    receipt.status = Status::ReviewRequired;
                    receipt.problem = Some(problem);
                }
            }
            Err(problem) => {
                receipt.status = Status::Pending;
                receipt.problem = Some(problem);
            }
        }
    }
    Ok(receipt)
}

fn validate_event(
    row: &ApprovalRecord,
    hash: &str,
    receipt: &OnchainWalletReceipt,
    tx: &Value,
) -> Result<(), String> {
    let position = row
        .response
        .transaction_ids
        .iter()
        .position(|id| id == hash)
        .ok_or("授权交易未登记")?;
    let (owner, spender, amount) = store::approval_words(&row.plan, position)?;
    if tx["blockHash"].as_str() != receipt.block_ref.as_deref()
        || !same(&tx["transactionHash"], hash)
        || tx["status"] != "0x1"
    {
        return Err("授权事件与已核验区块不一致，需复核重组或交易结果".into());
    }
    // ERC-20 requires an Approval event on success; a successful EVM receipt alone is insufficient.
    let valid = tx["logs"].as_array().is_some_and(|logs| {
        logs.iter().any(|log| {
            same(&log["address"], &row.plan.token_address)
                && log["removed"] != true
                && log["topics"].as_array().is_some_and(|v| v.len() == 3)
                && same(&log["topics"][0], APPROVAL_TOPIC)
                && same(&log["topics"][1], &owner)
                && same(&log["topics"][2], &spender)
                && same(&log["data"], &amount)
        })
    });
    if !valid {
        return Err(
            "未找到与钱包、spender、授权金额一致的 Approval 事件；不能把交易打包当作授权成功"
                .into(),
        );
    }
    if receipt.asset_changes_raw != vec![Some("0".into())]
        || receipt.additional_native_change_raw.as_deref() != Some("0")
    {
        return Err("授权交易出现额外资产变化，已保留实际收支，需复核".into());
    }
    Ok(())
}

fn same(value: &Value, expected: &str) -> bool {
    value
        .as_str()
        .is_some_and(|v| v.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests;
