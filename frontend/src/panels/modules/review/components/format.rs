use shared_types::{ExecutionFillConfidence, MissReason, OrderUpdateSource, StrategyKind};

pub(super) fn environment_label(environment: Option<shared_types::ExecutionEnvironment>) -> &'static str {
    match environment {
        Some(shared_types::ExecutionEnvironment::Live) => "实盘",
        Some(shared_types::ExecutionEnvironment::Paper) => "模拟",
        None => "环境待核对",
    }
}

pub(super) fn environment_token(environment: Option<shared_types::ExecutionEnvironment>) -> &'static str {
    match environment {
        Some(shared_types::ExecutionEnvironment::Live) => "live",
        Some(shared_types::ExecutionEnvironment::Paper) => "paper",
        None => "unknown",
    }
}

pub(super) fn strategy_label(kind: StrategyKind) -> &'static str {
    kind.label_zh()
}

pub(super) fn reason_label(reason: MissReason) -> &'static str {
    match reason {
        MissReason::RiskBlocked => "风控阻断",
        MissReason::DepthInsufficient => "深度不足",
        MissReason::LatencyExceeded => "延迟超限",
        MissReason::PriceMoved => "价格移动",
        MissReason::ManualSkip => "手动跳过",
        MissReason::SignalDecayed => "信号衰减",
    }
}

pub(super) fn fill_confidence_label(confidence: ExecutionFillConfidence) -> &'static str {
    match confidence {
        ExecutionFillConfidence::VenueFill => "逐笔成交",
        ExecutionFillConfidence::VenueOrderSnapshot => "订单快照",
        ExecutionFillConfidence::OrderQuery => "订单回查",
        ExecutionFillConfidence::AdapterAck => "仅 受理确认 推定",
        ExecutionFillConfidence::Manual => "人工数据依据",
        ExecutionFillConfidence::Unknown => "未知置信",
    }
}

pub(super) fn order_update_source_label(source: OrderUpdateSource) -> &'static str {
    match source {
        OrderUpdateSource::Unknown => "未知来源",
        OrderUpdateSource::Internal => "内部状态",
        OrderUpdateSource::AdapterAck => "ACK",
        OrderUpdateSource::OrderQuery => "订单回查",
        OrderUpdateSource::PrivateWs => "私有 WS",
        OrderUpdateSource::FundingPoller => "资金费轮询",
        OrderUpdateSource::Reconcile => "对账",
        OrderUpdateSource::Manual => "人工",
    }
}

pub(super) fn signed_money(value: f64) -> String {
    if !value.is_finite() {
        return "未知".into();
    }
    let magnitude = money_magnitude(value.abs());
    if magnitude == "0.00" {
        "$0.00".into()
    } else if value.is_sign_negative() {
        format!("-${magnitude}")
    } else {
        format!("+${magnitude}")
    }
}

pub(super) fn proven_signed_money(has_samples: bool, value: f64) -> String {
    if has_samples {
        signed_money(value)
    } else {
        "—".into()
    }
}

pub(super) fn proven_signed_class(has_samples: bool, value: f64) -> &'static str {
    if has_samples {
        signed_class(value)
    } else {
        "muted"
    }
}

pub(super) fn signed_class(value: f64) -> &'static str {
    if value.is_sign_negative() {
        "negative"
    } else {
        "positive"
    }
}

pub(super) fn money(value: f64) -> String {
    if !value.is_finite() {
        return "未知".into();
    }
    let magnitude = money_magnitude(value.abs());
    if value.is_sign_negative() && magnitude != "0.00" {
        format!("-${magnitude}")
    } else {
        format!("${magnitude}")
    }
}

fn money_magnitude(value: f64) -> String {
    let precision = if value == 0.0 || value >= 1.0 {
        2
    } else if value >= 0.01 {
        4
    } else {
        6
    };
    let mut formatted = format!("{value:.precision$}");
    while formatted.ends_with('0')
        && formatted
            .split_once('.')
            .is_some_and(|(_, decimals)| decimals.len() > 2)
    {
        formatted.pop();
    }
    formatted
}

pub(super) fn pct(value: f64) -> String {
    format!("{value:.1}%")
}

pub(super) fn record_time(ms: i64) -> String {
    if ms <= 0 {
        return "时间未知".into();
    }
    crate::panels::modules::timestamp::local_date_hm(ms).unwrap_or_else(|| "时间未知".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_record_time_does_not_claim_epoch_date() {
        assert_eq!(record_time(0), "时间未知");
        assert_eq!(record_time(-1), "时间未知");
    }

    #[test]
    fn money_keeps_small_realized_values_visible() {
        assert_eq!(signed_money(-0.00422), "-$0.00422");
        assert_eq!(signed_money(-1.504), "-$1.50");
        assert_eq!(signed_money(0.0), "$0.00");
        assert_eq!(money(0.749982), "$0.75");
        assert_eq!(money(f64::NAN), "未知");
    }
}
