use crate::panels::shared::execution_mode_label as product_execution_mode_label;
use shared_types::{
    ExecutionMode, FeeProduct, MarginMode, OrderType, TimeInForce, TradeFeeSnapshot, TradeFeeSource,
};

use super::model::PreviewFeeEvidence;

pub(super) fn fee_evidence_lines(snapshots: &[TradeFeeSnapshot]) -> Vec<PreviewFeeEvidence> {
    snapshots.iter().map(fee_evidence_line).collect()
}

fn fee_evidence_line(snapshot: &TradeFeeSnapshot) -> PreviewFeeEvidence {
    PreviewFeeEvidence {
        label: format!(
            "{} {} {}",
            snapshot.venue,
            snapshot.symbol,
            fee_product_label(snapshot.product)
        ),
        rate: format!(
            "开 {:.3}% / 平 {:.3}%",
            snapshot.open_fee_bps / 100.0,
            snapshot.close_fee_bps / 100.0
        ),
        source: fee_source_label(snapshot.source).into(),
        health: fee_health_label(snapshot),
    }
}

fn fee_source_label(source: TradeFeeSource) -> &'static str {
    match source {
        TradeFeeSource::AccountApi => "账户 API",
        TradeFeeSource::OfficialSchedule => "官方费率表",
        TradeFeeSource::Manual => "手工录入",
        TradeFeeSource::Unverified => "未验证",
    }
}

fn fee_product_label(product: FeeProduct) -> &'static str {
    match product {
        FeeProduct::Spot => "现货",
        FeeProduct::Perp => "永续",
        FeeProduct::Margin => "杠杆",
        FeeProduct::Unknown => "未知产品",
    }
}

fn fee_health_label(snapshot: &TradeFeeSnapshot) -> String {
    if let Some(problem) = snapshot.verification_problem.as_deref() {
        return format!("验证问题：{problem}");
    }
    if let Some(evidence) = snapshot.evidence.as_ref() {
        if let Some(problem) = evidence.problem.as_deref() {
            return format!("证据问题：{problem}");
        }
        return fee_evidence_health_label(snapshot, evidence);
    }
    match snapshot.source {
        TradeFeeSource::AccountApi if snapshot.fetched_at_ms > 0 => {
            fee_snapshot_health_label("账户接口读取", snapshot)
        }
        TradeFeeSource::OfficialSchedule => "缺官方证据".into(),
        TradeFeeSource::Manual => "仅观察，不作为实盘证据".into(),
        TradeFeeSource::Unverified => "未验证，不作为实盘证据".into(),
        TradeFeeSource::AccountApi => "账户接口时间缺失".into(),
    }
}

fn fee_evidence_health_label(
    snapshot: &TradeFeeSnapshot,
    evidence: &shared_types::TradeFeeEvidence,
) -> String {
    let mut parts = vec![
        format!("证据 {}", evidence.evidence_id),
        evidence.source_name.clone(),
        format!("checked_at_ms {}", evidence.checked_at_ms),
        evidence.source_url.clone(),
    ];
    push_optional_fee_part(&mut parts, "tier", evidence.tier.as_deref());
    push_optional_fee_part(&mut parts, "scope", evidence.scope.as_deref());
    push_optional_fee_part(&mut parts, "version", evidence.schedule_version.as_deref());
    push_snapshot_freshness(&mut parts, snapshot);
    parts.join(" · ")
}

fn fee_snapshot_health_label(label: &str, snapshot: &TradeFeeSnapshot) -> String {
    let mut parts = vec![
        label.to_owned(),
        format!("fetched_at_ms {}", snapshot.fetched_at_ms),
    ];
    push_snapshot_freshness(&mut parts, snapshot);
    parts.join(" · ")
}

fn push_snapshot_freshness(parts: &mut Vec<String>, snapshot: &TradeFeeSnapshot) {
    if let Some(freshness_ms) = snapshot.freshness_ms {
        parts.push(format!("freshness_ms {freshness_ms}"));
    }
}

fn push_optional_fee_part(parts: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(format!("{label} {value}"));
    }
}

pub(super) fn money(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("${:.0}K", value / 1_000.0)
    } else {
        format!("${value:.0}")
    }
}

pub(super) fn parse_number(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(parsed) = finite_number(trimmed) {
        return Some(parsed);
    }
    let compact: String = trimmed
        .chars()
        .filter(|ch| !matches!(ch, ',' | '$' | '%' | ' '))
        .collect();
    finite_number(&compact)
}

fn finite_number(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

pub(super) fn execution_mode_label(mode: ExecutionMode) -> &'static str {
    product_execution_mode_label(mode)
}

pub(super) fn parse_margin_mode(value: &str) -> MarginMode {
    match value {
        "Isolated" => MarginMode::Isolated,
        _ => MarginMode::Cross,
    }
}

pub(super) fn parse_time_in_force(value: &str) -> TimeInForce {
    match value {
        "FOK" => TimeInForce::Fok,
        "GTC" => TimeInForce::Gtc,
        "GTX" => TimeInForce::Gtx,
        _ => TimeInForce::Ioc,
    }
}

pub(super) fn parse_order_type(value: &str) -> OrderType {
    match value {
        "Market" => OrderType::Market,
        "Post-only" => OrderType::PostOnly,
        _ => OrderType::Limit,
    }
}
