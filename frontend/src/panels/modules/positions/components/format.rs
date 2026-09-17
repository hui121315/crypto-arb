pub(super) fn signed_money(value: f64) -> String {
    if !value.is_finite() {
        return "未知".to_owned();
    }
    if value == 0.0 {
        return "$0".to_owned();
    }
    let prefix = if value >= 0.0 { "+" } else { "-" };
    format!("{prefix}{}", compact_money(value.abs()))
}

pub(super) fn signed_pct(value: f64) -> String {
    format!("{value:+.2}%")
}

pub(super) fn money(value: f64) -> String {
    compact_money(value)
}

pub(super) fn precise_money(value: f64) -> String {
    if !value.is_finite() {
        return "未知".to_owned();
    }
    let abs = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    if abs >= 1_000_000.0 {
        format!("{sign}${:.2}M", abs / 1_000_000.0)
    } else if abs >= 10_000.0 {
        format!("{sign}${:.2}K", abs / 1_000.0)
    } else if abs > 0.0 && abs < 0.01 {
        format!("{sign}<$0.01")
    } else {
        format!("{sign}${abs:.2}")
    }
}

pub(super) fn quantity(value: f64) -> String {
    if !value.is_finite() {
        return "未知".to_owned();
    }
    let abs = value.abs();
    let rendered = if abs >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if abs >= 10_000.0 {
        format!("{:.2}K", value / 1_000.0)
    } else if abs >= 1.0 {
        format!("{value:.4}")
    } else if abs >= 0.01 {
        format!("{value:.6}")
    } else {
        format!("{value:.8}")
    };
    trim_decimal(rendered)
}

pub(super) fn pct(value: f64) -> String {
    format!("{value:.1}%")
}

fn compact_money(value: f64) -> String {
    if !value.is_finite() {
        return "未知".to_owned();
    }
    let abs = value.abs();
    if abs == 0.0 {
        return "$0".to_owned();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if abs >= 1_000_000.0 {
        format!("{sign}${:.2}M", abs / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("{sign}${:.1}K", abs / 1_000.0)
    } else if abs >= 1.0 {
        format!("{sign}${abs:.0}")
    } else if abs >= 0.01 {
        format!("{sign}${abs:.2}")
    } else if abs >= 0.0001 {
        format!("{sign}${abs:.4}")
    } else if abs >= 0.000001 {
        format!("{sign}${abs:.6}")
    } else {
        format!("{sign}<$0.000001")
    }
}

fn trim_decimal(mut value: String) -> String {
    let suffix = value
        .chars()
        .last()
        .filter(|last| matches!(last, 'K' | 'M'));
    if suffix.is_some() {
        value.pop();
    }
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    if let Some(suffix) = suffix {
        value.push(suffix);
    }
    value
}
