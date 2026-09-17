use super::*;
use crate::services::onchain_comparison::{replenishment_credit, token_identity_service};
use crate::services::onchain_cross_chain_run_store::recovery::{basis, scope};
use shared_types::{
    OnchainChainSettlementStatus as ReceiptStatus, OnchainCrossChainRecovery,
    OnchainTokenIdentityRequest,
};

pub(super) fn report(evidence: &LifiTransferEvidence) -> OnchainCrossChainRecovery {
    OnchainCrossChainRecovery {
        provider_status: evidence.provider_status.clone(),
        substatus: evidence.substatus.clone(),
        message: evidence.message.clone(),
        provider_transaction_id: evidence.transaction_id.clone(),
        sending_transaction_id: evidence.sending_tx_hash.clone(),
        sending_chain_id: evidence.sending_chain_id,
        receiving_transaction_id: evidence.receiving_tx_hash.clone(),
        receiving_chain_id: evidence.receiving_chain_id,
        receiving_token_chain_id: evidence.receiving_token_chain_id,
        receiving_token: evidence.receiving_token.clone(),
        reported_amount_raw: evidence.receiving_amount_raw.clone(),
        reported_receiver: evidence.to_address.clone(),
        observed_at_ms: evidence.observed_at_ms,
        official_docs_url: evidence.official_docs_url.clone(),
        token_resolution: None,
        receipt: None,
        problem: None,
    }
}

pub(super) async fn reconcile(
    state: &AppState,
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    mut report: OnchainCrossChainRecovery,
    now_ms: i64,
) -> Result<(), String> {
    if report
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.status != ReceiptStatus::Pending)
    {
        return save(state, run, leg.position, report, now_ms).await;
    }
    let scope = match scope(run, leg, &report) {
        Ok(scope) => scope,
        Err(problem) => {
            report.problem = Some(problem);
            return save(state, run, leg.position, report, now_ms).await;
        }
    };
    // Freeze the exceptional outcome before another remote read or a possible restart.
    report.problem = None;
    state.onchain_cross_chain_runs().record_bridge_recovery(
        &run.run_id,
        leg.position,
        report.clone(),
        now_ms,
    )?;
    if report.token_resolution.is_none() {
        match token_identity_service::resolve(
            state,
            OnchainTokenIdentityRequest {
                chain: scope.0,
                address: scope.2,
                custom_rpc_url: None,
            },
        )
        .await
        {
            Ok(resolution) => report.token_resolution = Some(resolution),
            Err(problem) => {
                report.problem = Some(format!("异常到账合约精度读取失败：{problem}"));
                return save(state, run, leg.position, report, common::time::now_ms()).await;
            }
        }
    }
    let request = match basis(run, leg, &report) {
        Ok(request) => request,
        Err(problem) => {
            report.problem = Some(problem);
            return save(state, run, leg.position, report, common::time::now_ms()).await;
        }
    };
    let receipt = match report
        .receipt
        .as_ref()
        .filter(|receipt| receipt.basis == request && receipt.status == ReceiptStatus::Complete)
    {
        Some(receipt) => receipt.clone(),
        None => replenishment_credit::wallet::check(state, &request).await,
    };
    let credit = receipt
        .asset_changes_raw
        .first()
        .and_then(Option::as_deref)
        .and_then(|raw| raw.parse::<i128>().ok());
    report.problem = receipt.problem.clone().or_else(|| {
        if credit.is_none_or(|amount| amount <= 0) {
            Some("原钱包尚未核实正数异常到账；不把桥报告金额计入余额".into())
        } else if report
            .reported_amount_raw
            .as_deref()
            .and_then(|raw| raw.parse::<i128>().ok())
            != credit
        {
            Some("桥报告金额与钱包净到账不同，已保留真实数量，需核对扣费或批量转账".into())
        } else {
            None
        }
    });
    report.receipt = Some(receipt);
    save(state, run, leg.position, report, common::time::now_ms()).await
}

async fn save(
    state: &AppState,
    run: &OnchainCrossChainRun,
    position: u8,
    report: OnchainCrossChainRecovery,
    now_ms: i64,
) -> Result<(), String> {
    let updated = state.onchain_cross_chain_runs().record_bridge_recovery(
        &run.run_id,
        position,
        report,
        now_ms,
    )?;
    if updated.status != run.status {
        emit_run_webhook(state, &updated).await;
    }
    Ok(())
}
