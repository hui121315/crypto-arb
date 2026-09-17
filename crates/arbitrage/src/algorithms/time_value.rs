//! 结算时间价值。
//!
//! - 距离下次结算的时间影响是否值得开仓（接近结算时收益最确定）
//! - "狙击窗口"：15 分钟内将结算，进入则可锁定单次费率

use chrono::Utc;

const SNIPE_WINDOW_MS: i64 = 15 * 60 * 1000;

/// 给定下次结算时间戳（毫秒），返回距离结算的毫秒数（不会为负）。
pub fn time_to_settlement_ms(next_funding_time_ms: i64) -> i64 {
    time_to_settlement_ms_at(next_funding_time_ms, Utc::now().timestamp_millis())
}

pub fn time_to_settlement_ms_at(next_funding_time_ms: i64, now_ms: i64) -> i64 {
    if next_funding_time_ms <= 0 {
        return 0;
    }
    (next_funding_time_ms - now_ms).max(0)
}

/// 是否处于狙击窗口（15 分钟内将结算）。
pub fn is_snipe_ready(next_funding_time_ms: i64) -> bool {
    is_snipe_ready_at(next_funding_time_ms, Utc::now().timestamp_millis())
}

pub fn is_snipe_ready_at(next_funding_time_ms: i64, now_ms: i64) -> bool {
    if next_funding_time_ms <= 0 {
        return false;
    }
    let dt = time_to_settlement_ms_at(next_funding_time_ms, now_ms);
    dt > 0 && dt < SNIPE_WINDOW_MS
}

/// 时间价值衰减因子（线性近似）。
///
/// - 距离结算 < 15 分钟：1.0（满价值）
/// - 距离结算 = 8h：0.0（最低，刚刚结算完）
pub fn time_decay_factor(next_funding_time_ms: i64, funding_interval_hours: u32) -> f64 {
    time_decay_factor_at(
        next_funding_time_ms,
        funding_interval_hours,
        Utc::now().timestamp_millis(),
    )
}

pub fn time_decay_factor_at(
    next_funding_time_ms: i64,
    funding_interval_hours: u32,
    now_ms: i64,
) -> f64 {
    if next_funding_time_ms <= 0 {
        return 0.0;
    }
    let dt_ms = time_to_settlement_ms_at(next_funding_time_ms, now_ms);
    let interval_ms = (funding_interval_hours.max(1) as i64) * 3_600_000;
    if dt_ms >= interval_ms {
        return 0.0;
    }
    if dt_ms <= SNIPE_WINDOW_MS {
        return 1.0;
    }
    // 线性衰减：从 1.0 → 0.0 over [snipe, interval]
    let span = (interval_ms - SNIPE_WINDOW_MS) as f64;
    let elapsed = (interval_ms - dt_ms) as f64;
    (elapsed / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn time_to_settlement_in_future() {
        let now = 1_700_000_000_000;
        let future = now + 60_000;
        let dt = time_to_settlement_ms_at(future, now);
        assert_eq!(dt, 60_000);
    }

    #[test]
    fn time_to_settlement_in_past_clamped_zero() {
        let now = 1_700_000_000_000;
        assert_eq!(time_to_settlement_ms_at(now - 60_000, now), 0);
    }

    #[test]
    fn snipe_window_logic() {
        let now = 1_700_000_000_000;
        let in_window = now + 5 * 60_000;
        let out_window = now + 60 * 60_000;
        assert!(is_snipe_ready_at(in_window, now));
        assert!(!is_snipe_ready_at(out_window, now));
        assert!(!is_snipe_ready_at(0, now));
    }

    #[test]
    fn time_decay_within_snipe_is_one() {
        let now = 1_700_000_000_000;
        let in_window = now + 10 * 60_000;
        let f = time_decay_factor_at(in_window, 8, now);
        assert!((f - 1.0).abs() < 1e-9);
    }

    #[test]
    fn time_decay_at_full_interval_is_zero() {
        let now = 1_700_000_000_000;
        let far = now + 9 * 3_600_000;
        let f = time_decay_factor_at(far, 8, now);
        assert!((f - 0.0).abs() < 1e-9);
    }

    #[test]
    fn time_decay_midway_decreases() {
        let now = 1_700_000_000_000;
        let mid = now + 4 * 3_600_000;
        let f = time_decay_factor_at(mid, 8, now);
        assert!(f > 0.0 && f < 1.0);
    }

    #[test]
    fn missing_next_funding_time_has_no_time_value() {
        assert_eq!(time_decay_factor_at(0, 8, 1_700_000_000_000), 0.0);
        assert!(!is_snipe_ready_at(0, 1_700_000_000_000));
    }
}
