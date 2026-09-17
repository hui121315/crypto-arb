use serde::Deserialize;
use shared_types::stocks::{comparison::positive, *};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Historical {
    rfq_id: String,
    client_id: Option<u32>,
    symbol: String,
    side: StockRfqSide,
    quantity: Option<String>,
    status: String,
    execution_mode: String,
    deferred_settlement_quote_id: Option<String>,
}

pub(super) fn select(bytes: &[u8], record: &StockRfq) -> Result<Option<Historical>, String> {
    let rows: Vec<Historical> = serde_json::from_slice(bytes).map_err(|_| "RFQ 历史结构无效")?;
    if rows.len() > 100 {
        return Err("RFQ 历史超出本次核对范围".into());
    }
    let mut rows = rows
        .into_iter()
        .filter(|r| Some(&r.rfq_id) == record.rfq_id.as_ref());
    let row = rows.next();
    if rows.next().is_some() {
        return Err("RFQ 历史 ID 重复，需人工核对".into());
    }
    Ok(row)
}

pub(super) fn apply(record: &mut StockRfq, native: Historical, now: i64) -> Result<bool, String> {
    if Some(&native.rfq_id) != record.rfq_id.as_ref()
        || native.client_id != Some(record.client_id)
        || native.symbol != record.symbol
        || native.side != record.request.side
        || native.execution_mode != "AwaitAccept"
        || native.quantity.as_deref().and_then(positive) != positive(&record.request.quantity)
        || record.acceptance.as_ref().is_some_and(|a| {
            native
                .deferred_settlement_quote_id
                .as_ref()
                .is_some_and(|id| id != &a.quote_id)
        })
    {
        return Err("RFQ 历史与原请求不匹配".into());
    }
    let phase = match native.status.as_str() {
        "Filled" => StockRfqPhase::Filled,
        "Cancelled" => StockRfqPhase::Cancelled,
        "Expired" => StockRfqPhase::Expired,
        "New"
            if native
                .deferred_settlement_quote_id
                .as_deref()
                .is_some_and(|id| id.parse::<u64>().is_ok_and(|id| id > 0)) =>
        {
            StockRfqPhase::AcceptedBinding
        }
        "New" => return Ok(false),
        _ => return Err("RFQ 历史状态暂不支持，保留原请求待核对".into()),
    };
    if !super::rfq_protocol::terminal_receipt(record, phase)? || record.phase == phase {
        return Ok(false);
    }
    record.phase = phase;
    record.candidate = None;
    record.needs_recheck = record.settlement_pending();
    record.problem = record
        .needs_recheck
        .then(|| "交易所已报告成交，实际股数与成交金额仍待核对".into());
    record.updated_at_ms = now;
    // Historical times have no documented timezone. Do not use them to recreate a live quote window.
    Ok(true)
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Fill {
    rfq_id: String,
    quote_id: String,
    client_id: Option<u32>,
    symbol: String,
    side: StockRfqSide,
    quantity: Option<String>,
    fill_quantity: Option<String>,
    fill_quote_quantity: Option<String>,
    fill_price: String,
}

pub(super) fn parse_fills(bytes: &[u8]) -> Result<Vec<Fill>, String> {
    let rows: Vec<Fill> = serde_json::from_slice(bytes).map_err(|_| "RFQ 成交历史结构无效")?;
    if rows.len() >= 100 {
        return Err("RFQ 成交历史可能被截断，暂不合计".into());
    }
    if rows.is_empty() {
        return Err("已成交 RFQ 暂无实际成交明细；保留原记录，不重发".into());
    }
    Ok(rows)
}

#[cfg(test)]
pub(super) fn fills(record: &mut StockRfq, bytes: &[u8], now: i64) -> Result<bool, String> {
    apply_fills(record, parse_fills(bytes)?, now)
}

pub(super) fn apply_fills(
    record: &mut StockRfq,
    rows: Vec<Fill>,
    now: i64,
) -> Result<bool, String> {
    if record.phase != StockRfqPhase::Filled {
        return Err("RFQ 尚未确认成交终态，保留原状态".into());
    }
    let mut unique = BTreeMap::new();
    for row in rows {
        if Some(&row.rfq_id) != record.rfq_id.as_ref()
            || row.symbol != record.symbol
            || row.side != record.request.side
            || row.client_id != Some(record.client_id)
            || row.quantity.as_deref().and_then(positive) != positive(&record.request.quantity)
            || positive(&row.fill_price).is_none()
            || row.fill_quantity.as_deref().and_then(positive).is_none()
            || row
                .fill_quote_quantity
                .as_deref()
                .and_then(positive)
                .is_none()
            || !row.quote_id.parse::<u64>().is_ok_and(|id| id > 0)
            || record
                .acceptance
                .as_ref()
                .is_some_and(|a| a.quote_id != row.quote_id)
        {
            return Err("RFQ 成交历史身份或数量不匹配".into());
        }
        if let Some(old) = unique.get(&row.quote_id) {
            if old != &row {
                return Err("同一报价出现不同成交回执，停止合计".into());
            }
        } else {
            unique.insert(row.quote_id.clone(), row);
        }
    }
    let sum = |quote: bool| -> Option<String> {
        unique
            .values()
            .try_fold(rust_decimal::Decimal::ZERO, |total, row| {
                let value = if quote {
                    row.fill_quote_quantity.as_deref()
                } else {
                    row.fill_quantity.as_deref()
                }?;
                total.checked_add(positive(value)?)
            })
            .map(|v| v.normalize().to_string())
    };
    let quantity = sum(false).ok_or("RFQ 成交股数溢出")?;
    let quote = sum(true).ok_or("RFQ 成交金额溢出")?;
    if positive(&quantity) > positive(&record.request.quantity) {
        return Err("RFQ 合计成交股数超过请求".into());
    }
    for (old, new) in [
        (&record.executed_quantity, &quantity),
        (&record.executed_quote_quantity, &quote),
    ] {
        if old
            .as_deref()
            .and_then(positive)
            .is_some_and(|old| positive(new).is_none_or(|new| new < old))
        {
            return Err("RFQ 成交明细少于已核实累计值，保留原数值待核对".into());
        }
    }
    let fills = unique
        .into_values()
        .map(|r| StockRfqFill {
            quote_id: r.quote_id,
            quantity: r.fill_quantity.expect("validated fill quantity"),
            quote_quantity: r
                .fill_quote_quantity
                .expect("validated fill quote quantity"),
            price: r.fill_price,
        })
        .collect::<Vec<_>>();
    if record
        .fills
        .iter()
        .any(|old| !fills.iter().any(|r| r == old))
    {
        return Err("RFQ 历史缺少或改写已核实明细，保留原数值待核对".into());
    }
    let before = record.clone();
    record.executed_quantity = Some(quantity);
    record.executed_quote_quantity = Some(quote);
    record.fills = fills;
    record.needs_recheck = record.settlement_pending();
    record.problem = record
        .needs_recheck
        .then(|| "成交明细尚未覆盖请求股数，保留实际数值继续核对".into());
    if !record.needs_recheck {
        record.settlement.paused = false;
        record.settlement.next_at_ms = None;
    }
    if before == *record {
        return Ok(false);
    }
    record.updated_at_ms = now;
    Ok(true)
}
