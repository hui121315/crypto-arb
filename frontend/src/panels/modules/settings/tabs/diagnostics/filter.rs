use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HealthStatusFilter {
    All,
    Attention,
    Blocked,
    Warn,
    Unknown,
    Ok,
    Unsupported,
}

impl HealthStatusFilter {
    pub(super) const fn as_key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Attention => "attention",
            Self::Blocked => "blocked",
            Self::Warn => "warn",
            Self::Unknown => "unknown",
            Self::Ok => "ok",
            Self::Unsupported => "unsupported",
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Attention => "需关注",
            Self::Blocked => "阻断",
            Self::Warn => "观察",
            Self::Unknown => "待验证",
            Self::Ok => "正常",
            Self::Unsupported => "不支持",
        }
    }

    pub(super) fn from_key(value: &str) -> Self {
        match value {
            "attention" => Self::Attention,
            "blocked" => Self::Blocked,
            "warn" => Self::Warn,
            "unknown" => Self::Unknown,
            "ok" => Self::Ok,
            "unsupported" => Self::Unsupported,
            _ => Self::All,
        }
    }

    pub(super) const fn matches(self, status: VenueOperationStatus) -> bool {
        match self {
            Self::All => true,
            Self::Attention => !matches!(status, VenueOperationStatus::Ok),
            Self::Blocked => matches!(status, VenueOperationStatus::Blocked),
            Self::Warn => matches!(status, VenueOperationStatus::Warn),
            Self::Unknown => matches!(status, VenueOperationStatus::Unknown),
            Self::Ok => matches!(status, VenueOperationStatus::Ok),
            Self::Unsupported => matches!(status, VenueOperationStatus::Unsupported),
        }
    }
}

pub(super) fn credential_sample(configured: Option<bool>, supported: Option<bool>) -> String {
    match (configured, supported) {
        (_, Some(false)) => "不支持".to_owned(),
        (Some(true), _) => "静态字段完整".to_owned(),
        (Some(false), _) => "静态缺字段".to_owned(),
        _ => "-".to_owned(),
    }
}

pub(super) fn operation_configured_label(row: &VenueOperationHealth) -> &'static str {
    match row.configured {
        Some(true) => "配置存在",
        Some(false) => "配置缺失",
        None => "配置未知",
    }
}

pub(super) fn operation_capability_label(row: &VenueOperationHealth) -> &'static str {
    if !row.capability_supported() {
        "不支持"
    } else if row.supported == Some(true) {
        "支持"
    } else {
        "未声明（按支持）"
    }
}

pub(super) fn operation_usable_label(row: &VenueOperationHealth) -> &'static str {
    if row.is_currently_usable() {
        "可用"
    } else {
        "不可用"
    }
}

pub(super) fn operation_sample(row: &VenueOperationHealth) -> String {
    let base = match (row.requested, row.rows) {
        (Some(requested), Some(rows)) => format!("{rows}/{requested}"),
        _ => credential_sample(row.configured, row.supported),
    };
    let latency = operation_latency_sample(row);
    if latency.is_empty() {
        base
    } else if base == "-" {
        latency
    } else {
        format!("{base} · {latency}")
    }
}

pub(super) fn operation_latency_sample(row: &VenueOperationHealth) -> String {
    let label = operation_latency_label(row);
    let mut parts = Vec::with_capacity(2);
    if let Some(latency_ms) = row.latency_ms {
        parts.push(format!("{label} {latency_ms}ms"));
    }
    if let Some(latency_p95_ms) = row.latency_p95_ms {
        parts.push(format!("p95≤{latency_p95_ms}ms"));
    }
    parts.join(" · ")
}

pub(super) fn operation_latency_label(row: &VenueOperationHealth) -> &'static str {
    if VenueOperationKind::parse(&row.operation) == VenueOperationKind::HttpRest {
        "HTTP RTT"
    } else {
        "延迟"
    }
}

pub(super) fn ratio_label(value: f64) -> String {
    if value.is_finite() {
        format!("{:.1}%", value * 100.0)
    } else {
        "-".to_owned()
    }
}
