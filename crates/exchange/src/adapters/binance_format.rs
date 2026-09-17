//! Binance local formatting and symbol helpers.

use shared_types::{OrderSide, OrderType, TimeInForce};

pub(super) fn serialize_query(params: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

pub(super) fn build_spot_symbols_param(symbols: &[String]) -> Option<String> {
    if symbols.is_empty() {
        return None;
    }
    let mut candidates = Vec::with_capacity(symbols.len());
    for raw in symbols {
        let normalized = normalize_spot_symbol(raw);
        let (base, quote) = crate::spot::resolve_pair(&normalized)?;
        candidates.push(format!("{base}{quote}"));
    }
    candidates.sort_unstable();
    candidates.dedup();
    (!candidates.is_empty())
        .then(|| serde_json::to_string(&candidates).ok())
        .flatten()
}

fn normalize_spot_symbol(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(c, '/' | '-' | '_'))
        .collect::<String>()
        .to_ascii_uppercase()
}

pub(super) fn is_usdm_perp(symbol: &str) -> bool {
    symbol.ends_with("USDT") || symbol.ends_with("USDC")
}

pub(super) fn is_usdt_perp(symbol: &str) -> bool {
    symbol.ends_with("USDT")
}

/// Preserve an explicitly requested USD(S)-M quote on public WS subscriptions.
/// Bare bases keep the product discovery default of USDT.
pub(super) fn usdm_stream_symbol(symbol: &str) -> String {
    usdm_symbol_candidates(symbol)
        .into_iter()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub(super) fn usdm_symbol_candidates(symbol: &str) -> Vec<String> {
    let compact = normalize_spot_symbol(symbol);
    for suffix in ["USDTSWAP", "USDTPERP", "USDCSWAP", "USDCPERP"] {
        if let Some(base) = compact.strip_suffix(suffix) {
            return vec![format!("{}{}", base, &suffix[..4])];
        }
    }
    if is_usdm_perp(&compact) {
        return vec![compact];
    }
    let base = crate::adapter::strip_common_suffixes(symbol).to_ascii_uppercase();
    if base.is_empty() {
        return Vec::new();
    }
    // 千倍族候选：币安把低价币上成 1000SHIB / 1000PEPE、极端者 1MBABYDOGE。
    // 顺序即优先级——调用方（`Binance::to_exchange_symbol`）拿上市符号集
    // 挑第一个真实存在的；无集合时取首位（与历史行为一致）。
    vec![
        format!("{base}USDT"),
        format!("1000{base}USDT"),
        format!("1M{base}USDT"),
        format!("{base}USDC"),
    ]
}

pub(super) fn binance_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "BUY",
        OrderSide::Sell => "SELL",
    }
}

pub(super) fn binance_order_type(order_type: OrderType) -> &'static str {
    match order_type {
        OrderType::Market => "MARKET",
        OrderType::Limit | OrderType::PostOnly => "LIMIT",
    }
}

pub(super) fn binance_time_in_force(
    order_type: OrderType,
    time_in_force: TimeInForce,
) -> Option<&'static str> {
    match order_type {
        OrderType::Market => None,
        OrderType::PostOnly => Some("GTX"),
        OrderType::Limit => Some(binance_limit_time_in_force(time_in_force)),
    }
}

fn binance_limit_time_in_force(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "GTC",
        TimeInForce::Ioc => "IOC",
        TimeInForce::Fok => "FOK",
        TimeInForce::Gtx => "GTX",
    }
}

pub(super) fn number_param(value: f64) -> String {
    let formatted = format!("{value:.12}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
#[path = "binance_format_tests.rs"]
mod tests;
