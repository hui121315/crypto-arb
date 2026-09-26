use super::*;

#[derive(Debug)]
pub(super) enum ReadProblem {
    Unavailable(String),
    Conflict(String),
}

impl ReadProblem {
    pub(super) fn record(self, transfer: &mut StockFundingTransfer) {
        let message = match self {
            Self::Unavailable(message) => message,
            Self::Conflict(message) => {
                transfer
                    .evidence_conflict
                    .get_or_insert_with(|| message.clone());
                message
            }
        };
        transfer.problem = Some(message);
    }
}

fn created_at(record: &StockFundingDepositRecord) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(&record.created_at)
        .map(|at| at.timestamp_millis())
        .map_err(|_| "入账时间缺少明确时区".into())
}

pub(super) fn deposit_matches(
    plan: &StockFundingPlan,
    d: &StockFundingDepositRecord,
) -> Result<(), String> {
    let t = plan.transfer.as_ref().ok_or("原链上提交未知")?;
    let at = t.submitted_at_ms.ok_or("原链上提交时间未知")?;
    let time = created_at(d)?;
    let p = &t.preparation;
    if d.id < 0
        || d.source != "solana"
        || d.symbol != plan.request.funding_asset
        || Some(&d.transaction_hash) != t.transaction_hash.as_ref()
        || d.to_address.as_ref().is_some_and(|a| {
            a != &plan.terms.destination && Some(a) != p.destination_token_account.as_ref()
        })
        || d.from_address.as_ref().is_some_and(|a| {
            a != &plan.request.wallet_address && Some(a) != p.source_token_account.as_ref()
        })
        || ![
            "cancelled",
            "confirmed",
            "declined",
            "expired",
            "initiated",
            "ownershipVerificationRequired",
            "pending",
            "refunded",
            "senderVerificationRequired",
        ]
        .contains(&d.status.as_str())
        || time < at.saturating_sub(5000)
        || time > plan.updated_at_ms.saturating_add(5000)
    {
        return Err("Backpack 入账的原签名、资产、网络、地址或时序不匹配".into());
    }
    decimal(&d.quantity)?;
    Ok(())
}

pub(super) fn merge_deposit(
    plan: &StockFundingPlan,
    mut new: StockFundingDepositRecord,
) -> Result<StockFundingDepositRecord, String> {
    let Some(old) = plan.transfer.as_ref().and_then(|t| t.deposit.as_ref()) else {
        return Ok(new);
    };
    if old.id != new.id
        || old.source != new.source
        || old.symbol != new.symbol
        || old.transaction_hash != new.transaction_hash
        || created_at(old)? != created_at(&new)?
        || old.status == "confirmed"
            && (old.status != new.status || decimal(&old.quantity)? != decimal(&new.quantity)?)
    {
        return Err("原入账编号、数量、身份或已知终态冲突，保留占用".into());
    }
    // Preserve the original evidence while allowing previously absent addresses to arrive.
    for (prior, next) in [
        (&old.to_address, &mut new.to_address),
        (&old.from_address, &mut new.from_address),
    ] {
        if let Some(prior) = prior {
            if next.as_ref().is_some_and(|next| next != prior) {
                return Err("原入账已知收发地址冲突，保留占用".into());
            }
            *next = Some(prior.clone());
        }
    }
    if decimal(&old.quantity)? == decimal(&new.quantity)? {
        new.quantity = old.quantity.clone();
    }
    new.created_at = old.created_at.clone();
    Ok(new)
}

pub(super) fn deposit_from_rows(
    plan: &StockFundingPlan,
    rows: &Value,
) -> Result<Option<StockFundingDepositRecord>, ReadProblem> {
    let rows = rows
        .as_array()
        .filter(|r| r.len() <= 100)
        .ok_or_else(|| ReadProblem::Unavailable("Backpack 入账历史响应无效或过大".into()))?;
    let hash = plan
        .transfer
        .as_ref()
        .and_then(|t| t.transaction_hash.as_deref())
        .ok_or_else(|| ReadProblem::Unavailable("原补库签名未知".into()))?;
    let mut matched = rows
        .iter()
        .filter(|r| r["transactionHash"].as_str() == Some(hash));
    let Some(row) = matched.next() else {
        return Ok(None);
    };
    if matched.next().is_some() {
        return Err(ReadProblem::Conflict(
            "同一补库交易出现多条入账记录，保留占用".into(),
        ));
    }
    let d: StockFundingDepositRecord = serde_json::from_value(row.clone())
        .map_err(|_| ReadProblem::Unavailable("入账响应缺少原始字段".into()))?;
    deposit_matches(plan, &d).map_err(ReadProblem::Conflict)?;
    merge_deposit(plan, d)
        .map(Some)
        .map_err(ReadProblem::Conflict)
}
