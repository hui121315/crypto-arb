//! 执行状态条的纯文案格式化：腿标签/成交/费用/证据、来源/时间、金额与成本分项文案。
//! 运行态派生见 `state.rs`，视图组件见父模块 `execution_status_bar.rs`。

use shared_types::{
    ExecutionFillConfidence, ExecutionLedgerEventType, ExecutionRunEventKind, ExecutionRunLeg,
    ExecutionRunLegEvidence, ExecutionRunTimelineEvent, OrderUpdateSource,
};

use crate::panels::modules::timestamp::local_date_hm;

pub(super) fn leg_label(leg: &ExecutionRunLeg) -> String {
    match leg.role {
        shared_types::HedgeLegRole::Long => format!("多腿 {}", leg.exchange),
        shared_types::HedgeLegRole::Short => format!("空腿 {}", leg.exchange),
    }
}

pub(super) fn fill_text(leg: &ExecutionRunLeg) -> String {
    match (leg.filled_quantity, leg.filled_notional_usd) {
        (Some(quantity), Some(notional)) => format!("{quantity:.6} / {}", money(notional)),
        (Some(quantity), None) => format!("{quantity:.6} / 待名义"),
        _ => "待成交".into(),
    }
}

fn fee_text(value: Option<f64>) -> String {
    value
        .map(|value| format!("fee {}", money(value)))
        .unwrap_or_else(|| "fee -".into())
}

pub(super) fn leg_evidence_text(
    leg: &ExecutionRunLeg,
    evidence: &ExecutionRunLegEvidence,
) -> String {
    let mut parts = Vec::new();
    if let Some(plan) = evidence.compile_plan.as_ref() {
        let native_symbol = plan
            .instrument_spec
            .as_ref()
            .map(|instrument| instrument.native_symbol.as_str())
            .unwrap_or(&plan.symbol);
        parts.push(format!("native {native_symbol}"));
        if let Some(instrument) = plan.instrument_spec.as_ref() {
            let mut precision = Vec::new();
            if let Some(value) = instrument.price_tick {
                precision.push(format!("tick {value}"));
            }
            if let Some(value) = instrument.qty_step {
                precision.push(format!("step {value}"));
            }
            if let Some(value) = instrument.contract_size {
                precision.push(format!("contract {value}"));
            }
            if !precision.is_empty() {
                parts.push(format!("精度 {}", precision.join(" / ")));
            }
        }
        if !plan.venue_capability.source.is_empty() {
            parts.push(format!("能力 {}", plan.venue_capability.source));
        }
    }
    if let Some(source) = leg.finality_source {
        parts.push(format!("终态 {}", order_update_source_label(source)));
    }
    if evidence.finality_confidence != ExecutionFillConfidence::Unknown {
        parts.push(format!(
            "置信 {}",
            fill_confidence_label(evidence.finality_confidence)
        ));
    }
    if let Some(confirmed_at) = leg.confirmed_filled_at_ms {
        parts.push(format!("确认 {}", historical_time_label(confirmed_at)));
    }
    parts.push(fee_text(leg.filled_fee));
    parts.join(" · ")
}

pub(super) fn timeline_event_title(event: &ExecutionRunTimelineEvent) -> String {
    let role = event
        .leg_role
        .map(|role| match role {
            shared_types::HedgeLegRole::Long => "多腿",
            shared_types::HedgeLegRole::Short => "空腿",
        })
        .unwrap_or_default();
    if role.is_empty() {
        event_kind_label(event.kind).to_owned()
    } else {
        format!("{role} {}", event_kind_label(event.kind))
    }
}

pub(super) fn timeline_event_meta(event: &ExecutionRunTimelineEvent) -> String {
    let mut parts = vec![
        historical_time_label(event.occurred_at_ms),
        order_update_source_label(event.source).to_owned(),
    ];
    if let Some(event_type) = event.ledger_event_type {
        parts.push(ledger_event_type_label(event_type).to_owned());
    }
    if event.finality_confidence != ExecutionFillConfidence::Unknown {
        parts.push(format!(
            "置信 {}",
            fill_confidence_label(event.finality_confidence)
        ));
    }
    if let Some(identity) = event.order_identity.as_ref() {
        let order_id = identity
            .exchange_order_id
            .as_deref()
            .unwrap_or(&identity.public_client_order_id);
        if !order_id.is_empty() {
            parts.push(format!("订单 {order_id}"));
        }
    }
    if let Some(request_id) = event.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    parts.join(" · ")
}

pub(super) fn timeline_event_class(event: &ExecutionRunTimelineEvent) -> &'static str {
    match event.kind {
        ExecutionRunEventKind::Failure => "execution-timeline-row danger",
        ExecutionRunEventKind::Unwind | ExecutionRunEventKind::Cancel => {
            "execution-timeline-row warn"
        }
        ExecutionRunEventKind::Fill | ExecutionRunEventKind::Closed => {
            "execution-timeline-row done"
        }
        _ => "execution-timeline-row",
    }
}

fn order_update_source_label(source: OrderUpdateSource) -> &'static str {
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

fn event_kind_label(kind: ExecutionRunEventKind) -> &'static str {
    match kind {
        ExecutionRunEventKind::Preview => "预览",
        ExecutionRunEventKind::Preflight => "预检",
        ExecutionRunEventKind::Submit => "提交",
        ExecutionRunEventKind::OrderUpdate => "订单更新",
        ExecutionRunEventKind::Fill => "成交",
        ExecutionRunEventKind::Cancel => "撤单",
        ExecutionRunEventKind::Funding => "资金费",
        ExecutionRunEventKind::Unwind => "补救",
        ExecutionRunEventKind::Reconcile => "终态回查",
        ExecutionRunEventKind::Failure => "失败",
        ExecutionRunEventKind::Closed => "关闭",
    }
}

fn ledger_event_type_label(event_type: ExecutionLedgerEventType) -> &'static str {
    match event_type {
        ExecutionLedgerEventType::OrderState => "订单状态",
        ExecutionLedgerEventType::FillSnapshot => "成交快照",
        ExecutionLedgerEventType::FillEvent => "成交事件",
        ExecutionLedgerEventType::FeeSnapshot => "费用快照",
        ExecutionLedgerEventType::FundingPayment => "资金费事件",
        ExecutionLedgerEventType::Slippage => "滑点事件",
        ExecutionLedgerEventType::OrderbookEvidence => "盘口证据",
        ExecutionLedgerEventType::Cancel => "撤单事件",
    }
}

fn fill_confidence_label(confidence: ExecutionFillConfidence) -> &'static str {
    match confidence {
        ExecutionFillConfidence::VenueFill => "交易所成交",
        ExecutionFillConfidence::VenueOrderSnapshot => "交易所快照",
        ExecutionFillConfidence::OrderQuery => "订单查询",
        ExecutionFillConfidence::AdapterAck => "ACK",
        ExecutionFillConfidence::Manual => "人工",
        ExecutionFillConfidence::Unknown => "未知",
    }
}

pub(super) fn time_label(ms: i64) -> String {
    if ms <= 0 {
        return "--:--".into();
    }
    format_time_label(ms)
}

fn historical_time_label(ms: i64) -> String {
    if ms <= 0 {
        return "日期时间未知".into();
    }
    local_date_hm(ms).unwrap_or_else(|| "日期时间未知".into())
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

pub(super) fn open_actual_cost_text(value: Option<f64>) -> String {
    value
        .map(|value| format!("开仓真实 {}", money(value)))
        .unwrap_or_else(|| "开仓真实待成交回报".into())
}

pub(super) fn actual_cost_text(value: Option<f64>, is_closed: bool) -> String {
    value
        .map(|value| format!("真实总成本 {}", money(value)))
        .unwrap_or_else(|| {
            if is_closed {
                "真实总成本见复盘".into()
            } else {
                "真实总成本待平仓事实源".into()
            }
        })
}

pub(super) fn funding_actual_text(
    value: Option<f64>,
    event_count: usize,
    is_closed: bool,
) -> String {
    value.map_or_else(
        || {
            if is_closed {
                "资金费见复盘".into()
            } else {
                "资金费待账本".into()
            }
        },
        |value| {
            if event_count == 0 {
                format!("资金费 {}", precise_money(value))
            } else {
                format!("资金费 {} · {} 条事件", precise_money(value), event_count)
            }
        },
    )
}

pub(super) fn delta_cost_text(value: Option<f64>, is_closed: bool) -> String {
    value
        .map(|value| format!("差异 {}", money(value)))
        .unwrap_or_else(|| {
            if is_closed {
                "成本差异见复盘".into()
            } else {
                "差异待平仓".into()
            }
        })
}

fn precise_money(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let abs = value.abs();
    if abs < 10.0 {
        format!("{sign}${abs:.2}")
    } else {
        money(value)
    }
}

pub(super) fn money(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let value = value.abs();
    if value >= 1_000_000.0 {
        format!("{sign}${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{sign}${:.1}K", value / 1_000.0)
    } else {
        format!("{sign}${value:.0}")
    }
}
