const MAX_SUPPORTED_TIMESTAMP_MS: i64 = 8_640_000_000_000_000;

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_ms() -> i64 {
    js_sys::Date::now().clamp(0.0, i64::MAX as f64) as i64
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(i64::MAX as u128) as i64
        })
}

pub(crate) fn utc_hms(ms: i64) -> Option<String> {
    if !(-MAX_SUPPORTED_TIMESTAMP_MS..=MAX_SUPPORTED_TIMESTAMP_MS).contains(&ms) {
        return None;
    }
    let seconds = ms.div_euclid(1_000).rem_euclid(86_400);
    Some(format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    ))
}

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn utc_date_hm(ms: i64) -> Option<String> {
    if !(-MAX_SUPPORTED_TIMESTAMP_MS..=MAX_SUPPORTED_TIMESTAMP_MS).contains(&ms) {
        return None;
    }
    let total_seconds = ms.div_euclid(1_000);
    let days = total_seconds.div_euclid(86_400);
    let seconds = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60
    ))
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let adjusted = days + 719_468;
    let era = if adjusted >= 0 {
        adjusted
    } else {
        adjusted - 146_096
    } / 146_097;
    let day_of_era = adjusted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = month_index + if month_index < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn local_hms(ms: i64) -> Option<String> {
    if !(-MAX_SUPPORTED_TIMESTAMP_MS..=MAX_SUPPORTED_TIMESTAMP_MS).contains(&ms) {
        return None;
    }
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    Some(format!(
        "{:02}:{:02}:{:02}",
        date.get_hours(),
        date.get_minutes(),
        date.get_seconds()
    ))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn local_date_hm(ms: i64) -> Option<String> {
    if !(-MAX_SUPPORTED_TIMESTAMP_MS..=MAX_SUPPORTED_TIMESTAMP_MS).contains(&ms) {
        return None;
    }
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    ))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn local_hms(ms: i64) -> Option<String> {
    utc_hms(ms)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn local_date_hm(ms: i64) -> Option<String> {
    utc_date_hm(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_hms_formats_epoch_time_without_chrono_format_runtime() {
        assert_eq!(utc_hms(0).as_deref(), Some("00:00:00"));
        assert_eq!(utc_hms(45_296_000).as_deref(), Some("12:34:56"));
        assert_eq!(utc_hms(i64::MAX), None);
    }

    #[test]
    fn native_local_hms_has_a_deterministic_utc_fallback() {
        assert_eq!(local_hms(45_296_000).as_deref(), Some("12:34:56"));
        assert_eq!(local_hms(i64::MAX), None);
    }

    #[test]
    fn date_hm_formats_epoch_and_rejects_unsupported_values() {
        assert_eq!(utc_date_hm(0).as_deref(), Some("1970-01-01 00:00"));
        assert_eq!(local_date_hm(0).as_deref(), Some("1970-01-01 00:00"));
        assert_eq!(utc_date_hm(i64::MAX), None);
    }

    #[test]
    fn native_now_ms_uses_unix_epoch_milliseconds() {
        assert!(now_ms() > 1_700_000_000_000);
    }
}
