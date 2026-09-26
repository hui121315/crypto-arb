use super::*;
use crate::services::onchain_comparison::stock_funding_transfer as chain;
use funding_plan::decimal;
use serde_json::{json, Value};

mod deposit;
mod deposit_scan;
mod service;
use deposit::{deposit_from_rows, deposit_matches, merge_deposit, ReadProblem};
#[cfg(test)]
mod tests;

pub(super) fn phase(plan: &StockFundingPlan, t: &StockFundingTransfer) -> StockFundingPlanPhase {
    if t.submitted_at_ms.is_none() {
        return StockFundingPlanPhase::Reserved;
    }
    if t.evidence_conflict.is_some() {
        return if t.receipt.as_ref().is_some_and(|r| r.succeeded) {
            StockFundingPlanPhase::DepositPending
        } else {
            StockFundingPlanPhase::Transferring
        };
    }
    match &t.receipt {
        Some(r) if r.within_plan && !r.succeeded => StockFundingPlanPhase::TransferFailed,
        Some(r)
            if r.within_plan
                && r.succeeded
                && t.deposit_scan
                    .as_ref()
                    .is_none_or(|s| s.completed_at_ms.is_some() && s.matched)
                && t.deposit.as_ref().is_some_and(|d| {
                    d.status == "confirmed"
                        && decimal(&d.quantity).ok() == decimal(&plan.terms.quantity).ok()
                }) =>
        {
            StockFundingPlanPhase::Deposited
        }
        Some(r) if r.succeeded => StockFundingPlanPhase::DepositPending,
        _ => StockFundingPlanPhase::Transferring,
    }
}

pub(super) fn validate(plan: &StockFundingPlan) -> Result<(), String> {
    let t = plan.transfer.as_ref().ok_or("原链上补库记录缺失")?;
    chain::validate_preparation(plan, &t.preparation)?;
    if plan.withdrawal.is_some()
        || plan.request.target != StockFundingTarget::Backpack
        || t.preparation.prepared_at_ms > plan.updated_at_ms
        || t.problem.as_ref().is_some_and(|s| s.len() > 1024)
        || t.evidence_conflict
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 1024)
        || (plan.phase != phase(plan, t)
            && !(plan.phase == StockFundingPlanPhase::Cancelled && t.submitted_at_ms.is_none()))
    {
        return Err("原补库转账方向、状态或证据不一致".into());
    }
    let Some(at) = t.submitted_at_ms else {
        return if t.transaction_hash.is_none()
            && !t.acknowledged
            && t.query_count == 0
            && t.last_query_at_ms.is_none()
            && t.receipt.is_none()
            && t.deposit.is_none()
            && t.evidence_conflict.is_none()
            && t.deposit_scan.is_none()
        {
            Ok(())
        } else {
            Err("未提交补库不能带有到账记录".into())
        };
    };
    if at < t.preparation.prepared_at_ms
        || at >= plan.terms.valid_until_ms
        || at > plan.updated_at_ms
        || !t.transaction_hash.as_deref().is_some_and(
            crate::services::onchain_comparison::stock_costs::execution::valid_signature,
        )
        || (t.query_count == 0) != t.last_query_at_ms.is_none()
        || t.last_query_at_ms
            .is_some_and(|n| n < at || n > plan.updated_at_ms)
    {
        return Err("原补库提交身份或查询时序无效".into());
    }
    if let Some(r) = &t.receipt {
        let expected = if r.succeeded { chain::amount(plan)? } else { 0 };
        let within = r.source_debit_raw == expected
            && r.destination_credit_raw == expected
            && r.network_fee_lamports <= t.preparation.network_fee_lamports
            && r.account_creation_lamports
                <= t.preparation
                    .account_creation
                    .as_ref()
                    .map_or(0, |c| c.rent_budget_lamports)
            && (r.succeeded || r.account_creation_lamports == 0)
            && (plan.request.funding_asset != plan.request.security_asset
                || !r.succeeded
                || plan
                    .terms
                    .mint
                    .next_change_at_ms
                    .is_none_or(|at| r.block_time_ms < at));
        if Some(&r.transaction_hash) != t.transaction_hash.as_ref()
            || r.slot < t.preparation.slot
            || r.block_time_ms < at.saturating_sub(5000)
            || r.block_time_ms > r.checked_at_ms.saturating_add(5000)
            || r.checked_at_ms < at
            || r.checked_at_ms > plan.updated_at_ms
            || r.within_plan != within
            || r.wallet_debit_lamports
                != r.network_fee_lamports
                    .checked_add(r.account_creation_lamports)
                    .and_then(|n| {
                        n.checked_add(if plan.request.funding_asset == "SOL" {
                            r.source_debit_raw
                        } else {
                            0
                        })
                    })
                    .ok_or("收支溢出")?
        {
            return Err("原补库实际收支或链上终态不一致".into());
        }
    }
    if let Some(d) = &t.deposit {
        if !t.receipt.as_ref().is_some_and(|r| r.succeeded) {
            return Err("交易所入账缺少原转账成功回执".into());
        }
        deposit_matches(plan, d)?;
    }
    deposit_scan::validate(plan)
}

pub(super) fn transition(old: &StockFundingPlan, new: &StockFundingPlan) -> Result<(), String> {
    if old.withdrawal != new.withdrawal {
        return Err("补库转入不能改写提现记录".into());
    }
    let b = new.transfer.as_ref().ok_or("原补库转账记录不能删除")?;
    let Some(a) = &old.transfer else {
        return if old.phase == StockFundingPlanPhase::Reserved
            && b.submitted_at_ms.is_none()
            && new.updated_at_ms < new.terms.valid_until_ms
            && b.preparation.prepared_at_ms <= new.updated_at_ms
        {
            Ok(())
        } else {
            Err("只能为有效的原补库计划保存未签名交易".into())
        };
    };
    if a.preparation != b.preparation
        || a.submitted_at_ms
            .is_some_and(|at| b.submitted_at_ms != Some(at))
        || a.transaction_hash
            .as_ref()
            .is_some_and(|h| b.transaction_hash.as_ref() != Some(h))
        || a.acknowledged && !b.acknowledged
        || b.query_count < a.query_count
        || b.query_count > a.query_count.saturating_add(1)
        || b.last_query_at_ms < a.last_query_at_ms
        || b.query_count == a.query_count && b.last_query_at_ms != a.last_query_at_ms
        || b.query_count > a.query_count && b.last_query_at_ms != Some(new.updated_at_ms)
        || a.receipt
            .as_ref()
            .is_some_and(|r| b.receipt.as_ref() != Some(r))
        || a.evidence_conflict
            .as_ref()
            .is_some_and(|reason| b.evidence_conflict.as_ref() != Some(reason))
    {
        return Err("原补库消息、签名、收支或查询记录不能改写".into());
    }
    if a.submitted_at_ms.is_none()
        && b.submitted_at_ms.is_some()
        && (old.phase != StockFundingPlanPhase::Reserved
            || b.submitted_at_ms != Some(new.updated_at_ms)
            || b.acknowledged
            || b.receipt.is_some()
            || b.deposit.is_some()
            || b.evidence_conflict.is_some()
            || b.deposit_scan.is_some()
            || b.query_count != 0)
    {
        return Err("补库只能从原有效计划记录一次提交".into());
    }
    if a.submitted_at_ms.is_none()
        && b.submitted_at_ms.is_none()
        && !(old.phase == StockFundingPlanPhase::Reserved
            && new.phase == StockFundingPlanPhase::Cancelled
            && a == b)
    {
        return Err("未签名补库交易不能被重新编制或复用".into());
    }
    if a.deposit.is_some() {
        let n = b.deposit.as_ref().ok_or("原入账记录不能删除")?;
        let merged = merge_deposit(old, n.clone())?;
        if merged.to_address != n.to_address || merged.from_address != n.from_address {
            return Err("原交易所入账凭据冲突，保留占用".into());
        }
    }
    deposit_scan::transition(old, new)
}
