pub(super) fn raw_units(decimals: u8, units: u16) -> String {
    format!("{units}{}", "0".repeat(usize::from(decimals)))
}

pub(super) fn decimal_units(raw: &str, decimals: u8) -> String {
    let raw = raw.trim();
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return raw.to_owned();
    }
    let decimals = usize::from(decimals);
    if decimals == 0 {
        return normalized_whole(raw);
    }

    let padded = if raw.len() <= decimals {
        format!("{}{}", "0".repeat(decimals + 1 - raw.len()), raw)
    } else {
        raw.to_owned()
    };
    let split = padded.len() - decimals;
    let whole = normalized_whole(&padded[..split]);
    let fraction = padded[split..].trim_end_matches('0');
    if fraction.is_empty() {
        whole
    } else {
        format!("{whole}.{fraction}")
    }
}

pub(super) fn decimal_to_raw_units(value: &str, decimals: u8) -> Option<String> {
    let value = value.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if value.is_empty()
        || value.matches('.').count() > 1
        || (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let fraction = fraction.trim_end_matches('0');
    let decimals = usize::from(decimals);
    if fraction.len() > decimals {
        return None;
    }
    let whole = if whole.is_empty() { "0" } else { whole };
    let raw = format!(
        "{}{}{}",
        whole,
        fraction,
        "0".repeat(decimals - fraction.len())
    );
    Some(normalized_whole(&raw))
}

fn normalized_whole(value: &str) -> String {
    let value = value.trim_start_matches('0');
    if value.is_empty() {
        "0".to_owned()
    } else {
        value.to_owned()
    }
}

pub(super) fn percent_input(bps: f64) -> String {
    (bps / 100.0).to_string()
}

pub(super) fn percent_to_bps(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| value * 100.0)
}

pub(in crate::panels::modules::onchain) fn normalized_asset(value: &str) -> String {
    value
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| char::from(byte.to_ascii_uppercase()))
        .collect()
}

pub(in crate::panels::modules::onchain) fn explicit_pair_assets(
    symbol: &str,
) -> Option<(String, String)> {
    let value = symbol.trim().to_ascii_uppercase();
    let (base, quote) = ['/', '-', ':', '_']
        .into_iter()
        .find_map(|separator| value.split_once(separator))?;
    let base = normalized_asset(base);
    let quote = normalized_asset(quote);
    (!base.is_empty() && !quote.is_empty()).then_some((base, quote))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_amounts_round_trip_without_floating_point() {
        assert_eq!(decimal_units("1250000", 6), "1.25");
        assert_eq!(decimal_units("1", 6), "0.000001");
        assert_eq!(decimal_to_raw_units("1.25", 6).as_deref(), Some("1250000"));
        assert_eq!(decimal_to_raw_units("0.000001", 6).as_deref(), Some("1"));
        assert_eq!(
            decimal_to_raw_units("1.0000000", 6).as_deref(),
            Some("1000000")
        );
        assert_eq!(decimal_to_raw_units("0.0000001", 6), None);
    }

    #[test]
    fn explicit_cex_pair_keeps_both_user_selected_assets() {
        assert_eq!(
            explicit_pair_assets(" sol/usd "),
            Some(("SOL".to_owned(), "USD".to_owned()))
        );
        assert_eq!(explicit_pair_assets("SOL"), None);
    }
}
