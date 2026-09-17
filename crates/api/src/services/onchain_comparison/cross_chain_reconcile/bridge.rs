use super::super::{lifi, replenishment_credit};
use super::*;

pub(super) async fn confirm_destination(
    state: &AppState,
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    evidence: &LifiTransferEvidence,
    now_ms: i64,
) -> Result<(), String> {
    let basis = match credit_request(run, leg, evidence) {
        Ok(Some(basis)) => basis,
        Ok(None) => {
            return record_pending(
                state,
                run,
                "桥服务尚未提供完整的源交易与目标链身份",
                evidence,
                now_ms,
            )
        }
        Err(problem) => return pause(state, run, problem, now_ms).await,
    };
    let receipt = match leg.destination_receipt.as_ref().filter(|r| {
        r.basis == basis && r.status == shared_types::OnchainChainSettlementStatus::Complete
    }) {
        Some(receipt) => receipt.clone(),
        None => replenishment_credit::wallet::check(state, &basis).await,
    };
    // Preserve all receipt evidence even when the provider reports a different amount.
    let updated = state.onchain_cross_chain_runs().record_wallet_receipt(
        &run.run_id,
        leg.position,
        receipt,
        true,
        evidence.receiving_amount_raw.as_deref(),
        common::time::now_ms(),
    )?;
    if updated.status != run.status {
        emit_run_webhook(state, &updated).await;
    }
    Ok(())
}

pub(super) fn credit_request(
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    evidence: &LifiTransferEvidence,
) -> Result<Option<shared_types::OnchainWalletReceiptBasis>, String> {
    let execution = leg.bridge_execution.as_ref().ok_or("缺少已提交桥合同")?;
    let route = run
        .build
        .legs
        .iter()
        .find(|route| route.position == leg.position)
        .ok_or("缺少桥腿链身份")?;
    if lifi::chain_id(&route.to_chain) != Some(execution.to_chain_id)
        || lifi::chain_id(&route.from_chain) != Some(execution.from_chain_id)
    {
        return Err("桥合同与已授权的链路径不一致".into());
    }
    let Some(sending) = evidence.sending_tx_hash.as_deref() else {
        return Ok(None);
    };
    if !leg
        .source_transaction_id
        .as_deref()
        .is_some_and(|hash| lifi::address_matches(execution.from_chain_id, hash, sending))
        || evidence
            .transaction_id
            .as_deref()
            .is_some_and(|id| id != execution.transaction_id)
        || evidence
            .sending_chain_id
            .is_some_and(|chain| chain != execution.from_chain_id)
        || evidence
            .receiving_chain_id
            .is_some_and(|chain| chain != execution.to_chain_id)
        || evidence
            .receiving_token_chain_id
            .is_some_and(|chain| chain != execution.to_chain_id)
        || evidence.to_address.as_deref().is_some_and(|wallet| {
            !lifi::address_matches(execution.to_chain_id, &execution.to_address, wallet)
        })
        || evidence.receiving_token.as_deref().is_some_and(|token| {
            !lifi::token_matches(execution.to_chain_id, &execution.to_token, token)
        })
    {
        return Err("LI.FI 终态与已提交的交易、链、钱包或代币身份不一致".into());
    }
    if evidence.receiving_chain_id.is_none() || evidence.receiving_tx_hash.is_none() {
        return Ok(None);
    }
    crate::services::onchain_cross_chain_run_store::receipts::basis(
        run,
        leg,
        evidence.receiving_tx_hash.as_deref(),
    )
    .map(Some)
}
