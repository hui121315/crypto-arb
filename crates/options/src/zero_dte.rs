//! 0DTE（当日到期）期权筛选与优先级排序。
//!
//! 业务定义：到期时间 ≤ 24 小时的期权属于 "0DTE"，特征是 theta 衰减极快。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 0DTE 阈值：到期时间小于该时长（小时）即为 0DTE。
pub const ZERO_DTE_HOURS: f64 = 24.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeroDteCandidate {
    pub instrument_name: String,
    pub strike: f64,
    pub option_type: String,
    pub mark_price: f64,
    pub iv: Option<f64>,
    pub delta: Option<f64>,
    pub volume_24h: f64,
    /// 距到期的小时数（≥0）
    pub time_to_expiry_hours: f64,
    pub expiry_timestamp_ms: i64,
}

impl ZeroDteCandidate {
    /// 判定是否属于 0DTE。
    pub fn is_zero_dte(&self) -> bool {
        self.time_to_expiry_hours >= 0.0 && self.time_to_expiry_hours <= ZERO_DTE_HOURS
    }
}

/// 给定到期时间戳（毫秒），计算距当前的小时数（不会为负）。
pub fn hours_to_expiry(expiry_timestamp_ms: i64) -> f64 {
    let now = Utc::now().timestamp_millis();
    let diff = (expiry_timestamp_ms - now).max(0);
    diff as f64 / 3_600_000.0
}

/// 给定到期 [`DateTime<Utc>`]，计算距当前的小时数。
pub fn hours_to_expiry_dt(expiry: DateTime<Utc>) -> f64 {
    hours_to_expiry(expiry.timestamp_millis())
}

/// 从一组候选中筛选出 0DTE，并按"距到期时间升序"排序（越快到期越靠前）。
pub fn filter_zero_dte(candidates: Vec<ZeroDteCandidate>) -> Vec<ZeroDteCandidate> {
    let mut filtered: Vec<ZeroDteCandidate> =
        candidates.into_iter().filter(|c| c.is_zero_dte()).collect();
    filtered.sort_by(|a, b| {
        a.time_to_expiry_hours
            .partial_cmp(&b.time_to_expiry_hours)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    filtered
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use pretty_assertions::assert_eq;

    fn cand(name: &str, hours: f64) -> ZeroDteCandidate {
        ZeroDteCandidate {
            instrument_name: name.into(),
            strike: 30000.0,
            option_type: "call".into(),
            mark_price: 100.0,
            iv: None,
            delta: None,
            volume_24h: 1000.0,
            time_to_expiry_hours: hours,
            expiry_timestamp_ms: 0,
        }
    }

    #[test]
    fn classifies_within_24h_as_zero_dte() {
        assert!(cand("a", 0.5).is_zero_dte());
        assert!(cand("b", 12.0).is_zero_dte());
        assert!(cand("c", 24.0).is_zero_dte());
    }

    #[test]
    fn classifies_beyond_24h_as_not_zero_dte() {
        assert!(!cand("d", 25.0).is_zero_dte());
        assert!(!cand("e", 100.0).is_zero_dte());
    }

    #[test]
    fn negative_hours_treated_as_expired() {
        // 已过期不属于 0DTE
        assert!(!cand("expired", -1.0).is_zero_dte());
    }

    #[test]
    fn filter_sorts_by_time_ascending() {
        let v = vec![
            cand("c", 20.0),
            cand("a", 5.0),
            cand("b", 12.0),
            cand("far", 48.0),
        ];
        let r = filter_zero_dte(v);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].instrument_name, "a");
        assert_eq!(r[1].instrument_name, "b");
        assert_eq!(r[2].instrument_name, "c");
    }

    #[test]
    fn hours_to_expiry_in_future() {
        let future = Utc::now() + Duration::hours(8);
        let h = hours_to_expiry_dt(future);
        assert!(h > 7.5 && h <= 8.0);
    }

    #[test]
    fn hours_to_expiry_in_past_clamped_zero() {
        let past = Utc::now() - Duration::hours(1);
        let h = hours_to_expiry_dt(past);
        assert!((h - 0.0).abs() < 1e-9);
    }
}
