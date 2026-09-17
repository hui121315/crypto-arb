use super::super::replenishment_credit::swap;
use crate::state::AppState;
use shared_types::{
    OnchainChainSettlement, OnchainChainSettlementBasis, OnchainChainSettlementStatus as Status,
    OnchainExecutionBuildResponse, OnchainExecutionLegKind, OnchainExecutionLegStatus,
    OnchainExecutionSubmitResponse,
};
use std::{
    collections::BTreeSet,
    sync::{Mutex, OnceLock},
};

const MAX_ATTEMPTS: u8 = 12;
const RETRY_MS: u64 = 5_000;
static RUNNING: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
static READERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub(super) fn seed(
    build: &OnchainExecutionBuildResponse,
    transaction_id: &str,
) -> Option<OnchainChainSettlement> {
    let assets = build.settlement_assets.clone()?;
    if assets.input.address != build.input_token || assets.output.address != build.output_token {
        return None;
    }
    Some(swap::pending_receipt(
        &OnchainChainSettlementBasis {
            chain: build.chain.clone(),
            wallet: build.wallet_address.clone(),
            transaction_id: transaction_id.into(),
            assets,
            maximum_input_raw: build.input_amount_raw.clone(),
            minimum_output_raw: build.minimum_output_amount_raw.clone(),
        },
        "交易收支将在终态确认后核算".into(),
    ))
}

fn pending(run: &OnchainExecutionSubmitResponse) -> Option<&OnchainChainSettlement> {
    run.legs.iter().find_map(|leg| {
        let receipt = leg.chain_settlement.as_ref()?;
        (leg.kind == OnchainExecutionLegKind::Chain
            && matches!(
                leg.status,
                OnchainExecutionLegStatus::Confirmed
                    | OnchainExecutionLegStatus::Rejected
                    | OnchainExecutionLegStatus::Failed
            )
            && leg.transaction_id.as_deref() == Some(receipt.basis.transaction_id.as_str())
            && receipt.status == Status::Pending
            && receipt.attempts < MAX_ATTEMPTS)
            .then_some(receipt)
    })
}

struct Running(String);
impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut running) = RUNNING.get_or_init(Default::default).lock() {
            running.remove(&self.0);
        }
    }
}

pub(super) fn spawn(state: &AppState, run: &OnchainExecutionSubmitResponse) {
    if pending(run).is_none() || state.onchain_execution_run_store().readiness().is_err() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let Ok(mut running) = RUNNING.get_or_init(Default::default).lock() else {
        return;
    };
    if !running.insert(run.run_id.clone()) {
        return;
    }
    let guard = Running(run.run_id.clone());
    let state = state.clone();
    handle.spawn(async move {
        let _guard = guard;
        loop {
            let Some(previous) = state
                .onchain_execution_runs()
                .get(&_guard.0)
                .and_then(|run| pending(&run).cloned())
            else {
                break;
            };
            let Ok(permit) = READERS.acquire().await else {
                break;
            };
            let mut next = match tokio::time::timeout(
                std::time::Duration::from_secs(12),
                swap::check(&state, &previous.basis),
            )
            .await
            {
                Ok(row) => row,
                Err(_) => swap::pending_receipt(&previous.basis, "链上成交核算读取超时".into()),
            };
            drop(permit);
            next.attempts = previous.attempts + 1;
            if next.status == Status::Pending && next.attempts >= MAX_ATTEMPTS {
                next.status = Status::ReviewRequired;
                next.problem = Some(format!(
                    "{}；已停止本轮自动读取，请核对 RPC 与交易哈希",
                    next.problem.as_deref().unwrap_or("收支未核清")
                ));
            }
            let again = next.status == Status::Pending;
            if !save(&state, &_guard.0, &previous, next) || !again {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_MS)).await;
        }
    });
}

fn save(
    state: &AppState,
    run_id: &str,
    previous: &OnchainChainSettlement,
    next: OnchainChainSettlement,
) -> bool {
    let Some(mut current) = state.onchain_execution_runs().get_mut(run_id) else {
        return false;
    };
    let Some(index) = current
        .legs
        .iter()
        .position(|leg| leg.chain_settlement.as_ref() == Some(previous))
    else {
        return false;
    };
    let mut updated = current.clone();
    updated.legs[index].chain_settlement = Some(next);
    updated.updated_at_ms = common::time::now_ms();
    // Only accounting data is changed: this worker cannot submit, reverse or retry a trade.
    super::settlement::enrich(state, &mut updated);
    match state.onchain_execution_run_store().append_run(&updated) {
        Ok(()) => {
            *current = updated;
            true
        }
        Err(problem) => {
            *current = super::interrupted_response(updated, problem);
            false
        }
    }
}

pub(super) fn preserve_known(
    previous: &OnchainExecutionSubmitResponse,
    next: &mut OnchainExecutionSubmitResponse,
) {
    for leg in &mut next.legs {
        let Some(incoming) = leg.chain_settlement.as_mut() else {
            continue;
        };
        let saved = previous
            .legs
            .iter()
            .filter_map(|leg| leg.chain_settlement.as_ref())
            .find(|saved| saved.basis == incoming.basis && saved.attempts >= incoming.attempts);
        if let Some(saved) = saved.filter(|_| incoming.status == Status::Pending) {
            *incoming = saved.clone();
        }
    }
}

pub(super) fn reconcile_quantity(run: &mut OnchainExecutionSubmitResponse, can_finalize: bool) {
    use rust_decimal::Decimal;
    use shared_types::{
        OnchainCexSettlementStatus, OnchainExecutionRunStatus as RunStatus, OrderSide,
    };
    const DIFFERENCE: &str = "双端实际数量未对齐";
    run.quantity_reconciled = false;
    let eligible = run.status == RunStatus::Completed
        || (run.status == RunStatus::Exposed
            && run
                .problem
                .as_deref()
                .is_some_and(|p| p.starts_with("预计双端净余量") || p.starts_with(DIFFERENCE)));
    if !eligible || !can_finalize || run.compensation_order_id.is_some() {
        return;
    }
    let mut primary_legs = run
        .legs
        .iter()
        .filter(|leg| leg.kind == OnchainExecutionLegKind::PrimaryCex);
    let Some(primary_leg) = primary_legs.next() else {
        return;
    };
    let Some(primary) = primary_leg.settlement.as_ref().filter(|row| {
        primary_legs.next().is_none()
            && row.status == OnchainCexSettlementStatus::Complete
            && matches!(
                primary_leg.status,
                OnchainExecutionLegStatus::Filled | OnchainExecutionLegStatus::Cancelled
            )
            && run.cex_order_id.as_deref() == Some(row.basis.order_id.as_str())
            && primary_leg.order_id == run.cex_order_id
            && primary_leg.venue.eq_ignore_ascii_case(&row.basis.venue)
            && primary_leg.symbol.as_deref() == Some(row.basis.symbol.as_str())
            && primary_leg.filled_quantity == Some(row.basis.confirmed_quantity)
    }) else {
        return;
    };
    let mut chain_legs = run
        .legs
        .iter()
        .filter(|leg| leg.kind == OnchainExecutionLegKind::Chain);
    let Some(chain_leg) = chain_legs.next() else {
        return;
    };
    let Some(chain) = chain_leg.chain_settlement.as_ref().filter(|row| {
        chain_legs.next().is_none()
            && chain_leg.status == OnchainExecutionLegStatus::Confirmed
            && row.status == Status::Complete
            && run.chain_transaction_id.as_deref() == Some(row.basis.transaction_id.as_str())
            && chain_leg.transaction_id == run.chain_transaction_id
            && chain_leg.venue.eq_ignore_ascii_case(&row.basis.chain)
    }) else {
        return;
    };
    if run
        .legs
        .iter()
        .filter(|leg| leg.kind == OnchainExecutionLegKind::QuoteConversion)
        .any(|leg| {
            leg.settlement.as_ref().is_none_or(|row| {
                row.status != OnchainCexSettlementStatus::Complete
                    || leg.order_id.as_deref() != Some(row.basis.order_id.as_str())
                    || !leg.venue.eq_ignore_ascii_case(&row.basis.venue)
                    || leg.symbol.as_deref() != Some(row.basis.symbol.as_str())
            })
        })
    {
        return;
    }
    let (token, raw, cex) = match primary.basis.side {
        OrderSide::Buy => (
            &chain.basis.assets.input,
            chain.input_amount_raw.as_deref(),
            primary.credit_amount.as_deref(),
        ),
        OrderSide::Sell => (
            &chain.basis.assets.output,
            chain.output_amount_raw.as_deref(),
            primary.debit_amount.as_deref(),
        ),
    };
    if !token.symbol.eq_ignore_ascii_case(&primary.basis.base_asset) || token.decimals > 28 {
        return;
    }
    let Some(cex) = cex
        .and_then(|v| Decimal::from_str_exact(v).ok())
        .filter(|v| *v > Decimal::ZERO)
    else {
        return;
    };
    let Some(chain) = raw
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| Decimal::from_str_exact(v).ok())
        .and_then(|v| v.checked_mul(Decimal::new(1, u32::from(token.decimals))))
        .filter(|v| *v > Decimal::ZERO)
    else {
        return;
    };
    let difference = match primary.basis.side {
        OrderSide::Buy => cex.checked_sub(chain),
        OrderSide::Sell => chain.checked_sub(cex),
    };
    let Some(difference) = difference else {
        return;
    };
    if difference == Decimal::ZERO {
        run.quantity_reconciled = true;
        if run.status == RunStatus::Exposed {
            run.status = RunStatus::Completed;
            run.remaining_exposure_usd = 0.0;
            run.message = "按 CEX 净成交与链上实际收支核对，对冲数量已对齐；链费另计".into();
            run.problem = None;
        }
    } else {
        run.status = RunStatus::Exposed;
        run.message = format!("{DIFFERENCE}，保留剩余币量待处理");
        let remaining = if difference > Decimal::ZERO {
            format!("剩余 {}", difference.normalize())
        } else {
            format!("还缺 {}", (-difference).normalize())
        };
        run.problem = Some(format!(
            "{DIFFERENCE}：{remaining} {}；使用 CEX 净成交与链上实际扣款/到账，未包含单列的网络费",
            token.symbol
        ));
    }
    run.recovery_actions = super::recovery_actions(run.status);
}

#[cfg(test)]
mod tests;
