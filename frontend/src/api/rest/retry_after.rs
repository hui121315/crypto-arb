//! HTTP `Retry-After` 头解析（RFC 7231 §7.1.3）：delta-seconds 或 IMF-fixdate → 毫秒。
//! 被 `error.rs` 的 `error_from_response` 消费。

pub(in crate::api::rest) fn parse_retry_after_ms(value: &str) -> Option<u64> {
    parse_retry_after_ms_with_clock(value, now_unix_ms())
}

#[cfg(target_arch = "wasm32")]
fn now_unix_ms() -> Option<f64> {
    let now = js_sys::Date::now();
    now.is_finite().then_some(now)
}

#[cfg(not(target_arch = "wasm32"))]
fn now_unix_ms() -> Option<f64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => Some(elapsed.as_millis() as f64),
        Err(_) => None,
    }
}

/// Parse an HTTP `Retry-After` header value (RFC 7231 §7.1.3) into milliseconds.
///
/// Accepts both forms: a non-negative delta-seconds integer, or an IMF-fixdate
/// HTTP-date, in which case the delay is the remaining time until that instant
/// relative to `now_ms` (clamped to zero when the date is already in the past).
fn parse_retry_after_ms_with_clock(value: &str, now_ms: Option<f64>) -> Option<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(seconds.saturating_mul(1_000));
    }
    let now_ms = now_ms?;
    let target_ms = http_date_to_unix_ms(trimmed)?;
    let delta_ms = target_ms - now_ms;
    if delta_ms <= 0.0 {
        Some(0)
    } else {
        Some(delta_ms.min(u64::MAX as f64) as u64)
    }
}

#[cfg(test)]
fn parse_retry_after_ms_at(value: &str, now_ms: f64) -> Option<u64> {
    parse_retry_after_ms_with_clock(value, Some(now_ms))
}

/// Parse an IMF-fixdate (e.g. `Sun, 06 Nov 1994 08:49:37 GMT`) into Unix epoch
/// milliseconds. Only the fixed-length GMT form mandated by RFC 7231 is accepted.
fn http_date_to_unix_ms(value: &str) -> Option<f64> {
    let value = value.strip_suffix(" GMT")?;
    let rest = value.split_once(", ")?.1;
    let mut fields = rest.split(' ');
    let day: i64 = fields.next()?.parse().ok()?;
    let month = month_from_abbrev(fields.next()?)?;
    let year: i64 = fields.next()?.parse().ok()?;
    let time = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if clock.next().is_some() {
        return None;
    }
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(secs as f64 * 1_000.0)
}

fn month_from_abbrev(abbr: &str) -> Option<i64> {
    Some(match abbr {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

/// Days since the Unix epoch (1970-01-01) for a proleptic Gregorian date,
/// using Howard Hinnant's `days_from_civil` algorithm.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_numeric_seconds_convert_to_ms() {
        assert_eq!(parse_retry_after_ms_at("120", 0.0), Some(120_000));
        assert_eq!(parse_retry_after_ms_at("  0 ", 0.0), Some(0));
    }

    #[test]
    fn retry_after_numeric_saturates_instead_of_overflowing() {
        assert_eq!(
            parse_retry_after_ms_at(&u64::MAX.to_string(), 0.0),
            Some(u64::MAX)
        );
    }

    #[test]
    fn retry_after_numeric_does_not_depend_on_wall_clock() {
        assert_eq!(parse_retry_after_ms_with_clock("7", None), Some(7_000));
        assert_eq!(
            parse_retry_after_ms_with_clock("Sun, 06 Nov 1994 08:49:37 GMT", None),
            None
        );
    }

    #[test]
    fn retry_after_http_date_returns_delta_until_target() {
        // 1994-11-06 08:49:37 GMT == 784_111_777 s (RFC 7231 example).
        let target_ms = 784_111_777_000.0;
        assert_eq!(
            parse_retry_after_ms_at("Sun, 06 Nov 1994 08:49:37 GMT", target_ms - 5_000.0),
            Some(5_000)
        );
    }

    #[test]
    fn retry_after_http_date_in_past_clamps_to_zero() {
        let target_ms = 784_111_777_000.0;
        assert_eq!(
            parse_retry_after_ms_at("Sun, 06 Nov 1994 08:49:37 GMT", target_ms + 10_000.0),
            Some(0)
        );
    }

    #[test]
    fn retry_after_rejects_unparseable_values() {
        assert_eq!(parse_retry_after_ms_at("", 0.0), None);
        assert_eq!(parse_retry_after_ms_at("soon", 0.0), None);
        assert_eq!(
            parse_retry_after_ms_at("Sun, 06 Foo 1994 08:49:37 GMT", 0.0),
            None
        );
    }
}
