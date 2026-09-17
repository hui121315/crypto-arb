//! `CloseRun` 成本对账的显示文本派生。

use shared_types::{
    CloseRunCostReconciliation, CloseRunUnwindLegEvidence, ExecutionLedgerQuality, OrderSide,
    OrderUpdateSource,
};

use super::super::super::format::money;

pub(in crate::panels::modules::positions::components) fn close_run_cost_label(
    cost: Option<&CloseRunCostReconciliation>,
) -> String {
    cost.and_then(|summary| summary.total_actual_cost_usd)
        .map(|value| format!("总成本 {}", money(value)))
        .unwrap_or_else(|| "成本待证据".to_owned())
}

pub(in crate::panels::modules::positions::components) fn close_run_cost_detail(
    cost: Option<&CloseRunCostReconciliation>,
) -> String {
    let Some(summary) = cost else {
        return "等待费用 / 滑点回放".to_owned();
    };
    let mut parts = Vec::new();
    push_cost_part(&mut parts, "平仓费", summary.close_fee_usd);
    push_cost_part(&mut parts, "平仓滑点", summary.close_slippage_usd);
    push_cost_part(&mut parts, "补偿费", summary.compensation_fee_usd);
    push_cost_part(&mut parts, "补偿滑点", summary.compensation_slippage_usd);
    push_cost_part(&mut parts, "资金费", summary.funding_usd);
    push_cost_part(&mut parts, "人工处理", summary.manual_handling_usd);
    if parts.is_empty() {
        "等待费用 / 滑点回放".to_owned()
    } else {
        parts.join(" · ")
    }
}

pub(in crate::panels::modules::positions::components) fn close_run_cost_title(
    cost: Option<&CloseRunCostReconciliation>,
) -> String {
    let Some(summary) = cost else {
        return "CloseRun 成本对账未生成".to_owned();
    };
    let mut parts = Vec::new();
    if !summary.evidence_order_ids.is_empty() {
        parts.push(format!("orders {}", summary.evidence_order_ids.join(",")));
    }
    push_event_part(&mut parts, "events", &summary.evidence_event_ids);
    push_event_part(&mut parts, "平仓费事件", &summary.close_fee_event_ids);
    push_event_part(
        &mut parts,
        "平仓滑点事件",
        &summary.close_slippage_event_ids,
    );
    push_event_part(
        &mut parts,
        "补偿费事件",
        &summary.compensation_fee_event_ids,
    );
    push_event_part(
        &mut parts,
        "补偿滑点事件",
        &summary.compensation_slippage_event_ids,
    );
    push_event_part(&mut parts, "资金费事件", &summary.funding_event_ids);
    push_event_part(
        &mut parts,
        "人工处理事件",
        &summary.manual_handling_event_ids,
    );
    if !summary.missing_fields.is_empty() {
        parts.push(format!("缺证据 {}", summary.missing_fields.join(",")));
    }
    if parts.is_empty() {
        "成本证据完整".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn push_event_part(parts: &mut Vec<String>, label: &str, ids: &[String]) {
    if !ids.is_empty() {
        parts.push(format!("{label} {}", ids.join(",")));
    }
}

fn push_cost_part(parts: &mut Vec<String>, label: &str, value: Option<f64>) {
    if let Some(value) = value {
        parts.push(format!("{label} {}", money(value)));
    }
}

pub(in crate::panels::modules::positions::components) fn close_candidate_label(
    index: usize,
    candidate: &CloseRunUnwindLegEvidence,
) -> String {
    format!(
        "#{} {} {} {} @ {}",
        index + 1,
        candidate.venue,
        candidate.symbol,
        order_side_label(candidate.compensation_order_side),
        format_candidate_qty(candidate)
    )
}

pub(in crate::panels::modules::positions::components) fn close_candidate_title(
    candidate: &CloseRunUnwindLegEvidence,
) -> String {
    let mut parts = vec![format!(
        "{} {} · price {:.6} · {}",
        candidate.venue,
        candidate.symbol,
        candidate.mark_price,
        close_candidate_notional_evidence(candidate)
    )];
    if let Some(order_id) = candidate.order_id.as_deref() {
        parts.push(format!("order {order_id}"));
    }
    if let Some(client_order_id) = candidate.client_order_id.as_deref() {
        parts.push(format!("client {client_order_id}"));
    }
    if let Some(exchange_order_id) = candidate.exchange_order_id.as_deref() {
        parts.push(format!("exchange {exchange_order_id}"));
    }
    parts.join(" · ")
}

pub(in crate::panels::modules::positions::components) fn close_candidate_evidence(
    candidate: &CloseRunUnwindLegEvidence,
) -> String {
    let mut parts = vec![close_candidate_notional_evidence(candidate)];
    if let Some(source) = candidate.finality_source {
        parts.push(format!("终态 {}", finality_source_label(source)));
    }
    if let Some(confirmed_at) = candidate.confirmed_filled_at_ms {
        parts.push(format!("确认 {}", time_label(confirmed_at)));
    }
    parts.join(" · ")
}

pub(super) fn close_candidate_notional_evidence(candidate: &CloseRunUnwindLegEvidence) -> String {
    let mut parts = vec![format!(
        "{} {}",
        notional_quality_label(candidate.notional_quality),
        money(candidate.notional_usd)
    )];
    if !candidate.notional_source.is_empty() {
        parts.push(format!("source {}", candidate.notional_source));
    }
    if !candidate.notional_missing_fields.is_empty() {
        parts.push(format!(
            "缺证据 {}",
            candidate.notional_missing_fields.join(",")
        ));
    }
    parts.join(" · ")
}

fn notional_quality_label(quality: ExecutionLedgerQuality) -> &'static str {
    match quality {
        ExecutionLedgerQuality::Actual => "名义 实际",
        ExecutionLedgerQuality::Estimated => "名义 估算",
        ExecutionLedgerQuality::Missing => "名义 缺证据",
    }
}

fn finality_source_label(source: OrderUpdateSource) -> &'static str {
    match source {
        OrderUpdateSource::Unknown => "未知",
        OrderUpdateSource::Internal => "内部",
        OrderUpdateSource::AdapterAck => "ACK",
        OrderUpdateSource::OrderQuery => "查询",
        OrderUpdateSource::PrivateWs => "私有WS",
        OrderUpdateSource::FundingPoller => "资金费轮询",
        OrderUpdateSource::Reconcile => "回查",
        OrderUpdateSource::Manual => "手动",
    }
}

pub(super) fn time_label(ms: i64) -> String {
    if ms <= 0 {
        return "--:--".into();
    }
    format_time_label(ms)
}

#[cfg(target_arch = "wasm32")]
fn format_time_label(ms: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

#[cfg(not(target_arch = "wasm32"))]
fn format_time_label(ms: i64) -> String {
    format!("{ms}ms")
}

pub(in crate::panels::modules::positions::components) fn compensation_button_label(
    candidate: &CloseRunUnwindLegEvidence,
) -> &'static str {
    match candidate.compensation_order_side {
        Some(OrderSide::Buy) => "补买",
        Some(OrderSide::Sell) => "补卖",
        None => "补偿",
    }
}

fn order_side_label(side: Option<OrderSide>) -> &'static str {
    match side {
        Some(OrderSide::Buy) => "买",
        Some(OrderSide::Sell) => "卖",
        None => "待方向",
    }
}

fn format_candidate_qty(candidate: &CloseRunUnwindLegEvidence) -> String {
    let qty = candidate
        .confirmed_quantity
        .unwrap_or(candidate.target_quantity)
        .abs();
    format!("{qty:.6}")
}
