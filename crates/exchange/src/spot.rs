//! 现货行情解析的交易所无关小内核。

use common::time::now_ms;
use rust_decimal::Decimal;
use shared_types::SpotTick;

pub(crate) const QUOTE_ASSETS: &[&str] = &["USDT", "USDC", "USD", "BTC", "ETH", "SOL"];

#[derive(Clone, Copy)]
pub(crate) struct SpotFields<'a> {
    pub venue: &'a str,
    pub base: &'a str,
    pub quote: &'a str,
    pub bid: &'a str,
    pub ask: &'a str,
    pub last: &'a str,
    pub bid_size: Option<&'a str>,
    pub ask_size: Option<&'a str>,
    pub volume_24h: &'a str,
    /// 交易所官方载荷给出的时间戳；官方载荷未携带时传 `None`，由内核分离落地时间。
    pub exchange_ts_ms: Option<i64>,
}

pub(crate) fn suffix_pair(symbol: &str) -> Option<(String, String)> {
    let upper = symbol.to_ascii_uppercase();
    QUOTE_ASSETS.iter().find_map(|quote| {
        upper
            .strip_suffix(quote)
            .filter(|base| !base.is_empty())
            .map(|base| (base.to_owned(), (*quote).to_owned()))
    })
}

pub(crate) fn delimited_pair(symbol: &str, delimiter: char) -> Option<(String, String)> {
    let upper = symbol.to_ascii_uppercase();
    let (base, quote) = upper.split_once(delimiter)?;
    if base.is_empty() || !QUOTE_ASSETS.contains(&quote) {
        return None;
    }
    Some((base.to_owned(), quote.to_owned()))
}

pub(crate) fn native_pair_symbol(symbol: &str, delimiter: char) -> Option<String> {
    let (base, quote) = pair_or_default(symbol)?;
    Some(format!("{base}{delimiter}{quote}"))
}

pub(crate) fn compact_pair_symbol(symbol: &str) -> Option<String> {
    let (base, quote) = pair_or_default(symbol)?;
    Some(format!("{base}{quote}"))
}

fn pair_or_default(symbol: &str) -> Option<(String, String)> {
    resolve_pair(symbol).or_else(|| {
        let trimmed = symbol.trim();
        (!trimmed.is_empty()).then(|| (trimmed.to_ascii_uppercase(), "USDT".to_owned()))
    })
}

/// 严格解析交易所符号为 `(base, quote)`：仅当能识别出受支持的计价资产
/// （分隔符或后缀命中 [`QUOTE_ASSETS`]）时返回 `Some`，否则 `None`。
/// 不像 [`pair_or_default`] 那样回退到默认 `USDT`，供覆盖规划层判定 "unsupported"。
pub(crate) fn resolve_pair(symbol: &str) -> Option<(String, String)> {
    let trimmed = symbol.trim();
    if trimmed.is_empty() {
        return None;
    }
    delimited_pair(trimmed, '/')
        .or_else(|| delimited_pair(trimmed, '-'))
        .or_else(|| delimited_pair(trimmed, '_'))
        .or_else(|| suffix_pair(trimmed))
}

pub(crate) fn spot_tick(fields: SpotFields<'_>) -> Option<SpotTick> {
    Some(SpotTick {
        venue: fields.venue.to_owned(),
        symbol: pair_symbol(fields.base, fields.quote),
        bid: positive_decimal(fields.bid)?,
        ask: positive_decimal(fields.ask)?,
        last: positive_decimal(fields.last)?,
        bid_size: optional_decimal(fields.bid_size),
        ask_size: optional_decimal(fields.ask_size),
        volume_24h: decimal(fields.volume_24h)?,
        exchange_ts_ms: valid_exchange_ts(fields.exchange_ts_ms),
        received_at_ms: now_ms(),
    })
}

pub(crate) fn symbol_matches(
    native_symbol: &str,
    base: &str,
    quote: &str,
    symbols: Option<&[String]>,
) -> bool {
    let Some(symbols) = symbols else {
        return true;
    };
    let native = native_symbol.to_ascii_uppercase();
    let compact = format!("{base}{quote}");
    let slash = format!("{base}/{quote}");
    let dash = format!("{base}-{quote}");
    let underscore = format!("{base}_{quote}");
    symbols.iter().any(|target| {
        let target = target.to_ascii_uppercase();
        target == base
            || target == native
            || target == compact
            || target == slash
            || target == dash
            || target == underscore
    })
}

/// 严格解析交易所时间戳：无法解析或非正数返回 `None`，绝不伪造成本地时间。
pub(crate) fn parse_millis_opt(value: &str) -> Option<i64> {
    value.parse::<i64>().ok().filter(|ms| *ms > 0)
}

fn pair_symbol(base: &str, quote: &str) -> String {
    format!("{base}/{quote}")
}

fn decimal(value: &str) -> Option<Decimal> {
    value
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value >= Decimal::ZERO)
}

/// 必需价格字段（bid/ask/last）的解析：必须是可解析且 **严格为正** 的十进制。
/// 缺失 / 无法解析 / 为 0 / 为负都返回 `None`，让上层 fail-closed 丢弃该 tick，
/// 避免把 "无报价" 伪造成 0 价污染机会扫描与风控。
fn positive_decimal(value: &str) -> Option<Decimal> {
    decimal(value).filter(|value| *value > Decimal::ZERO)
}

/// 官方载荷缺失 / 无法解析的尺寸返回 `None`，把 "无报量" 与真实的 0 报量区分开。
fn optional_decimal(value: Option<&str>) -> Option<Decimal> {
    value.and_then(decimal)
}

/// 仅接受严格为正的交易所时间戳；缺失或非法（含 0/负）一律 `None`，不与落地时间合并。
fn valid_exchange_ts(value: Option<i64>) -> Option<i64> {
    value.filter(|ms| *ms > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn suffix_pair_prefers_known_quote() {
        assert_eq!(suffix_pair("BTCUSDT"), Some(("BTC".into(), "USDT".into())));
        assert_eq!(suffix_pair("ETHBTC"), Some(("ETH".into(), "BTC".into())));
        assert_eq!(suffix_pair("BTC"), None);
    }

    #[test]
    fn symbol_match_accepts_base_pair_and_native_forms() {
        let targets = vec![
            "BTC".to_owned(),
            "ETH/USDT".to_owned(),
            "SOL-USDC".to_owned(),
            "ARB_USDT".to_owned(),
        ];
        assert!(symbol_matches("BTCUSDT", "BTC", "USDT", Some(&targets)));
        assert!(symbol_matches("ETH-USDT", "ETH", "USDT", Some(&targets)));
        assert!(symbol_matches("SOLUSDC", "SOL", "USDC", Some(&targets)));
        assert!(symbol_matches("ARB_USDT", "ARB", "USDT", Some(&targets)));
        assert!(!symbol_matches("DOGEUSDT", "DOGE", "USDT", Some(&targets)));
    }

    fn price_fields<'a>(bid: &'a str, ask: &'a str, last: &'a str) -> SpotFields<'a> {
        SpotFields {
            venue: "test",
            base: "BTC",
            quote: "USDT",
            bid,
            ask,
            last,
            bid_size: Some("1"),
            ask_size: Some("1"),
            volume_24h: "0",
            exchange_ts_ms: Some(1),
        }
    }

    #[test]
    fn spot_tick_drops_zero_negative_or_unparseable_required_prices() {
        assert!(spot_tick(price_fields("0", "100", "100")).is_none());
        assert!(spot_tick(price_fields("100", "0", "100")).is_none());
        assert!(spot_tick(price_fields("100", "100", "0")).is_none());
        assert!(spot_tick(price_fields("-1", "100", "100")).is_none());
        assert!(spot_tick(price_fields("", "100", "100")).is_none());
        assert!(spot_tick(price_fields("abc", "100", "100")).is_none());
    }

    #[test]
    fn spot_tick_accepts_positive_prices_with_zero_volume() {
        let tick =
            spot_tick(price_fields("100", "101", "100.5")).expect("positive prices yield a tick");
        assert_eq!(tick.bid, Decimal::new(100, 0));
        assert_eq!(tick.ask, Decimal::new(101, 0));
        assert_eq!(tick.volume_24h, Decimal::ZERO);
        assert_eq!(tick.bid_size, Some(Decimal::new(1, 0)));
        assert_eq!(tick.ask_size, Some(Decimal::new(1, 0)));
        assert_eq!(tick.exchange_ts_ms, Some(1));
        assert!(tick.received_at_ms > 0);
    }

    #[test]
    fn spot_tick_keeps_missing_sizes_as_none_not_zero() {
        let mut fields = price_fields("100", "101", "100.5");
        fields.bid_size = None;
        fields.ask_size = None;
        let tick = spot_tick(fields).expect("prices still valid without sizes");
        assert_eq!(tick.bid_size, None);
        assert_eq!(tick.ask_size, None);
    }

    #[test]
    fn spot_tick_distinguishes_real_zero_size_from_missing() {
        let mut fields = price_fields("100", "101", "100.5");
        fields.bid_size = Some("0");
        fields.ask_size = Some("0");
        let tick = spot_tick(fields).expect("zero size is a real quote, not a drop");
        assert_eq!(tick.bid_size, Some(Decimal::ZERO));
        assert_eq!(tick.ask_size, Some(Decimal::ZERO));
    }

    #[test]
    fn spot_tick_drops_unparseable_exchange_ts_to_none() {
        let mut fields = price_fields("100", "101", "100.5");
        fields.exchange_ts_ms = None;
        let tick = spot_tick(fields).expect("missing exchange ts is allowed");
        assert_eq!(tick.exchange_ts_ms, None);
        assert!(tick.received_at_ms > 0);
        assert_eq!(tick.best_timestamp_ms(), tick.received_at_ms);
    }

    #[test]
    fn parse_millis_opt_rejects_zero_and_garbage() {
        assert_eq!(parse_millis_opt("1700000000000"), Some(1_700_000_000_000));
        assert_eq!(parse_millis_opt("0"), None);
        assert_eq!(parse_millis_opt("-5"), None);
        assert_eq!(parse_millis_opt("abc"), None);
    }

    #[test]
    fn native_pair_symbol_formats_common_venue_shapes() {
        assert_eq!(compact_pair_symbol("btc/usdt").as_deref(), Some("BTCUSDT"));
        assert_eq!(
            native_pair_symbol("BTCUSDT", '-').as_deref(),
            Some("BTC-USDT")
        );
        assert_eq!(native_pair_symbol("ETH", '_').as_deref(), Some("ETH_USDT"));
    }
}
