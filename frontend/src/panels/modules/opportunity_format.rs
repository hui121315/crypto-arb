use shared_types::{RiskLevel, StrategyKind};

use crate::panels::modules::strategy_scope::p0_strategy_label_or_unavailable;

pub(crate) const MISSING_QUOTE_LABEL: &str = "待报价";
const MISSING_QUOTE_REASON: &str = "缺腿级行情证据";

pub(crate) fn missing_quote_label() -> &'static str {
    MISSING_QUOTE_LABEL
}

pub(crate) fn missing_quote_text(evidence: Option<&str>) -> String {
    let reason = evidence
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(MISSING_QUOTE_REASON);
    format!("{MISSING_QUOTE_LABEL} · {reason}")
}

pub(crate) fn is_missing_quote_label(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || value == "-" || value == MISSING_QUOTE_LABEL
}

pub(crate) fn quote_price_line(price: &str, evidence: Option<&str>) -> String {
    let price = price.trim();
    if is_missing_quote_label(price) {
        return format!("价格 {}", missing_quote_text(evidence));
    }
    format!("价格 {price}")
}

pub(crate) fn price(value: Option<f64>) -> String {
    let value = value.unwrap_or(0.0).max(0.0);
    if value <= f64::EPSILON {
        return missing_quote_label().into();
    }
    if value >= 100.0 {
        format!("{value:.2}")
    } else if value >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.8}")
    }
}

pub(crate) fn strategy_label(kind: Option<StrategyKind>) -> String {
    p0_strategy_label_or_unavailable(kind).into()
}

pub(crate) fn risk_label(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Low => "低",
        RiskLevel::Medium => "中",
        RiskLevel::High => "高",
    }
}

pub(crate) fn evidence_profit_class(cost_verified: bool, value_bps: f64) -> &'static str {
    if !cost_verified || !value_bps.is_finite() {
        "muted"
    } else if value_bps >= 0.0 {
        "positive"
    } else {
        "negative"
    }
}

#[cfg(test)]
mod tests {
    use super::{evidence_profit_class, missing_quote_text, quote_price_line};

    #[test]
    fn missing_quote_text_requires_reason() {
        assert_eq!(missing_quote_text(None), "待报价 · 缺腿级行情证据");
        assert_eq!(
            missing_quote_text(Some("证据 限频 · REST 兜底")),
            "待报价 · 证据 限频 · REST 兜底"
        );
    }

    #[test]
    fn quote_price_line_explains_missing_price() {
        assert_eq!(
            quote_price_line("-", Some("证据 缺数据 · 本地缓存")),
            "价格 待报价 · 证据 缺数据 · 本地缓存"
        );
        assert_eq!(quote_price_line("100.50", None), "价格 100.50");
    }

    #[test]
    fn profit_class_requires_verified_cost() {
        assert_eq!(evidence_profit_class(false, 0.0), "muted");
        assert_eq!(evidence_profit_class(false, 9.0), "muted");
    }

    #[test]
    fn profit_class_marks_verified_sign_only() {
        assert_eq!(evidence_profit_class(true, 0.0), "positive");
        assert_eq!(evidence_profit_class(true, 1.0), "positive");
        assert_eq!(evidence_profit_class(true, -0.1), "negative");
    }

    #[test]
    fn profit_class_mutes_non_finite_values() {
        assert_eq!(evidence_profit_class(true, f64::NAN), "muted");
        assert_eq!(evidence_profit_class(true, f64::INFINITY), "muted");
    }
}
