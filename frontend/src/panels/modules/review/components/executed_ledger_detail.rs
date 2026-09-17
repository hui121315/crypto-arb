use shared_types::{
    CloseRunCostReconciliation, CloseRunStatus, CloseRunUnwindPlanStatus, ExecutedTrade,
    ExecutionFillConfidence, ExecutionLedgerEventType, ExecutionLedgerQuality, OrderUpdateSource,
    ReviewCloseRunEvidence, ReviewLedgerEventEvidence, ReviewLedgerPayloadEvidence,
};

use super::format::{fill_confidence_label, order_update_source_label};

pub(in crate::panels::modules::review) fn ledger_event_drilldown_summary(
    row: &ExecutedTrade,
) -> String {
    let events = &row.evidence.ledger_events;
    let close_runs = close_run_drilldown_summary(row);
    if events.is_empty() {
        return close_runs.unwrap_or_else(|| "明细无 payload".to_owned());
    }
    let sources = ledger_event_sources(events);
    let mut samples = events
        .iter()
        .take(3)
        .map(ledger_event_summary)
        .collect::<Vec<_>>();
    if events.len() > samples.len() {
        samples.push(format!("+{} 条", events.len() - samples.len()));
    }
    let ledger = format!(
        "明细 {} 条 via {} · {}",
        events.len(),
        sources,
        samples.join("; ")
    );
    if let Some(close_runs) = close_runs {
        format!("{ledger} · {close_runs}")
    } else {
        ledger
    }
}

fn ledger_event_sources(events: &[ReviewLedgerEventEvidence]) -> String {
    let mut sources = Vec::<OrderUpdateSource>::new();
    for event in events {
        if !sources.contains(&event.source) {
            sources.push(event.source);
        }
    }
    sources
        .into_iter()
        .map(order_update_source_label)
        .collect::<Vec<_>>()
        .join("/")
}

fn ledger_event_summary(event: &ReviewLedgerEventEvidence) -> String {
    format!(
        "{} {} {} run:{} ticket:{} via {} {}",
        event.event_id,
        ledger_event_type_label(event.event_type),
        event.order.exchange,
        event.order.run_id.as_deref().unwrap_or("-"),
        event.order.ticket_id.as_deref().unwrap_or("-"),
        order_update_source_label(event.source),
        payload_summary(&event.payload)
    )
}

fn ledger_event_type_label(event_type: ExecutionLedgerEventType) -> &'static str {
    match event_type {
        ExecutionLedgerEventType::OrderState => "state",
        ExecutionLedgerEventType::FillSnapshot | ExecutionLedgerEventType::FillEvent => "fill",
        ExecutionLedgerEventType::FeeSnapshot => "fee",
        ExecutionLedgerEventType::FundingPayment => "funding",
        ExecutionLedgerEventType::Slippage => "slip",
        ExecutionLedgerEventType::OrderbookEvidence => "book",
        ExecutionLedgerEventType::Cancel => "cancel",
    }
}

pub(super) fn payload_summary(payload: &ReviewLedgerPayloadEvidence) -> String {
    match payload {
        ReviewLedgerPayloadEvidence::Fill {
            quantity,
            average_price,
            quality,
            confidence,
            ..
        } => fill_payload_summary(*quantity, *average_price, *quality, *confidence),
        ReviewLedgerPayloadEvidence::Fee {
            amount,
            currency,
            quality,
        } => format!(
            "费用 {} {} · {}",
            compact_number(*amount),
            currency.as_deref().unwrap_or(""),
            quality_label(*quality)
        ),
        ReviewLedgerPayloadEvidence::Funding {
            amount,
            currency,
            quality,
            ..
        } => format!(
            "Funding {} {} · {}",
            compact_number(*amount),
            currency,
            quality_label(*quality)
        ),
        ReviewLedgerPayloadEvidence::Slippage {
            amount_usd,
            reference_price,
            fill_price,
            quality,
            ..
        } => format!(
            "滑点 ${} · 参考价 {} · 成交价 {} · {}",
            compact_number(*amount_usd),
            compact_number(*reference_price),
            compact_number(*fill_price),
            quality_label(*quality)
        ),
        ReviewLedgerPayloadEvidence::Orderbook {
            depth_usd_20bps,
            max_notional_usd,
            mid,
            quality,
            ..
        } => format!(
            "20bps 深度 {} · 最大金额 {} · 中间价 {} · {}",
            optional_money(*depth_usd_20bps),
            optional_money(*max_notional_usd),
            optional_number(*mid),
            quality_label(*quality)
        ),
    }
}

fn fill_payload_summary(
    quantity: f64,
    average_price: f64,
    quality: ExecutionLedgerQuality,
    confidence: ExecutionFillConfidence,
) -> String {
    format!(
        "数量 {} · 成交价 {} · {} · {}",
        compact_number(quantity),
        compact_number(average_price),
        quality_label(quality),
        fill_confidence_label(confidence)
    )
}

fn quality_label(quality: ExecutionLedgerQuality) -> &'static str {
    match quality {
        ExecutionLedgerQuality::Actual => "已确认",
        ExecutionLedgerQuality::Estimated => "估算",
        ExecutionLedgerQuality::Missing => "缺证据",
    }
}

fn optional_money(value: Option<f64>) -> String {
    value
        .map(|value| format!("${}", compact_number(value)))
        .unwrap_or_else(|| "-".to_owned())
}

fn optional_number(value: Option<f64>) -> String {
    value.map(compact_number).unwrap_or_else(|| "-".to_owned())
}

fn compact_number(value: f64) -> String {
    if (value.fract()).abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.4}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn close_run_drilldown_summary(row: &ExecutedTrade) -> Option<String> {
    let evidence = &row.evidence.close_run_evidence;
    if evidence.is_empty() {
        return None;
    }
    let mut samples = evidence
        .iter()
        .take(2)
        .map(close_run_summary)
        .collect::<Vec<_>>();
    if evidence.len() > samples.len() {
        samples.push(format!("+{} 条", evidence.len() - samples.len()));
    }
    Some(format!(
        "CloseRun {} 条 · {}",
        evidence.len(),
        samples.join("; ")
    ))
}

fn close_run_summary(evidence: &ReviewCloseRunEvidence) -> String {
    let cost = evidence
        .cost_reconciliation
        .as_ref()
        .map(cost_reconciliation_summary)
        .unwrap_or_else(|| "cost:unproven".to_owned());
    format!(
        "{} {} run:{} ticket:{} unwind:{} comp:{} {}",
        evidence.close_run_id,
        close_run_status_label(evidence.status),
        evidence.run_id,
        evidence.ticket_id,
        evidence
            .unwind_status
            .map(close_run_unwind_status_label)
            .unwrap_or("-"),
        evidence.compensation_attempt_count,
        cost,
    )
}

fn cost_reconciliation_summary(cost: &CloseRunCostReconciliation) -> String {
    let missing = if cost.missing_fields.is_empty() {
        "none".to_owned()
    } else {
        cost.missing_fields.join(",")
    };
    format!(
        "cost:{} [close_fee:{} close_slip:{} comp_fee:{} comp_slip:{} funding:{} manual:{} total:{} missing:{}]",
        cost.evidence_event_ids.len(),
        cost_component(cost.close_fee_usd, &cost.close_fee_event_ids),
        cost_component(cost.close_slippage_usd, &cost.close_slippage_event_ids),
        cost_component(
            cost.compensation_fee_usd,
            &cost.compensation_fee_event_ids,
        ),
        cost_component(
            cost.compensation_slippage_usd,
            &cost.compensation_slippage_event_ids,
        ),
        cost_component(cost.funding_usd, &cost.funding_event_ids),
        cost_component(cost.manual_handling_usd, &cost.manual_handling_event_ids),
        optional_cost(cost.total_actual_cost_usd),
        missing,
    )
}

fn cost_component(value: Option<f64>, event_ids: &[String]) -> String {
    format!("{}/{}", optional_cost(value), event_ids.len())
}

fn optional_cost(value: Option<f64>) -> String {
    value
        .map(|value| format!("${}", compact_number(value)))
        .unwrap_or_else(|| "-".to_owned())
}

fn close_run_status_label(status: CloseRunStatus) -> &'static str {
    match status {
        CloseRunStatus::Submitted => "submitted",
        CloseRunStatus::Succeeded => "succeeded",
        CloseRunStatus::PartiallySubmitted => "partial",
        CloseRunStatus::UnwindRequired => "unwind_required",
        CloseRunStatus::CompensationSubmitted => "compensation_submitted",
        CloseRunStatus::Compensated => "compensated",
        CloseRunStatus::CompensationFailed => "compensation_failed",
        CloseRunStatus::ManuallyResolved => "manual_resolved",
        CloseRunStatus::Failed => "failed",
    }
}

fn close_run_unwind_status_label(status: CloseRunUnwindPlanStatus) -> &'static str {
    match status {
        CloseRunUnwindPlanStatus::BlockedPendingManualRecheck => "blocked_manual_recheck",
        CloseRunUnwindPlanStatus::CompensationSubmitted => "compensation_submitted",
        CloseRunUnwindPlanStatus::Compensated => "compensated",
        CloseRunUnwindPlanStatus::CompensationFailed => "compensation_failed",
        CloseRunUnwindPlanStatus::ManualTerminalRecorded => "manual_terminal",
    }
}

#[cfg(test)]
#[path = "executed_ledger_detail_tests.rs"]
mod tests;
