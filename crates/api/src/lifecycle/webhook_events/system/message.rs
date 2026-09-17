use shared_types::{RiskStatusSlot, SystemHealth, WebhookEventKind};
use std::collections::BTreeMap;

pub(super) fn system_payload(health: &SystemHealth, kind: WebhookEventKind) -> serde_json::Value {
    let mut payload = serde_json::to_value(health).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "message".to_owned(),
            serde_json::Value::String(system_message(health, kind)),
        );
    }
    payload
}

fn system_message(health: &SystemHealth, kind: WebhookEventKind) -> String {
    let mut lines = Vec::with_capacity(5);
    match kind {
        WebhookEventKind::RiskAlert => {
            lines.push(format!("风险 {}", risk_label(health.risk)));
            lines.push(format!(
                "净 Delta {}（{:+.1}% NAV）",
                signed_usd(health.net_delta_usd),
                health.net_delta_pct_of_nav
            ));
        }
        WebhookEventKind::SystemDegradation => lines.push("系统 DEGRADED".to_owned()),
        _ => lines.push("系统状态变化".to_owned()),
    }
    if let Some(funding) = health.next_funding.as_ref() {
        lines.push(format!(
            "资金费率 {}@{} · {}m · 预估流出 {}",
            funding.symbol,
            funding.venue,
            funding.minutes_to_settle,
            unsigned_usd(funding.estimated_outflow_usd)
        ));
    }
    let disconnected = u32::try_from(health.ws.disconnected.len()).unwrap_or(u32::MAX);
    let healthy_ws = health.ws.channels.saturating_sub(disconnected);
    lines.push(format!(
        "运行健康 API {}/{} · WS {}/{}",
        health.api.healthy, health.api.total, healthy_ws, health.ws.channels
    ));
    if !health.problems.is_empty() {
        lines.push(format!(
            "问题 {} 项 · {}",
            health.problems.len(),
            problem_code_summary(health)
        ));
    }
    lines.join("\n")
}

const fn risk_label(risk: RiskStatusSlot) -> &'static str {
    match risk {
        RiskStatusSlot::Ok => "OK",
        RiskStatusSlot::Warn => "WARN",
        RiskStatusSlot::Block => "BLOCK",
    }
}

fn signed_usd(value: f64) -> String {
    if !value.is_finite() {
        return "未知".to_owned();
    }
    if value >= 0.0 {
        format!("+${value:.2}")
    } else {
        format!("-${:.2}", value.abs())
    }
}

fn unsigned_usd(value: f64) -> String {
    if value.is_finite() {
        format!("${:.2}", value.abs())
    } else {
        "未知".to_owned()
    }
}

fn problem_code_summary(health: &SystemHealth) -> String {
    let mut counts = BTreeMap::<&str, usize>::new();
    for problem in &health.problems {
        *counts.entry(problem.code.as_str()).or_default() += 1;
    }
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|(left_code, left_count), (right_code, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_code.cmp(right_code))
    });
    counts
        .into_iter()
        .take(3)
        .map(|(code, count)| format!("{code} x{count}"))
        .collect::<Vec<_>>()
        .join(" · ")
}
