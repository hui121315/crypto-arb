const BPS_PER_PERCENT: f64 = 100.0;

pub(crate) fn signed_bps_percent(value_bps: f64) -> String {
    format!("{:+.3}%", clean_zero(value_bps / BPS_PER_PERCENT))
}

pub(crate) fn unsigned_bps_percent(value_bps: f64) -> String {
    format!("{:.3}%", clean_zero(value_bps / BPS_PER_PERCENT).abs())
}

pub(crate) fn depth_bps_percent_label(value_bps: f64) -> String {
    format!("{}% 深度", decimal_text(value_bps / BPS_PER_PERCENT, 2))
}

pub(crate) fn bps_input_as_percent(value: &str) -> String {
    parse_decimal(value).map_or_else(String::new, |bps| decimal_text(bps / BPS_PER_PERCENT, 3))
}

pub(crate) fn percent_input_as_bps(value: &str) -> String {
    parse_decimal(value).map_or_else(String::new, |pct| decimal_text(pct * BPS_PER_PERCENT, 3))
}

fn clean_zero(value: f64) -> f64 {
    if value.abs() < 0.000_5 {
        0.0
    } else {
        value
    }
}

fn parse_decimal(value: &str) -> Option<f64> {
    let compact: String = value
        .trim()
        .chars()
        .filter(|ch| !matches!(ch, ',' | '%' | ' '))
        .collect();
    if compact.is_empty() {
        return None;
    }
    compact
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn decimal_text(value: f64, precision: usize) -> String {
    let mut text = format!("{value:.precision$}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        "0".into()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_bps_as_percent() {
        assert_eq!(signed_bps_percent(109.62), "+1.096%");
        assert_eq!(unsigned_bps_percent(22.5), "0.225%");
        assert_eq!(depth_bps_percent_label(5.0), "0.05% 深度");
    }

    #[test]
    fn converts_limit_offset_input_without_changing_internal_unit() {
        assert_eq!(bps_input_as_percent("5"), "0.05");
        assert_eq!(percent_input_as_bps("0.05%"), "5");
        assert_eq!(percent_input_as_bps(""), "");
    }
}
