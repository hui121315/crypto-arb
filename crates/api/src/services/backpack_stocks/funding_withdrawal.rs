use super::*;
use crate::services::onchain_comparison::stock_inventory;
use funding_plan::decimal;
use serde_json::{json, Value};

mod service;
pub(super) use service::check_frozen_terms;
#[cfg(test)]
mod tests;

const PATH: &str = "/wapi/v1/capital/withdrawals";
const QUERY_INTERVAL_MS: i64 = 5_000;

#[derive(Debug)]
enum ReadProblem {
    Unavailable(String),
    Conflict(String),
}

impl ReadProblem {
    fn record(self, withdrawal: &mut StockFundingWithdrawal) {
        let message = match self {
            Self::Unavailable(message) => message,
            Self::Conflict(message) => {
                withdrawal.evidence_conflict.get_or_insert_with(|| message.clone());
                message
            }
        };
        withdrawal.problem = Some(message);
    }
}

fn client_id(plan: &StockFundingPlan) -> String {
    format!("bp-{}", plan.plan_id)
}

pub(super) fn validate(plan: &StockFundingPlan) -> Result<(), String> {
    funding_followup::validate(plan)?;
    if plan.transfer.is_some() {return funding_transfer::validate(plan);}
    let Some(w) = &plan.withdrawal else {
        return if matches!(
            plan.phase,
            StockFundingPlanPhase::Reserved | StockFundingPlanPhase::Cancelled
        ) {
            Ok(())
        } else {
            Err("补库提交记录缺失，不能释放占用".into())
        };
    };
    if plan.request.target != StockFundingTarget::Solana
        || !matches!(
            plan.phase,
            StockFundingPlanPhase::Withdrawing | StockFundingPlanPhase::Received
        )
        || (plan.phase == StockFundingPlanPhase::Received) != w.receipt.is_some()
        || w.client_id != client_id(plan)
        || w.submitted_at_ms < plan.terms.created_at_ms
        || w.submitted_at_ms >= plan.terms.valid_until_ms
        || w.submitted_at_ms > plan.updated_at_ms
        || (w.query_count == 0) != w.last_query_at_ms.is_none()
        || w.last_query_at_ms
            .is_some_and(|t| t < w.submitted_at_ms || t > plan.updated_at_ms)
        || w.problem.as_ref().is_some_and(|s| s.len() > 1024)
        || w.evidence_conflict
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 1024)
    {
        return Err("补库提交身份、版本或时序无效".into());
    }
    if let Some(r) = &w.remote {
        remote_matches(plan, r)?;
    }
    if let Some(r) = &w.receipt {
        let remote = w.remote.as_ref().ok_or("到账缺少原提现凭据")?;
        stock_inventory::validate_owner(&r.fee_payer)?;
        if remote.is_internal
            || remote.status != "confirmed"
            || remote.transaction_hash.as_deref() != Some(&r.transaction_hash)
            || r.destination != plan.terms.destination
            || Some(&r.mint) != plan.terms.token.contract_address.as_ref()
            || Some(r.decimals) != plan.terms.token.native_decimals
            || r.credited_raw.parse::<u64>().ok().is_none_or(|v| v == 0)
            || r.slot == 0
            || r.slot < plan.terms.mint.slot
            || r.block_time_ms < w.submitted_at_ms.saturating_sub(5_000)
            || r.block_time_ms > r.checked_at_ms.saturating_add(5_000)
            || r.checked_at_ms < w.submitted_at_ms
            || r.checked_at_ms > plan.updated_at_ms
        {
            return Err("补库到账身份、原始数量或终态证据无效".into());
        }
    }
    Ok(())
}

pub(super) fn transition(old: &StockFundingPlan, new: &StockFundingPlan) -> Result<(), String> {
    if old.request != new.request
        || old.terms != new.terms
        || old.plan_id != new.plan_id
        || new.revision != old.revision.checked_add(1).ok_or("补库版本溢出")?
        || new.updated_at_ms < old.updated_at_ms
    {
        return Err("补库历史身份或版本被改写".into());
    }
    validate(new)?;
    if old.followup != new.followup {
        return funding_followup::transition(old, new);
    }
    if old.transfer.is_some() || new.transfer.is_some() {return funding_transfer::transition(old,new);}
    match (&old.withdrawal, &new.withdrawal) {
        (None, None)
            if old.phase == StockFundingPlanPhase::Reserved
                && new.phase == StockFundingPlanPhase::Cancelled =>
        {
            Ok(())
        }
        (None, Some(w))
            if old.phase == StockFundingPlanPhase::Reserved
                && new.updated_at_ms < new.terms.valid_until_ms
                && w.submitted_at_ms == new.updated_at_ms
                && w.remote.is_none()
                && w.receipt.is_none()
                && w.query_count == 0 =>
        {
            Ok(())
        }
        (Some(a), Some(b)) => {
            if a.client_id != b.client_id
                || a.submitted_at_ms != b.submitted_at_ms
                || b.query_count < a.query_count
                || b.query_count > a.query_count.saturating_add(1)
                || b.last_query_at_ms < a.last_query_at_ms
                || (b.query_count == a.query_count && b.last_query_at_ms != a.last_query_at_ms)
                || (b.query_count > a.query_count && b.last_query_at_ms != Some(new.updated_at_ms))
                || a.receipt
                    .as_ref()
                    .is_some_and(|r| b.receipt.as_ref() != Some(r))
                || a.evidence_conflict
                    .as_ref()
                    .is_some_and(|reason| b.evidence_conflict.as_ref() != Some(reason))
            {
                return Err("原补库提交、查询或到账记录不能被改写".into());
            }
            if a.remote.is_some() {
                let n = b.remote.as_ref().ok_or("原提现凭据不能删除")?;
                if merge_remote(old, n.clone())? != *n {
                    return Err("原提现已知数量、费用或链上交易不能被移除或改写".into());
                }
            }
            Ok(())
        }
        _ => Err("已提交的补库不能取消、过期或重新提交".into()),
    }
}

fn remote_matches(plan: &StockFundingPlan, r: &StockWithdrawalRecord) -> Result<(), String> {
    let w = plan.withdrawal.as_ref().ok_or("未保存提现意图")?;
    let created = chrono::DateTime::parse_from_rfc3339(&r.created_at)
        .map_err(|_| "原提现创建时间无效")?
        .timestamp_millis();
    if r.id < 0
        || r.client_id != w.client_id
        || r.blockchain != "Solana"
        || r.symbol != plan.request.funding_asset
        || r.to_address != plan.terms.destination
        || decimal(&r.quantity)? != decimal(&plan.terms.quantity)?
        || created < w.submitted_at_ms.saturating_sub(5_000)
        || created > plan.updated_at_ms.saturating_add(5_000)
        || r.status.is_empty()
        || r.status.len() > 64
        || !r
            .status
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err("提现编号、网络、资产、数量或接收地址不匹配".into());
    }
    if let Some(fee) = &r.fee {
        decimal(fee)?;
    }
    if let Some(hash) = &r.transaction_hash {
        if bs58::decode(hash)
            .into_vec()
            .ok()
            .is_none_or(|v| v.len() != 64)
        {
            return Err("原提现 Solana 交易签名无效".into());
        }
    }
    Ok(())
}

fn parse(plan: &StockFundingPlan, value: Value) -> Result<StockWithdrawalRecord, ReadProblem> {
    let mut row: StockWithdrawalRecord =
        serde_json::from_value(value).map_err(|_| ReadProblem::Unavailable("原提现响应不完整，继续查询原编号，不能重提".into()))?;
    if row.transaction_hash.as_deref() == Some("") {
        row.transaction_hash = None;
    }
    remote_matches(plan, &row).map_err(ReadProblem::Conflict)?;
    merge_remote(plan, row).map_err(ReadProblem::Conflict)
}

fn merge_remote(plan: &StockFundingPlan, mut row: StockWithdrawalRecord) -> Result<StockWithdrawalRecord, String> {
    let Some(old) = plan.withdrawal.as_ref().and_then(|w| w.remote.as_ref()) else {
        return Ok(row);
    };
    if old.id != row.id
        || old.client_id != row.client_id
        || old.created_at != row.created_at
        || old.is_internal != row.is_internal
        || decimal(&old.quantity)? != decimal(&row.quantity)?
        || old.fee.as_ref().zip(row.fee.as_ref())
            .is_some_and(|(a, b)| decimal(a).ok() != decimal(b).ok())
        || old.transaction_hash.as_ref().zip(row.transaction_hash.as_ref())
            .is_some_and(|(a, b)| a != b)
        || (old.status == "confirmed" && row.status != "confirmed")
    {
        return Err("原提现身份、费用或链上交易发生冲突，保留占用并暂停自动核验".into());
    }
    // A sparse or differently formatted response cannot erase already verified evidence.
    row.quantity.clone_from(&old.quantity);
    if old.fee.is_some() { row.fee.clone_from(&old.fee); }
    if old.transaction_hash.is_some() { row.transaction_hash.clone_from(&old.transaction_hash); }
    Ok(row)
}

fn payload(plan: &StockFundingPlan, token: Option<&str>) -> Result<Value, String> {
    if token.is_some_and(|t| t.is_empty() || t.len() > 2048 || t.chars().any(char::is_control)) {
        return Err("2FA 凭证格式无效，未提交".into());
    }
    let mut body = json!({
        "address":plan.terms.destination, "blockchain":"Solana", "clientId":client_id(plan),
        "quantity":plan.terms.quantity, "symbol":plan.request.funding_asset,
        "autoBorrow":false, "autoLendRedeem":false
    });
    if let Some(token) = token {
        body["twoFactorToken"] = Value::String(token.into());
    }
    Ok(body)
}
