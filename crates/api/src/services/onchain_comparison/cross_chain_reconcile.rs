use crate::state::AppState;
use shared_types::{
    OnchainCrossChainLegKind, OnchainCrossChainLegProgress, OnchainCrossChainLegRunStatus,
    OnchainCrossChainRun, OnchainCrossChainRunStatus, WebhookEventKind,
};

use super::lifi::{LifiTransferEvidence, LifiTransferState, LIFI_STATUS_DOCS};

#[path = "cross_chain_reconcile/bridge.rs"]
mod bridge;
#[path = "cross_chain_reconcile/recovery.rs"]
mod recovery;

#[cfg(test)]
#[path = "cross_chain_reconcile/tests.rs"]
mod tests;

const NOT_FOUND_GRACE_MS: i64 = 5 * 60_000;

pub(crate) fn request_recheck(
    state: &AppState,
    request: &shared_types::OnchainCrossChainRecheckRequest,
    actor: &str,
) -> Result<OnchainCrossChainRun, common::AppError> {
    use crate::services::onchain_cross_chain_run_store::CrossChainRecheckError;
    use axum::http::StatusCode;
    state
        .onchain_cross_chain_runs()
        .request_recheck(
            request.run_id.trim(),
            actor,
            request.expected_position,
            common::time::now_ms(),
        )
        .map_err(|error| {
            let (status, code, message) = match error {
                CrossChainRecheckError::Missing => (
                    StatusCode::NOT_FOUND,
                    "ONCHAIN_CROSS_CHAIN_RUN_MISSING",
                    "跨链运行记录不存在".to_owned(),
                ),
                CrossChainRecheckError::ActorMismatch => (
                    StatusCode::FORBIDDEN,
                    "ONCHAIN_CROSS_CHAIN_ACTOR_MISMATCH",
                    "仅原授权身份可以恢复到账核验".to_owned(),
                ),
                CrossChainRecheckError::InvalidState(problem) => (
                    StatusCode::CONFLICT,
                    "ONCHAIN_CROSS_CHAIN_RECHECK_NOT_READY",
                    problem,
                ),
                CrossChainRecheckError::Persistence(problem) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "ONCHAIN_CROSS_CHAIN_RECHECK_NOT_DURABLE",
                    format!("恢复请求无法写入账本：{problem}"),
                ),
            };
            common::AppError::domain(status, code, message)
        })
}

pub(crate) async fn reconcile_pending(state: &AppState, now_ms: i64) {
    let mut rows = state.onchain_cross_chain_runs().runs(128, now_ms).rows;
    rows.sort_by_key(|run| {
        active_leg(run)
            .and_then(|leg| leg.last_checked_at_ms)
            .unwrap_or(0)
    });
    let mut checked = 0;
    for run in rows {
        if !run.reconciliation_due(now_ms) {
            continue;
        }
        if checked >= 2 {
            break;
        }
        checked += 1;
        if let Err(problem) = reconcile_once(state, &run.run_id, now_ms).await {
            tracing::warn!(run_id = %run.run_id, %problem, "cross-chain reconciliation failed");
        }
    }
}

async fn reconcile_once(state: &AppState, run_id: &str, now_ms: i64) -> Result<(), String> {
    let Some(run) = state.onchain_cross_chain_runs().claim_reconciliation(run_id, now_ms)? else {
        return Ok(());
    };
    if run.status == OnchainCrossChainRunStatus::Paused {
        emit_run_webhook(state, &run).await;
        return Ok(());
    }
    let result = reconcile_run(state, &run, now_ms).await;
    let updated = state.onchain_cross_chain_runs()
        .finish_reconciliation(run_id, common::time::now_ms().max(now_ms))?;
    if updated.status == OnchainCrossChainRunStatus::Paused {
        emit_run_webhook(state, &updated).await;
    }
    result
}

async fn reconcile_run(
    state: &AppState,
    run: &OnchainCrossChainRun,
    now_ms: i64,
) -> Result<(), String> {
    let leg = active_leg(run).ok_or_else(|| "cross-chain active leg is missing".to_owned())?;
    match leg.kind {
        OnchainCrossChainLegKind::SourceSwap | OnchainCrossChainLegKind::TargetSwap => {
            reconcile_swap(state, run, leg, now_ms).await
        }
        OnchainCrossChainLegKind::OutboundBridge | OnchainCrossChainLegKind::ReturnBridge => {
            reconcile_bridge(state, run, leg, now_ms).await
        }
    }
}

async fn reconcile_bridge(
    state: &AppState,
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    now_ms: i64,
) -> Result<(), String> {
    if let Some(report) = leg.bridge_recovery.as_ref().filter(|report|
        crate::services::onchain_cross_chain_run_store::recovery::basis(run, leg, report).is_ok()) {
        return recovery::reconcile(state, run, leg, report.clone(), now_ms).await;
    }
    let updated;
    let run = if leg
        .source_receipt
        .as_ref()
        .is_none_or(|v| v.status != shared_types::OnchainChainSettlementStatus::Complete)
    {
        updated = read_source_receipt(state, run, leg, now_ms).await?;
        if !matches!(
            updated.status,
            OnchainCrossChainRunStatus::AwaitingDestinationEvidence
                | OnchainCrossChainRunStatus::AwaitingSourceFinality
        ) || active_leg(&updated)
            .and_then(|v| v.source_receipt.as_ref())
            .is_none_or(|v| {
                v.block_ref.is_none()
                    || v.asset_changes_raw
                        .first()
                        .and_then(|v| v.as_deref())
                        .and_then(|v| v.parse::<i128>().ok())
                        .is_none_or(|v| v >= 0)
            })
        {
            return Ok(());
        }
        &updated
    } else {
        run
    };
    let leg = active_leg(run).ok_or("桥步骤已变化")?;
    let execution = leg
        .bridge_execution
        .as_ref()
        .ok_or_else(|| "active cross-chain leg has no durable bridge execution".to_owned())?;
    let source_transaction_id = leg
        .source_transaction_id
        .as_deref()
        .ok_or_else(|| "active bridge leg has no source transaction id".to_owned())?;
    let evidence = match super::lifi::fetch_status(
        source_transaction_id,
        execution.from_chain_id,
        execution.to_chain_id,
        &execution.tool,
    )
    .await
    {
        Ok(evidence) => evidence,
        Err(problem) => {
            state.onchain_cross_chain_runs().record_check_problem(
                &run.run_id,
                format!("LI.FI 跨链终态查询失败：{problem}"),
                LIFI_STATUS_DOCS.to_owned(),
                now_ms,
            )?;
            return Ok(());
        }
    };
    if evidence
        .transaction_id
        .as_deref()
        .is_some_and(|observed| observed != execution.transaction_id)
    {
        return pause(
            state,
            run,
            "LI.FI 返回的 transactionId 与已提交桥合同不一致".to_owned(),
            now_ms,
        )
        .await;
    }
    apply_evidence(state, run, leg, evidence, now_ms).await
}

async fn reconcile_swap(
    state: &AppState,
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    now_ms: i64,
) -> Result<(), String> {
    read_source_receipt(state, run, leg, now_ms).await?;
    Ok(())
}

async fn read_source_receipt(
    state: &AppState,
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    _now_ms: i64,
) -> Result<OnchainCrossChainRun, String> {
    let basis = crate::services::onchain_cross_chain_run_store::receipts::basis(run, leg, None)?;
    let receipt = match leg.source_receipt.as_ref().filter(|r| {
        r.basis == basis && r.status == shared_types::OnchainChainSettlementStatus::Complete
    }) {
        Some(receipt) => receipt.clone(),
        None => super::replenishment_credit::wallet::check(state, &basis).await,
    };
    let updated = state.onchain_cross_chain_runs().record_wallet_receipt(
        &run.run_id,
        leg.position,
        receipt,
        false,
        None,
        common::time::now_ms(),
    )?;
    if updated.status != run.status {
        emit_run_webhook(state, &updated).await;
    }
    Ok(updated)
}

async fn apply_evidence(
    state: &AppState,
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    evidence: LifiTransferEvidence,
    now_ms: i64,
) -> Result<(), String> {
    if leg.bridge_recovery.is_some() && !matches!(evidence.state,
        LifiTransferState::Partial | LifiTransferState::Refunded | LifiTransferState::Failed) {
        return record_pending(state, run, "已记录桥异常终态，后续状态不能恢复原套利路径", &evidence, now_ms);
    }
    match evidence.state {
        LifiTransferState::NotFound => {
            let submitted_at_ms = leg.source_submitted_at_ms.unwrap_or(run.updated_at_ms);
            if leg.recovery_started_at_ms.is_none()
                && now_ms.saturating_sub(submitted_at_ms) >= NOT_FOUND_GRACE_MS {
                pause(
                    state,
                    run,
                    "LI.FI 在 5 分钟宽限期后仍找不到源交易；已停止自动推进，避免重复桥接"
                        .to_owned(),
                    now_ms,
                )
                .await
            } else {
                record_pending(state, run, "LI.FI 尚未索引源交易", &evidence, now_ms)
            }
        }
        LifiTransferState::Pending => {
            record_pending(state, run, "LI.FI 跨链仍在处理中", &evidence, now_ms)
        }
        LifiTransferState::Completed => {
            bridge::confirm_destination(state, run, leg, &evidence, now_ms).await
        }
        LifiTransferState::CompletedEvidenceMissing => {
            let run = ensure_source_confirmed(state, run, leg, LIFI_STATUS_DOCS, now_ms)?;
            state.onchain_cross_chain_runs().record_check_problem(
                &run.run_id,
                "LI.FI 报告桥已完成，但目标链交易哈希或正数到账量尚未形成证据".to_owned(),
                evidence.official_docs_url,
                now_ms,
            )?;
            Ok(())
        }
        LifiTransferState::Partial | LifiTransferState::Refunded | LifiTransferState::Failed =>
            recovery::reconcile(state, run, leg, recovery::report(&evidence), now_ms).await,
    }
}

fn ensure_source_confirmed(
    state: &AppState,
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    source: &str,
    now_ms: i64,
) -> Result<OnchainCrossChainRun, String> {
    match leg.status {
        OnchainCrossChainLegRunStatus::Submitted => state
            .onchain_cross_chain_runs()
            .record_source_confirmed(&run.run_id, source.to_owned(), now_ms),
        OnchainCrossChainLegRunStatus::SourceConfirmed => Ok(run.clone()),
        _ => Err("bridge leg is not in a reconcilable submitted state".to_owned()),
    }
}

fn record_pending(
    state: &AppState,
    run: &OnchainCrossChainRun,
    label: &str,
    evidence: &LifiTransferEvidence,
    now_ms: i64,
) -> Result<(), String> {
    let detail = evidence
        .message
        .as_deref()
        .or(evidence.substatus.as_deref())
        .unwrap_or(&evidence.provider_status);
    state.onchain_cross_chain_runs().record_check_problem(
        &run.run_id,
        format!("{label}：{detail}"),
        evidence.official_docs_url.clone(),
        now_ms,
    )?;
    Ok(())
}

async fn pause(
    state: &AppState,
    run: &OnchainCrossChainRun,
    problem: String,
    now_ms: i64,
) -> Result<(), String> {
    let paused = state
        .onchain_cross_chain_runs()
        .pause(&run.run_id, problem, now_ms)?;
    emit_run_webhook(state, &paused).await;
    Ok(())
}

fn active_leg(run: &OnchainCrossChainRun) -> Option<&OnchainCrossChainLegProgress> {
    let position = run.active_position?;
    run.legs.iter().find(|leg| leg.position == position)
}

pub(super) async fn emit_run_webhook(state: &AppState, run: &OnchainCrossChainRun) {
    let kind = if run.status == OnchainCrossChainRunStatus::Paused {
        WebhookEventKind::RiskAlert
    } else {
        WebhookEventKind::ExecutionResult
    };
    let event_id = webhook_event_id(run);
    let payload = serde_json::json!({
        "title": "CROSSLINE 跨链套利运行态",
        "runId": run.run_id,
        "buildId": run.build.build_id,
        "status": run.status,
        "activePosition": run.active_position,
        "sourceChain": run.build.source_chain,
        "peerChain": run.build.peer_chain,
        "nextAction": run.next_action,
        "problem": run.problem,
        "legs": run.legs,
        "accounting": run.accounting,
        "automaticCheckDeadlineMs": run.automatic_check_deadline_ms(),
        "recoveryStartedAtMs": run.active_leg().and_then(|leg| leg.recovery_started_at_ms),
        "recoveryChecks": run.active_leg().map(|leg| leg.recovery_checks),
        "recoveryCheckLimit": OnchainCrossChainLegProgress::RECOVERY_CHECK_LIMIT,
    });
    if let Err(error) =
        crate::services::webhook::emit_idempotent(state, kind, event_id, payload).await
    {
        tracing::warn!(%error, run_id = %run.run_id, "cross-chain run webhook enqueue failed");
    }
}

fn webhook_event_id(run: &OnchainCrossChainRun) -> String {
    let position = run
        .active_position
        .or_else(|| {
            run.legs
                .iter()
                .filter(|leg| leg.status == OnchainCrossChainLegRunStatus::Completed)
                .map(|leg| leg.position)
                .max()
        })
        .unwrap_or_default();
    let mut id = format!("{}:cross-chain:{position}:{:?}", run.run_id, run.status).to_ascii_lowercase();
    if run.status == OnchainCrossChainRunStatus::Paused {
        if let Some(started) = run.active_leg().and_then(|leg| leg.recovery_started_at_ms) {
            id.push_str(&format!(":recheck:{started}"));
        }
    }
    id
}
