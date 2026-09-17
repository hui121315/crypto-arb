use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::stocks::comparison::positive;
use shared_types::stocks::*;

pub(super) fn submit_body(rfq: &StockRfq) -> Value {
    json!({"clientId":rfq.client_id,"symbol":rfq.symbol,"side":rfq.request.side,"quantity":rfq.request.quantity,
        "executionMode":"AwaitAccept","autoBorrow":false,"autoBorrowRepay":false,"autoLend":false,"autoLendRedeem":false})
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeRfq {
    rfq_id: String,
    client_id: Option<u32>,
    symbol: String,
    side: StockRfqSide,
    quantity: Option<String>,
    execution_mode: String,
    status: String,
    submission_time: i64,
    expiry_time: i64,
    created_at: i64,
    executed_quantity: Option<String>,
    executed_quote_quantity: Option<String>,
}

pub(super) fn validate_quantity(
    request: &StockRfqRequest,
    snapshot: &StockMarketSnapshot,
    now: i64,
) -> Result<String, String> {
    validate_rfq_quantity(request, snapshot, now)
}

fn id(value: &Value) -> Option<String> {
    value
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| value.as_u64())
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
}

pub(super) fn terminal_receipt(record: &StockRfq, next: StockRfqPhase) -> Result<bool, String> {
    if record.phase.terminal() {
        if next.terminal() && next != record.phase {
            return Err("RFQ 成交/取消终态互相矛盾，保留原回执与资金占用，停止自动处理".into());
        }
        return Ok(next == record.phase);
    }
    if record.acceptance.as_ref().is_some_and(|a| a.rejected)
        && matches!(next, StockRfqPhase::AcceptedBinding | StockRfqPhase::Filled)
    {
        return Err("RFQ 接受被拒绝后又收到锁资或成交回执，需要核对原请求".into());
    }
    Ok(true)
}

pub(super) fn apply_rest(
    record: &mut StockRfq,
    native: NativeRfq,
    now: i64,
) -> Result<bool, String> {
    let remote = id(&Value::String(native.rfq_id)).ok_or("Backpack RFQ ID 无效")?;
    if record.rfq_id.as_ref().is_some_and(|id| id != &remote)
        || native.symbol != record.symbol
        || native.side != record.request.side
        || native.client_id != Some(record.client_id)
        || native.quantity.as_deref().and_then(positive) != positive(&record.request.quantity)
        || native.execution_mode != "AwaitAccept"
        || native.created_at < record.created_at_ms.saturating_sub(60_000)
        || native.created_at > now + 3000
        || native.submission_time < native.created_at
        || native.expiry_time <= native.submission_time
        || (record.acceptance.is_some()
            && (record.submission_time_ms != Some(native.submission_time)
                || record.expiry_time_ms != Some(native.expiry_time)))
    {
        return Err("Backpack RFQ 回执与原请求或时窗不匹配".into());
    }
    let phase = match native.status.as_str() {
        "New" => StockRfqPhase::AwaitingQuotes,
        "Filled" => StockRfqPhase::Filled,
        "Cancelled" => StockRfqPhase::Cancelled,
        "Expired" => StockRfqPhase::Expired,
        _ => return Err("RFQ 返回未支持的状态，需人工核对".into()),
    };
    if !terminal_receipt(record, phase)? {
        return Ok(false);
    }
    if record.phase == StockRfqPhase::AcceptedBinding && phase == StockRfqPhase::AwaitingQuotes {
        return Ok(false);
    }
    if record
        .submission_time_ms
        .is_some_and(|t| native.submission_time < t)
    {
        return Ok(false);
    }
    let before = record.clone();
    // Refreshing an RFQ replaces the quote window, so an old candidate cannot inherit it.
    if record
        .submission_time_ms
        .is_some_and(|t| t != native.submission_time)
        || record
            .expiry_time_ms
            .is_some_and(|t| t != native.expiry_time)
    {
        record.candidate = None;
        if record.phase == StockRfqPhase::Candidate {
            record.phase = StockRfqPhase::AwaitingQuotes;
        }
    }
    record.rfq_id = Some(remote);
    record.submission_time_ms = Some(native.submission_time);
    record.expiry_time_ms = Some(native.expiry_time);
    // New does not distinguish a pending RFQ from a binding accepted broker quote.
    if phase.terminal() || matches!(record.phase, StockRfqPhase::SubmissionUnknown) {
        record.phase = phase;
    }
    if phase.terminal() {
        record.candidate = None;
        record.needs_recheck = false;
    }
    for value in [&native.executed_quantity, &native.executed_quote_quantity]
        .into_iter()
        .flatten()
    {
        if value
            .parse::<rust_decimal::Decimal>()
            .ok()
            .is_none_or(|v| v.is_sign_negative())
        {
            return Err("RFQ 实际成交数量无效".into());
        }
    }
    if native
        .executed_quantity
        .as_deref()
        .and_then(|q| q.parse::<rust_decimal::Decimal>().ok())
        .is_some_and(|q| q > positive(&record.request.quantity).unwrap_or_default())
    {
        return Err("RFQ 成交股数超过原请求".into());
    }
    for (old, new) in [
        (&record.executed_quantity, &native.executed_quantity),
        (
            &record.executed_quote_quantity,
            &native.executed_quote_quantity,
        ),
    ] {
        if let (Some(old), Some(new)) = (
            old.as_deref().and_then(positive),
            new.as_deref()
                .and_then(|s| s.parse::<rust_decimal::Decimal>().ok()),
        ) {
            if new < old {
                return Err("RFQ 累计成交回执倒退，保留已核实数值".into());
            }
        }
    }
    if native.executed_quantity.is_some() {
        record.executed_quantity = native.executed_quantity;
    }
    if native.executed_quote_quantity.is_some() {
        record.executed_quote_quantity = native.executed_quote_quantity;
    }
    if record.phase == StockRfqPhase::Filled {
        record.needs_recheck = record.settlement_pending();
    } else if !record.phase.terminal() {
        record.needs_recheck = record.candidate.is_none();
    }
    record.updated_at_ms = now;
    record.problem = if record.settlement_pending() {
        Some("交易所已报告成交，实际股数与成交金额仍待核对".into())
    } else if record.acceptance.is_some() && record.needs_recheck {
        Some("已核对原 RFQ；接受与结算结果仍待确认，不重复接受".into())
    } else if record.needs_recheck {
        Some("已核对原 RFQ，等待新的私有 WS 报价；不会重发询价".into())
    } else {
        None
    };
    Ok(before != *record)
}

pub(super) fn acknowledgement(bytes: &[u8]) -> Result<NativeRfq, String> {
    serde_json::from_slice(bytes).map_err(|_| "Backpack RFQ 回执结构无效".into())
}

pub(super) fn open_records(bytes: &[u8], record: &StockRfq) -> Result<Option<NativeRfq>, String> {
    #[derive(Deserialize)]
    struct Row {
        rfq: NativeRfq,
    }
    let rows: Vec<Row> = serde_json::from_slice(bytes).map_err(|_| "Backpack RFQ 查询结构无效")?;
    if rows.len() > 256 {
        return Err("Backpack RFQ 查询条数过大".into());
    }
    let mut selected = rows.into_iter().map(|r| r.rfq).filter(|r| {
        r.client_id == Some(record.client_id)
            && r.symbol == record.symbol
            && r.side == record.request.side
    });
    let row = selected.next();
    if selected.next().is_some() {
        return Err("同一 clientId 出现多个 RFQ，停止自动选择".into());
    }
    Ok(row)
}

pub(super) fn frame_id(text: &str) -> Result<Option<(String, Value)>, String> {
    let envelope: Value =
        serde_json::from_str(text).map_err(|_| "Backpack 私有 WS 消息不是 JSON")?;
    if envelope.get("error").is_some_and(|v| !v.is_null())
        || envelope
            .get("code")
            .is_some_and(|c| !c.is_null() && c.as_u64() != Some(0))
    {
        return Err("Backpack 私有订阅被拒绝，请检查 API 凭证权限".into());
    }
    let frame = if let Some(data) = envelope.get("data") {
        let stream = envelope["stream"].as_str().unwrap_or_default();
        if stream != "account.rfqUpdate" && !stream.starts_with("account.rfqUpdate.") {
            return Ok(None);
        }
        data.clone()
    } else {
        envelope
    };
    let event = frame["e"].as_str().unwrap_or_default();
    if !matches!(
        event,
        "rfqAccepted"
            | "rfqRefreshed"
            | "rfqCandidate"
            | "rfqAcceptedBinding"
            | "rfqFilled"
            | "rfqCancelled"
    ) {
        return Ok(None);
    }
    Ok(Some((
        id(&frame["R"]).ok_or("私有 RFQ 事件 ID 无效")?,
        frame,
    )))
}

pub(super) fn apply_event(record: &mut StockRfq, frame: &Value, now: i64) -> Result<bool, String> {
    if frame["s"].as_str() != Some(&record.symbol)
        || frame.get("S").is_some_and(|s| {
            serde_json::from_value::<StockRfqSide>(s.clone()).ok() != Some(record.request.side)
        })
    {
        return Err("RFQ 私有事件市场或方向不匹配".into());
    }
    if let Some(client) = frame.get("C").filter(|c| !c.is_null()) {
        if client
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .or_else(|| client.as_u64().and_then(|n| u32::try_from(n).ok()))
            != Some(record.client_id)
        {
            return Err("RFQ 私有事件 clientId 不匹配".into());
        }
    }
    if frame
        .get("q")
        .filter(|q| !q.is_null())
        .is_some_and(|q| q.as_str().and_then(positive) != positive(&record.request.quantity))
    {
        return Err("RFQ 私有事件股数不匹配".into());
    }
    let time = frame["T"]
        .as_i64()
        .filter(|t| {
            *t / 1000 >= record.created_at_ms.saturating_sub(3000) && *t / 1000 <= now + 3000
        })
        .ok_or("RFQ 引擎时间无效")?;
    if record.source_at_us.is_some_and(|t| time < t) {
        return Ok(false);
    }
    let event = frame["e"].as_str().ok_or("RFQ 事件类型缺失")?;
    if let Some(a) = &record.acceptance {
        if matches!(event, "rfqCandidate" | "rfqAccepted" | "rfqRefreshed") {
            return Ok(false);
        }
        if time / 1000 < a.submitted_at_ms.saturating_sub(3000) {
            return Ok(false);
        }
        if matches!(event, "rfqAcceptedBinding" | "rfqFilled") {
            let price = frame["p"]
                .as_str()
                .and_then(positive)
                .ok_or("RFQ 回执缺 taker 价格")?;
            let limit = positive(&a.taker_price).ok_or("原 RFQ 价格无效")?;
            if id(&frame["u"]).as_deref() != Some(&a.quote_id)
                || (record.request.side == StockRfqSide::Bid && price > limit)
                || (record.request.side == StockRfqSide::Ask && price < limit)
            {
                return Err("RFQ 回执报价编号或成交价格与已接受计划不一致".into());
            }
        }
    }
    let terminal = match event {
        "rfqFilled" if frame["X"].as_str() == Some("Filled") => Some(StockRfqPhase::Filled),
        "rfqFilled" => return Err("RFQ 成交事件状态不一致".into()),
        "rfqCancelled" => Some(match frame["X"].as_str() {
            Some("Expired") => StockRfqPhase::Expired,
            Some("Cancelled") => StockRfqPhase::Cancelled,
            _ => return Err("RFQ 取消事件状态未知".into()),
        }),
        "rfqAcceptedBinding" => Some(StockRfqPhase::AcceptedBinding),
        _ => None,
    };
    if let Some(next) = terminal {
        if !terminal_receipt(record, next)? {
            return Ok(false);
        }
    }
    if record.phase.terminal() {
        if event == "rfqFilled" {
            let price = frame["p"]
                .as_str()
                .and_then(positive)
                .ok_or("RFQ 回执缺 taker 价格")?;
            if let Some(old) = &record.fill_price {
                if positive(old) != Some(price) {
                    return Err("同一 RFQ 出现不同的最终 taker 成交价，保留原回执待核对".into());
                }
            } else {
                // REST may establish Filled first; retain the later native WS taker price once.
                record.fill_price = Some(price.normalize().to_string());
                record.source_at_us = Some(time);
                record.updated_at_ms = now.max(record.updated_at_ms);
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let before = record.clone();
    match event {
        "rfqAccepted" | "rfqRefreshed" => {
            if record.phase == StockRfqPhase::AcceptedBinding {
                return Ok(false);
            }
            let start = frame["w"].as_i64().ok_or("RFQ 缺报价截止时间")?;
            let end = frame["W"]
                .as_i64()
                .filter(|end| *end > start)
                .ok_or("RFQ 缺有效到期时间")?;
            if record.source_at_us == Some(time) && record.submission_time_ms == Some(start) {
                return Ok(false);
            }
            record.submission_time_ms = Some(start);
            record.expiry_time_ms = Some(end);
            record.candidate = None;
            record.phase = StockRfqPhase::AwaitingQuotes;
        }
        "rfqCandidate" => {
            if record.phase == StockRfqPhase::AcceptedBinding || record.cancel_requested {
                return Ok(false);
            }
            let quote_id = id(&frame["u"]).ok_or("RFQ 候选缺报价 ID")?;
            let price = frame["p"]
                .as_str()
                .and_then(positive)
                .ok_or("RFQ 候选缺 taker 价格")?;
            if frame["X"].as_str() != Some("New") {
                return Err("RFQ 候选状态无效".into());
            }
            if record
                .candidate
                .as_ref()
                .is_some_and(|c| c.quote_id == quote_id && c.source_at_us == time)
            {
                return Ok(false);
            }
            record.candidate = Some(StockRfqCandidate {
                quote_id,
                taker_price: price.normalize().to_string(),
                source_at_us: time,
                received_at_ms: now,
            });
            record.phase = StockRfqPhase::Candidate;
        }
        "rfqAcceptedBinding" => {
            if frame["X"].as_str() != Some("New") {
                return Err("RFQ 绑定事件状态不一致".into());
            }
            record.phase = StockRfqPhase::AcceptedBinding;
            record.candidate = None;
        }
        "rfqFilled" => {
            if frame["X"].as_str() != Some("Filled") {
                return Err("RFQ 成交事件状态不一致".into());
            }
            record.phase = StockRfqPhase::Filled;
            record.candidate = None;
            record.fill_price = frame["p"]
                .as_str()
                .and_then(positive)
                .map(|p| p.normalize().to_string());
            // The WS example omits actual filled amounts. Never substitute the requested amount.
        }
        "rfqCancelled" => {
            record.phase = match frame["X"].as_str() {
                Some("Expired") => StockRfqPhase::Expired,
                Some("Cancelled") => StockRfqPhase::Cancelled,
                _ => return Err("RFQ 取消事件状态未知".into()),
            };
            record.candidate = None;
        }
        _ => return Ok(false),
    }
    record.source_at_us = Some(time);
    record.updated_at_ms = now;
    record.needs_recheck = record.settlement_pending()
        || (record.phase == StockRfqPhase::Candidate && record.expiry_time_ms.is_none());
    record.problem = if record.settlement_pending() {
        Some("交易所已报告成交，实际股数与成交金额仍待核对".into())
    } else {
        record
            .needs_recheck
            .then(|| "报价已收到，等待原 RFQ 时窗核对".into())
    };
    Ok(before != *record)
}
