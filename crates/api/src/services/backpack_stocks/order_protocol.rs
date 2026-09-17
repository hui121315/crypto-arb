use super::*;
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn decimal(s: &str) -> Result<Decimal, String> {
    Decimal::from_str_exact(s).map_err(|_| "股票订单金额格式无效".into())
}

fn amount(v: &Value, key: &str) -> Result<String, String> {
    Ok(decimal(v[key].as_str().ok_or("股票订单缺少原始金额")?)?
        .normalize()
        .to_string())
}

pub(super) fn id(v: &Value) -> Option<String> {
    let s = v
        .as_str()
        .map(str::to_owned)
        .or_else(|| v.as_u64().map(|n| n.to_string()))?;
    (!s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
    .then_some(s)
}

pub(super) fn client(v: &Value) -> Option<u32> {
    id(v)?.parse().ok()
}

pub(super) fn identity(
    instruction: &StockCexInstruction,
    value: &Value,
    ws: bool,
) -> Result<(), String> {
    let StockCexInstruction::OrderBook {
        client_id,
        symbol,
        side,
        ..
    } = instruction
    else {
        return Err("不是股票订单簿计划".into());
    };
    let (c, s, d) = if ws {
        ("c", "s", "S")
    } else {
        ("clientId", "symbol", "side")
    };
    if client(&value[c]) != Some(*client_id)
        || value[s].as_str() != Some(symbol)
        || value[d].as_str()
            != Some(if *side == StockRfqSide::Bid {
                "Bid"
            } else {
                "Ask"
            })
    {
        return Err("股票回执与原订单的 clientId、市场或方向不一致".into());
    }
    Ok(())
}

pub(super) fn apply_order(
    row: &mut StockCexOrder,
    instruction: &StockCexInstruction,
    v: &Value,
    ws: bool,
    now: i64,
) -> Result<bool, String> {
    identity(instruction, v, ws)?;
    let StockCexInstruction::OrderBook {
        quantity,
        limit_price,
        ..
    } = instruction
    else {
        unreachable!()
    };
    let (order_id, q, p, tif, kind, status, z, quote) = if ws {
        ("i", "q", "p", "f", "o", "X", "z", "Z")
    } else {
        (
            "id",
            "quantity",
            "price",
            "timeInForce",
            "orderType",
            "status",
            "executedQuantity",
            "executedQuoteQuantity",
        )
    };
    if decimal(&amount(v, q)?)? != decimal(quantity)?
        || decimal(&amount(v, p)?)? != decimal(limit_price)?
        || v[tif] != "FOK"
        || v[kind] != if ws { "LIMIT" } else { "Limit" }
    {
        return Err("股票回执改变了原订单的数量、限价或 FOK 约束".into());
    }
    let remote = id(&v[order_id]).ok_or("股票回执缺少有效 orderId")?;
    if row.order_id.as_ref().is_some_and(|old| old != &remote) {
        return Err("原 clientId 对应多个远端订单，保留占用并停止自动核对".into());
    }
    let next = match v[status].as_str() {
        Some("New" | "PartiallyFilled") => StockCexOrderPhase::Open,
        Some("Filled") => StockCexOrderPhase::Filled,
        Some("Cancelled") => StockCexOrderPhase::Cancelled,
        Some("Expired") => StockCexOrderPhase::Expired,
        _ => return Err("股票订单状态尚未支持，不能认定成交或释放预留".into()),
    };
    let before = row.clone();
    row.order_id = Some(remote);
    if row.phase.terminal() && next.terminal() && row.phase != next {
        return Err("股票订单终态互相冲突，需核对原订单".into());
    }
    let executed = amount(v, z)?;
    let notional = amount(v, quote)?;
    let older = row
        .executed_quantity
        .as_deref()
        .map(decimal)
        .transpose()?
        .is_some_and(|old| decimal(&executed).is_ok_and(|n| n < old));
    if !older {
        if row.executed_quantity.as_deref().map(decimal).transpose()? == Some(decimal(&executed)?)
            && row
                .executed_quote_quantity
                .as_deref()
                .is_some_and(|old| decimal(old).ok() != decimal(&notional).ok())
        {
            return Err("相同累计股数的成交金额冲突".into());
        }
        row.executed_quantity = Some(executed);
        row.executed_quote_quantity = Some(notional);
        if !row.phase.terminal() {
            row.phase = next;
        }
    }
    if ws && v["e"] == "orderFill" {
        merge_fill(
            row,
            StockCexFill {
                trade_id: id(&v["t"]).ok_or("股票成交缺少 tradeId")?,
                quantity: amount(v, "l")?,
                price: amount(v, "L")?,
                fee: fee(v, "n", "N")?,
            },
        )?;
    }
    validate(row, instruction)?;
    finish(row, &before, now)
}

fn fee(v: &Value, amount_key: &str, asset_key: &str) -> Result<Option<StockTradeFee>, String> {
    if v[amount_key].is_null() || v[asset_key].is_null() {
        return Ok(None);
    }
    let asset = v[asset_key]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
        .ok_or("股票成交费用币种无效")?;
    Ok(Some(StockTradeFee {
        asset: asset.into(),
        quantity: amount(v, amount_key)?,
    }))
}

fn merge_fill(row: &mut StockCexOrder, fill: StockCexFill) -> Result<(), String> {
    if let Some(old) = row.fills.iter_mut().find(|f| f.trade_id == fill.trade_id) {
        if old.quantity != fill.quantity
            || old.price != fill.price
            || old
                .fee
                .as_ref()
                .zip(fill.fee.as_ref())
                .is_some_and(|(a, b)| a != b)
        {
            return Err("同一 tradeId 的成交或原币费用冲突".into());
        }
        if old.fee.is_none() {
            old.fee = fill.fee;
        }
    } else {
        if row.fills.len() >= 512 {
            return Err("股票订单成交明细超过单笔预算，请保留原记录核查".into());
        }
        row.fills.push(fill);
    }
    Ok(())
}

pub(super) fn apply_fills(
    row: &mut StockCexOrder,
    instruction: &StockCexInstruction,
    values: &[Value],
    now: i64,
) -> Result<bool, String> {
    let before = row.clone();
    for v in values {
        identity(instruction, v, false)?;
        if id(&v["orderId"]).as_ref() != row.order_id.as_ref() || row.order_id.is_none() {
            return Err("成交历史不属于原订单".into());
        }
        merge_fill(
            row,
            StockCexFill {
                trade_id: id(&v["tradeId"]).ok_or("成交历史缺少 tradeId")?,
                quantity: amount(v, "quantity")?,
                price: amount(v, "price")?,
                fee: fee(v, "fee", "feeSymbol")?,
            },
        )?;
    }
    validate(row, instruction)?;
    finish(row, &before, now)
}

fn finish(row: &mut StockCexOrder, before: &StockCexOrder, now: i64) -> Result<bool, String> {
    if row.receipt_complete() {
        row.problem = None;
        row.recheck.next_at_ms = None;
        row.recheck.paused = false;
    }
    if row != before {
        row.updated_at_ms = now.max(row.updated_at_ms);
        Ok(true)
    } else {
        Ok(false)
    }
}

pub(super) fn validate(
    row: &StockCexOrder,
    instruction: &StockCexInstruction,
) -> Result<(), String> {
    let StockCexInstruction::OrderBook {
        quantity,
        limit_price,
        side,
        ..
    } = instruction
    else {
        return Err("RFQ 不能混用订单簿回执".into());
    };
    let expected = decimal(quantity)?;
    if row.submitted_at_ms <= 0
        || row.updated_at_ms < row.submitted_at_ms
        || row.recheck.attempts > 6
        || row
            .order_id
            .as_ref()
            .is_some_and(|i| id(&Value::String(i.clone())).as_ref() != Some(i))
        || row.fills.len() > 512
    {
        return Err("股票回执身份、时间或核对预算无效".into());
    }
    let mut ids = BTreeSet::new();
    for f in &row.fills {
        let price = decimal(&f.price)?;
        if !ids.insert(&f.trade_id)
            || id(&Value::String(f.trade_id.clone())).is_none()
            || decimal(&f.quantity)? <= Decimal::ZERO
            || price <= Decimal::ZERO
            || if *side == StockRfqSide::Bid {
                price > decimal(limit_price)?
            } else {
                price < decimal(limit_price)?
            }
        {
            return Err("股票成交编号、股数或价格不符合原限价".into());
        }
        if let Some(fee) = &f.fee {
            decimal(&fee.quantity)?;
            if fee.asset.is_empty()
                || fee.asset.len() > 64
                || !fee
                    .asset
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            {
                return Err("股票费用币种无效".into());
            }
        }
    }
    let (fq, fv) = row.fill_totals().ok_or("股票成交金额溢出")?;
    let q = row.executed_quantity.as_deref().map(decimal).transpose()?;
    let v = row
        .executed_quote_quantity
        .as_deref()
        .map(decimal)
        .transpose()?;
    if q.is_some() != v.is_some()
        || q.is_some_and(|n| n < Decimal::ZERO || n > expected || n < fq)
        || v.is_some_and(|n| n < Decimal::ZERO || n < fv)
        || fq > expected
        || (row.phase == StockCexOrderPhase::Filled && q != Some(expected))
        || (row.phase == StockCexOrderPhase::Rejected
            && (q != Some(Decimal::ZERO) || v != Some(Decimal::ZERO) || row.order_id.is_some()))
        || (row.phase != StockCexOrderPhase::SubmissionUnknown
            && row.phase != StockCexOrderPhase::Rejected
            && (row.order_id.is_none() || q.is_none()))
        || (q == Some(Decimal::ZERO) && v != Some(Decimal::ZERO))
    {
        return Err("累计成交、明细或订单终态不一致，不能确认净到账".into());
    }
    Ok(())
}

pub(super) fn transition(old: Option<&StockCexOrder>, new: Option<&StockCexOrder>) -> bool {
    let Some(old) = old else {
        return true;
    };
    let Some(new) = new else {
        return false;
    };
    old.submitted_at_ms == new.submitted_at_ms
        && (!old.evidence_conflict || new.evidence_conflict)
        && old.updated_at_ms <= new.updated_at_ms
        && old.recheck.attempts <= new.recheck.attempts
        && old
            .order_id
            .as_ref()
            .is_none_or(|id| new.order_id.as_ref() == Some(id))
        && (!old.phase.terminal() || old.phase == new.phase)
        && [&old.executed_quantity, &old.executed_quote_quantity]
            .into_iter()
            .zip([&new.executed_quantity, &new.executed_quote_quantity])
            .all(|(a, b)| {
                a.as_deref().is_none_or(|a| {
                    b.as_deref()
                        .and_then(|b| decimal(b).ok())
                        .zip(decimal(a).ok())
                        .is_some_and(|(b, a)| b >= a)
                })
            })
        && old.fills.iter().all(|a| {
            new.fills.iter().any(|b| {
                a.trade_id == b.trade_id
                    && a.quantity == b.quantity
                    && a.price == b.price
                    && a.fee.as_ref().is_none_or(|f| b.fee.as_ref() == Some(f))
            })
        })
}
