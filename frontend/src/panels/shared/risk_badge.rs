use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RiskBadgeTone {
    Low,
    Medium,
    High,
    Unknown,
    Error,
}

impl RiskBadgeTone {
    fn from_label(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "低" | "low" => Self::Low,
            "中" | "medium" | "med" => Self::Medium,
            "高" | "high" => Self::High,
            "错误" | "失败" | "error" | "failed" => Self::Error,
            _ => Self::Unknown,
        }
    }

    fn class_name(self) -> &'static str {
        match self {
            Self::Low => "risk-badge low",
            Self::Medium => "risk-badge med",
            Self::High => "risk-badge high",
            Self::Unknown => "risk-badge unknown",
            Self::Error => "risk-badge error",
        }
    }

    fn label(self, raw: &str) -> String {
        match self {
            Self::Low => "低".into(),
            Self::Medium => "中".into(),
            Self::High => "高".into(),
            Self::Error => "错误".into(),
            Self::Unknown => unknown_label(raw),
        }
    }
}

#[component]
pub fn RiskBadge(#[prop(into)] risk: String) -> impl IntoView {
    let risk = risk.into_boxed_str();
    let tone = RiskBadgeTone::from_label(&risk);
    let label = tone.label(&risk);
    let title = risk_title(tone, &risk);

    view! { <span class=tone.class_name() title=title>{label}</span> }
}

fn unknown_label(raw: &str) -> String {
    match raw.trim() {
        "" | "-" | "--" | "未知" | "unknown" => "未知".into(),
        value => format!("未知:{value}"),
    }
}

fn risk_title(tone: RiskBadgeTone, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "风险等级：未知".into();
    }
    match tone {
        RiskBadgeTone::Low | RiskBadgeTone::Medium | RiskBadgeTone::High => {
            format!("风险等级：{}", tone.label(raw))
        }
        RiskBadgeTone::Unknown | RiskBadgeTone::Error => {
            format!("风险等级：{}；原始值：{raw}", tone.label(raw))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{risk_title, RiskBadgeTone};

    #[test]
    fn risk_badge_maps_only_explicit_low_to_low() {
        assert_eq!(RiskBadgeTone::from_label("低"), RiskBadgeTone::Low);
        assert_eq!(RiskBadgeTone::from_label("low"), RiskBadgeTone::Low);
        assert_eq!(RiskBadgeTone::Low.class_name(), "risk-badge low");
        assert_eq!(RiskBadgeTone::Low.label("low"), "低");
    }

    #[test]
    fn risk_badge_maps_unknown_and_error_fail_closed() {
        assert_eq!(RiskBadgeTone::from_label(""), RiskBadgeTone::Unknown);
        assert_eq!(RiskBadgeTone::from_label("-"), RiskBadgeTone::Unknown);
        assert_eq!(
            RiskBadgeTone::from_label("unexpected"),
            RiskBadgeTone::Unknown
        );
        assert_eq!(RiskBadgeTone::from_label("错误"), RiskBadgeTone::Error);
        assert_eq!(RiskBadgeTone::Unknown.class_name(), "risk-badge unknown");
        assert_eq!(RiskBadgeTone::Error.class_name(), "risk-badge error");
        assert_eq!(RiskBadgeTone::Unknown.label("-"), "未知");
        assert_eq!(RiskBadgeTone::Unknown.label("stale"), "未知:stale");
        assert!(risk_title(RiskBadgeTone::Unknown, "stale").contains("原始值：stale"));
    }
}
